//! Async-signal-safe fill of [`StackSnapshot`] for SIGPROF and SIGUSR2.
//!
//! Handlers only copy raw PCs and eval-hook keys. Symbolize / merge / fold live
//! in [`crate::features::stacktrace::parse`] and [`crate::features::stacktrace::fold`].

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::{compiler_fence, AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::Duration;

use core::ffi::{c_int, c_void};
use nix::libc;
use once_cell::sync::Lazy;
use probing_proto::prelude::CallFrame;

use crate::features::stacktrace::merge::demangle_native_symbol;
use crate::features::stacktrace::spy::call::RawCallLocation;
use crate::features::stacktrace::spy::spy_tls_addrs;

pub use crate::features::stacktrace::snapshot::{
    RawStackSnapshot, StackFlags, StackSnapshot, StackSource, MAX_NATIVE, MAX_PY,
};

const REG_SIZE: usize = 1024;

// ---------------------------------------------------------------------------
// Python-thread registry (TLS pointers resolved in normal context)
// ---------------------------------------------------------------------------

struct ThreadSlot {
    tid: AtomicU64,
    pystacks: AtomicPtr<Vec<RawCallLocation>>,
    writing: AtomicPtr<bool>,
    stack_lo: AtomicUsize,
    stack_hi: AtomicUsize,
    latest: UnsafeCell<StackSnapshot>,
    latest_seq: AtomicU64,
}

// `latest` is published via `latest_seq` (Release/Acquire); only read after seq != 0.
unsafe impl Sync for ThreadSlot {}

static REG_TABLE: [ThreadSlot; REG_SIZE] = [const {
    ThreadSlot {
        tid: AtomicU64::new(0),
        pystacks: AtomicPtr::new(std::ptr::null_mut()),
        writing: AtomicPtr::new(std::ptr::null_mut()),
        stack_lo: AtomicUsize::new(0),
        stack_hi: AtomicUsize::new(0),
        latest: UnsafeCell::new(StackSnapshot::zeroed()),
        latest_seq: AtomicU64::new(0),
    }
}; REG_SIZE];

static REG_FULL_WARNED: AtomicBool = AtomicBool::new(false);
static MAIN_OS_TID: AtomicU64 = AtomicU64::new(0);
static PPROF_SAMPLING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Record the Python main thread's OS tid (pthread id on macOS, gettid on Linux).
pub fn register_main_os_tid() {
    ensure_signal_altstack();
    let tid = current_tid();
    if tid == 0 {
        return;
    }
    let _ = MAIN_OS_TID.compare_exchange(0, tid, Ordering::AcqRel, Ordering::Acquire);
}

pub fn python_main_os_tid() -> Option<u64> {
    let tid = MAIN_OS_TID.load(Ordering::Acquire);
    if tid == 0 {
        None
    } else {
        Some(tid)
    }
}

pub fn set_pprof_sampling_active(active: bool) {
    PPROF_SAMPLING_ACTIVE.store(active, Ordering::Release);
}

pub fn is_pprof_sampling_active() -> bool {
    PPROF_SAMPLING_ACTIVE.load(Ordering::Acquire)
}

thread_local! {
    static THREAD_REGISTERED: std::cell::UnsafeCell<bool> = const { std::cell::UnsafeCell::new(false) };
    /// Per-thread signal alt stack installed (`sigaltstack` is per-thread on Darwin/Linux).
    #[cfg(unix)]
    static THREAD_ALTSTACK_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static THREAD_NAMES: Lazy<RwLock<HashMap<u64, String>>> = Lazy::new(|| RwLock::new(HashMap::new()));

/// Interned Python frame metadata keyed by callee `PyCodeObject` pointer.
#[derive(Clone, Debug)]
struct PyFrameSymbol {
    func: String,
    file: String,
    lineno: i32,
}

impl PyFrameSymbol {
    /// Folded flamegraph segment (basename only for cross-rank merge).
    fn folded_label(&self) -> String {
        let base = self.file.rsplit(['/', '\\']).next().unwrap_or(&self.file);
        format!("[py] {} ({}:{})", self.func, base, self.lineno)
    }

    fn to_call_frame(&self) -> CallFrame {
        CallFrame::PyFrame {
            file: self.file.clone(),
            func: self.func.clone(),
            lineno: self.lineno as i64,
            locals: Default::default(),
        }
    }
}

static PY_SYMBOLS: Lazy<RwLock<HashMap<usize, PyFrameSymbol>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
const PY_SYMBOLS_CAP: usize = 1 << 18;

#[inline]
fn slot_hash(tid: u64) -> usize {
    let h = tid.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> 40) as usize & (REG_SIZE - 1)
}

pub fn current_tid() -> u64 {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::syscall(libc::SYS_gettid) as u64
    }
    #[cfg(target_os = "macos")]
    {
        let mut t: u64 = 0;
        unsafe { libc::pthread_threadid_np(0, &mut t) };
        t
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

fn current_thread_name() -> Option<String> {
    let mut buf = [0 as libc::c_char; 64];
    let rc = unsafe { libc::pthread_getname_np(libc::pthread_self(), buf.as_mut_ptr(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn current_stack_bounds() -> (usize, usize) {
    #[cfg(target_os = "macos")]
    unsafe {
        let pt = libc::pthread_self();
        let base = libc::pthread_get_stackaddr_np(pt) as usize;
        let size = libc::pthread_get_stacksize_np(pt);
        (base.saturating_sub(size), base)
    }
    #[cfg(target_os = "linux")]
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return (0, 0);
        }
        let mut addr: *mut c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let ok = libc::pthread_attr_getstack(&attr, &mut addr, &mut size) == 0;
        libc::pthread_attr_destroy(&mut attr);
        if ok {
            let lo = addr as usize;
            (lo, lo + size)
        } else {
            (0, 0)
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        (0, 0)
    }
}

pub fn thread_name(tid: u64) -> Option<String> {
    THREAD_NAMES.read().ok().and_then(|m| m.get(&tid).cloned())
}

pub fn register_python_thread() {
    let already = THREAD_REGISTERED.with(|flag| unsafe {
        if *flag.get() {
            return true;
        }
        *flag.get() = true;
        false
    });
    // Always ensure alt stack on this thread (idempotent) — even on re-entry
    // after fork / late attach, so SIGPROF/SIGUSR2 never run on the training stack.
    ensure_signal_altstack();
    if already {
        return;
    }
    let tid = current_tid();
    if probing_core::is_python_main_thread() {
        register_main_os_tid();
    }
    let (ps, wr) = spy_tls_addrs();
    let (lo, hi) = current_stack_bounds();

    if let Some(name) = current_thread_name() {
        if let Ok(mut m) = THREAD_NAMES.write() {
            m.insert(tid, name);
        }
    }

    let publish = |slot: &ThreadSlot| {
        slot.stack_lo.store(lo, Ordering::Release);
        slot.stack_hi.store(hi, Ordering::Release);
        slot.pystacks.store(ps, Ordering::Release);
        slot.writing.store(wr, Ordering::Release);
    };

    let start = slot_hash(tid);
    for i in 0..REG_SIZE {
        let slot = &REG_TABLE[(start + i) & (REG_SIZE - 1)];
        let v = slot.tid.load(Ordering::Acquire);
        if v == tid {
            publish(slot);
            return;
        }
        if v == 0
            && slot
                .tid
                .compare_exchange(0, tid, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            publish(slot);
            return;
        }
    }

    if !REG_FULL_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "probing: stack thread registry full ({REG_SIZE} threads); \
             Python stacks for further threads will be missing"
        );
    }
}

fn thread_slot(tid: u64) -> Option<&'static ThreadSlot> {
    let start = slot_hash(tid);
    for i in 0..REG_SIZE {
        let slot = &REG_TABLE[(start + i) & (REG_SIZE - 1)];
        let v = slot.tid.load(Ordering::Acquire);
        if v == tid {
            return Some(slot);
        }
        if v == 0 {
            return None;
        }
    }
    None
}

/// Copy the registered thread's Python stack (PYSTACKS) without delivering a signal.
pub fn copy_registered_py_snapshot(tid: u64) -> Option<StackSnapshot> {
    let slot = thread_slot(tid)?;
    let mut sample = StackSnapshot::zeroed();
    sample.tid = tid;
    sample.source = StackSource::Vm;

    let wr = slot.writing.load(Ordering::Acquire);
    let ps = slot.pystacks.load(Ordering::Acquire);
    if wr.is_null() || ps.is_null() {
        sample.flags.insert(StackFlags::PY_ABSENT);
        return None;
    }
    if unsafe { *wr } {
        sample.flags.insert(StackFlags::PY_TORN);
        return None;
    }
    compiler_fence(Ordering::SeqCst);
    let stacks = unsafe { &*ps };
    let n = stacks.len().min(MAX_PY);
    for (i, stack) in stacks.iter().enumerate().take(n) {
        sample.py[i] = stack.callee();
    }
    compiler_fence(Ordering::SeqCst);
    if unsafe { *wr } {
        sample.flags.insert(StackFlags::PY_TORN);
        return None;
    }
    sample.py_len = n as u32;
    if stacks.len() > MAX_PY {
        sample.flags.insert(StackFlags::PY_TRUNCATED);
    }
    if sample.is_empty() {
        sample.flags.insert(StackFlags::PY_ABSENT);
        None
    } else {
        Some(sample)
    }
}

/// Whether a latest-slot read is consistent and belongs to `tid`.
fn latest_snapshot_read_ok(
    seq_before: u64,
    seq_after: u64,
    snap: &StackSnapshot,
    tid: u64,
) -> bool {
    seq_before != 0 && seq_before == seq_after && snap.tid == tid && !snap.is_empty()
}

/// Whether a SIGUSR2 snapshot may be published / accepted for `target_tid`.
fn sigusr2_snapshot_matches_target(snap: &StackSnapshot, target_tid: u64) -> bool {
    target_tid != 0 && !snap.is_empty() && snap.tid == target_tid
}

/// Store the latest SIGPROF snapshot for a thread so on-demand capture can reuse it.
pub fn store_latest_snapshot(snapshot: &StackSnapshot) {
    if snapshot.is_empty() {
        return;
    }
    let Some(slot) = thread_slot(snapshot.tid) else {
        return;
    };
    unsafe {
        // Avoid a Rust temporary of the full POD (can blow a near-full training stack).
        core::ptr::copy_nonoverlapping(snapshot, slot.latest.get(), 1);
    }
    slot.latest_seq.fetch_add(1, Ordering::Release);
}

/// Fill the current thread's latest slot from `uctx` (no large stack locals).
///
/// # Safety
/// Same as [`fill_raw_snapshot`].
pub unsafe fn fill_latest_from_uctx(uctx: *mut c_void, source: StackSource) -> bool {
    fill_latest_from_uctx_with(uctx, source, FillOpts::default())
}

/// # Safety
/// Same as [`fill_raw_snapshot`].
pub unsafe fn fill_latest_from_uctx_with(
    uctx: *mut c_void,
    source: StackSource,
    opts: FillOpts,
) -> bool {
    let tid = current_tid();
    let Some(slot) = thread_slot(tid) else {
        return false;
    };
    let latest = &mut *slot.latest.get();
    fill_raw_snapshot_with(latest, uctx, opts);
    latest.source = source;
    if latest.is_empty() {
        return false;
    }
    slot.latest_seq.fetch_add(1, Ordering::Release);
    true
}

/// Reuse the latest SIGPROF snapshot for `tid` when CPU sampling is active.
pub fn latest_snapshot_for_tid(tid: u64) -> Option<StackSnapshot> {
    latest_snapshot_with_seq(tid).map(|(snap, _)| snap)
}

/// Like [`latest_snapshot_for_tid`], also returning the slot generation for view caches.
pub fn latest_snapshot_with_seq(tid: u64) -> Option<(StackSnapshot, u64)> {
    let slot = thread_slot(tid)?;
    for _ in 0..4 {
        let seq_before = slot.latest_seq.load(Ordering::Acquire);
        if seq_before == 0 {
            return None;
        }
        let snap = unsafe { *slot.latest.get() };
        let seq_after = slot.latest_seq.load(Ordering::Acquire);
        if latest_snapshot_read_ok(seq_before, seq_after, &snap, tid) {
            return Some((snap, seq_after));
        }
    }
    None
}

pub fn intern_py_frame(loc: &RawCallLocation) {
    let key = loc.callee();
    if key == 0 {
        return;
    }
    if let Ok(g) = PY_SYMBOLS.read() {
        if g.contains_key(&key) {
            return;
        }
    }
    let entry = match loc.resolve_callee() {
        Ok(sym) => PyFrameSymbol {
            func: sym.name,
            file: sym.file,
            lineno: sym.line,
        },
        Err(_) => return,
    };
    if let Ok(mut g) = PY_SYMBOLS.write() {
        if g.len() < PY_SYMBOLS_CAP {
            g.entry(key).or_insert(entry);
        }
    }
}

pub fn clear_py_symbols() {
    if let Ok(mut g) = PY_SYMBOLS.write() {
        g.clear();
        g.shrink_to_fit();
    }
}

pub(crate) fn resolve_py_label(key: usize) -> String {
    if key != 0 {
        if let Ok(g) = PY_SYMBOLS.read() {
            if let Some(sym) = g.get(&key) {
                return sym.folded_label();
            }
        }
    }
    "[py] <unknown>".to_string()
}

pub(crate) fn resolve_py_call_frame(key: usize) -> CallFrame {
    if key != 0 {
        if let Ok(g) = PY_SYMBOLS.read() {
            if let Some(sym) = g.get(&key) {
                return sym.to_call_frame();
            }
        }
    }
    CallFrame::PyFrame {
        file: String::new(),
        func: resolve_py_label(key),
        lineno: 0,
        locals: Default::default(),
    }
}

/// Canonicalize user-space pointers (strip top-byte / PAC bits on aarch64).
#[inline]
fn strip_ptr_tag(p: usize) -> usize {
    #[cfg(target_arch = "aarch64")]
    {
        // Keep low 48 bits — safe for both TBI and pointer-auth tags.
        p & ((1usize << 48) - 1)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        p
    }
}

#[inline]
fn plausible(p: usize) -> bool {
    let p = strip_ptr_tag(p);
    (0x1000..0x0001_0000_0000_0000).contains(&p)
}

#[allow(unused_variables)]
unsafe fn regs_from_uctx(uctx: *mut c_void) -> (usize, usize) {
    if uctx.is_null() {
        return (0, 0);
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let uc = uctx as *const libc::ucontext_t;
        let mc = &(*uc).uc_mcontext;
        let pc = mc.gregs[libc::REG_RIP as usize] as usize;
        let fp = mc.gregs[libc::REG_RBP as usize] as usize;
        (strip_ptr_tag(pc), strip_ptr_tag(fp))
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        let uc = uctx as *const libc::ucontext_t;
        let mc = &(*uc).uc_mcontext;
        (
            strip_ptr_tag(mc.pc as usize),
            strip_ptr_tag(mc.regs[29] as usize),
        )
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        let uc = uctx as *const libc::ucontext_t;
        let mc = (*uc).uc_mcontext;
        if mc.is_null() {
            return (0, 0);
        }
        let ss = &(*mc).__ss;
        (
            strip_ptr_tag(ss.__rip as usize),
            strip_ptr_tag(ss.__rbp as usize),
        )
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let uc = uctx as *const libc::ucontext_t;
        let mc = (*uc).uc_mcontext;
        if mc.is_null() {
            return (0, 0);
        }
        let ss = &(*mc).__ss;
        (
            strip_ptr_tag(ss.__pc as usize),
            strip_ptr_tag(ss.__fp as usize),
        )
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    {
        (0, 0)
    }
}

unsafe fn walk_frame_pointers(start_fp: usize, out: &mut [usize], lo: usize, hi: usize) -> usize {
    let bounded = hi != 0 && lo < hi;
    // Without registered stack bounds, do not walk far — unbounded FP walks on a
    // deep training stack have caused signal-stack overflows / resume SIGILL.
    let max = if bounded { out.len() } else { out.len().min(8) };
    let in_stack =
        |fp: usize| !bounded || (fp >= lo && fp + 2 * std::mem::size_of::<usize>() <= hi);

    let mut fp = strip_ptr_tag(start_fp);
    let mut count = 0usize;
    while count < max {
        if !plausible(fp) || (fp & 0x7) != 0 || !in_stack(fp) {
            break;
        }
        let saved_fp = strip_ptr_tag(*(fp as *const usize));
        let ret = strip_ptr_tag(*((fp + std::mem::size_of::<usize>()) as *const usize));
        if !plausible(ret) {
            break;
        }
        out[count] = ret;
        count += 1;
        if saved_fp <= fp || saved_fp - fp > 0x20_0000 {
            break;
        }
        fp = saved_fp;
    }
    count
}

/// Options for signal-safe snapshot fill.
#[derive(Clone, Copy, Debug)]
pub struct FillOpts {
    /// Walk frame pointers beyond the interrupted PC. Off for on-demand
    /// `SIGUSR2` (UI refresh) — cheaper and less likely to disturb resume state.
    pub walk_native: bool,
}

impl Default for FillOpts {
    fn default() -> Self {
        Self { walk_native: true }
    }
}

/// Whether the calling thread is currently running on its signal alt stack.
#[cfg(unix)]
#[inline]
pub fn on_signal_altstack() -> bool {
    unsafe {
        let mut cur: libc::stack_t = std::mem::zeroed();
        if libc::sigaltstack(std::ptr::null(), &mut cur) != 0 {
            return false;
        }
        (cur.ss_flags & libc::SS_ONSTACK) != 0
    }
}

#[cfg(not(unix))]
#[inline]
pub fn on_signal_altstack() -> bool {
    false
}

/// Fill `out` from `ucontext` + registered `PYSTACKS` (async-signal-safe).
///
/// Prefer this over returning [`StackSnapshot`] by value in signal handlers.
/// Caller sets [`StackSnapshot::source`].
///
/// # Safety
///
/// `uctx` must be a valid `ucontext_t` from a signal handler (or null).
/// `out` must not be shared with concurrent writers.
pub unsafe fn fill_raw_snapshot(out: &mut StackSnapshot, uctx: *mut c_void) {
    fill_raw_snapshot_with(out, uctx, FillOpts::default());
}

/// Like [`fill_raw_snapshot`] with explicit options.
///
/// # Safety
///
/// Same as [`fill_raw_snapshot`]: `uctx` must be a valid `ucontext_t` from a
/// signal handler (or null); `out` must not be shared with concurrent writers.
pub unsafe fn fill_raw_snapshot_with(out: &mut StackSnapshot, uctx: *mut c_void, opts: FillOpts) {
    // Zero in place — NEVER `*out = StackSnapshot::zeroed()` which materializes
    // a ~1.4 KiB stack temporary and has caused resume SIGILL at `_platform_strlen`.
    core::ptr::write_bytes(
        out as *mut StackSnapshot as *mut u8,
        0,
        core::mem::size_of::<StackSnapshot>(),
    );
    out.tid = current_tid();

    let slot = thread_slot(out.tid);
    let (lo, hi) = match slot {
        Some(s) => (
            s.stack_lo.load(Ordering::Acquire),
            s.stack_hi.load(Ordering::Acquire),
        ),
        None => (0, 0),
    };

    let (pc, fp) = regs_from_uctx(uctx);
    let mut nlen = 0usize;
    if plausible(pc) {
        out.native[nlen] = pc;
        nlen += 1;
    }
    if opts.walk_native && nlen < MAX_NATIVE {
        nlen += walk_frame_pointers(fp, &mut out.native[nlen..], lo, hi);
    }
    out.native_len = nlen as u32;
    if opts.walk_native && nlen == MAX_NATIVE {
        out.flags.insert(StackFlags::NATIVE_TRUNCATED);
    }

    if let Some(slot) = slot {
        let wr = slot.writing.load(Ordering::Acquire);
        let ps = slot.pystacks.load(Ordering::Acquire);
        if wr.is_null() || ps.is_null() {
            out.flags.insert(StackFlags::PY_ABSENT);
        } else if *wr {
            out.flags.insert(StackFlags::PY_TORN);
        } else {
            compiler_fence(Ordering::SeqCst);
            let stacks = &*ps;
            let n = stacks.len().min(MAX_PY);
            for (i, stack) in stacks.iter().enumerate().take(n) {
                out.py[i] = stack.callee();
            }
            compiler_fence(Ordering::SeqCst);
            if *wr {
                out.py_len = 0;
                out.flags.insert(StackFlags::PY_TORN);
            } else {
                out.py_len = n as u32;
                if stacks.len() > MAX_PY {
                    out.flags.insert(StackFlags::PY_TRUNCATED);
                }
            }
        }
    } else {
        out.flags.insert(StackFlags::PY_ABSENT);
    }
}

/// Convenience wrapper (not for deep signal paths — prefer [`fill_raw_snapshot`]).
///
/// # Safety
///
/// Same as [`fill_raw_snapshot`]: `uctx` must be a valid `ucontext_t` from a
/// signal handler (or null).
pub unsafe fn capture_raw_snapshot(uctx: *mut c_void) -> StackSnapshot {
    let mut sample = StackSnapshot::zeroed();
    fill_raw_snapshot(&mut sample, uctx);
    sample
}

/// Install a **per-thread** signal alt stack for `SA_ONSTACK` handlers.
///
/// Critical on Darwin/Linux: `sigaltstack` is not process-wide. Installing only
/// on the HTTP / init thread leaves the Python main thread without an alt stack;
/// SIGPROF/SIGUSR2 then run on a deep training stack and resume as `SIGILL`
/// (observed at `_platform_strlen` after signal-frame corruption).
/// Minimum alt-stack size shared with the crash handler (256 KiB).
const SIGNAL_ALTSTACK_BYTES: usize = 256 * 1024;

#[cfg(unix)]
pub fn ensure_signal_altstack() {
    THREAD_ALTSTACK_READY.with(|ready| {
        if ready.get() {
            return;
        }
        unsafe {
            // Reuse crash-handler / prior alt stack — do NOT replace a larger
            // stack with a smaller one (that made SIGILL backtraces empty and
            // left stack capture on an undersized buffer).
            let mut cur: libc::stack_t = std::mem::zeroed();
            if libc::sigaltstack(std::ptr::null(), &mut cur) == 0
                && (cur.ss_flags & libc::SS_DISABLE) == 0
                && cur.ss_size >= SIGNAL_ALTSTACK_BYTES
            {
                ready.set(true);
                return;
            }
        }
        let mut buf = vec![0u8; SIGNAL_ALTSTACK_BYTES];
        let sp = buf.as_mut_ptr() as *mut c_void;
        let size = buf.len();
        std::mem::forget(buf);
        unsafe {
            let ss = libc::stack_t {
                ss_sp: sp,
                ss_size: size,
                ss_flags: 0,
            };
            if libc::sigaltstack(&ss, std::ptr::null_mut()) != 0 {
                log::warn!(
                    "probing: per-thread sigaltstack failed (tid={}); \
                     SIGPROF/SIGUSR2 may be unsafe on deep stacks",
                    current_tid()
                );
                return;
            }
        }
        ready.set(true);
    });
}

#[cfg(not(unix))]
pub fn ensure_signal_altstack() {}

pub fn symbolize_native_addr(addr: usize, cache: &mut HashMap<usize, CallFrame>) -> CallFrame {
    if let Some(frame) = cache.get(&addr) {
        return frame.clone();
    }
    let mut resolved_name: Option<String> = None;
    let mut file_name = String::new();
    let mut lineno = 0i64;
    let mut lang: Option<String> = None;
    backtrace::resolve(addr as *mut c_void, |sym| {
        if resolved_name.is_none() {
            if let Some(name) = sym.name().and_then(|n| n.as_str()) {
                let (demangled, tag) = demangle_native_symbol(name);
                resolved_name = Some(demangled);
                lang = tag.map(str::to_string);
            }
            if let Some(path) = sym.filename() {
                file_name = path.to_string_lossy().into_owned();
            }
            lineno = sym.lineno().unwrap_or(0) as i64;
        }
    });
    let func = resolved_name.unwrap_or_else(|| format!("0x{addr:x}"));
    let frame = CallFrame::CFrame {
        ip: format!("{addr:#x}"),
        file: file_name,
        func,
        lineno,
        lang,
    };
    cache.insert(addr, frame.clone());
    frame
}

// ---------------------------------------------------------------------------
// SIGUSR2 on-demand capture (same safe handler body as SIGPROF)
// ---------------------------------------------------------------------------

struct Sigusr2SnapshotSlot(UnsafeCell<StackSnapshot>);
unsafe impl Sync for Sigusr2SnapshotSlot {}

const ZERO_SNAPSHOT: StackSnapshot = StackSnapshot::zeroed();

static SIGUSR2_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);
static SIGUSR2_ARMED: AtomicBool = AtomicBool::new(false);
/// When armed, only this OS thread id may publish into [`SIGUSR2_SNAPSHOT`].
static SIGUSR2_TARGET_TID: AtomicU64 = AtomicU64::new(0);
static SIGUSR2_SEQ: AtomicU64 = AtomicU64::new(0);
static SIGUSR2_SNAPSHOT: Sigusr2SnapshotSlot = Sigusr2SnapshotSlot(UnsafeCell::new(ZERO_SNAPSHOT));

/// Generation counter before arming; [`take_sigusr2_snapshot`] returns data when seq advances.
pub fn sigusr2_capture_generation() -> u64 {
    SIGUSR2_SEQ.load(Ordering::Acquire)
}

pub fn set_sigusr2_armed(armed: bool) {
    SIGUSR2_ARMED.store(armed, Ordering::Release);
}

pub fn take_sigusr2_snapshot(after_seq: u64) -> Option<StackSnapshot> {
    if SIGUSR2_SEQ.load(Ordering::Acquire) <= after_seq {
        return None;
    }
    let snap = unsafe { *SIGUSR2_SNAPSHOT.0.get() };
    if snap.is_empty() {
        None
    } else {
        Some(snap)
    }
}

struct Sigusr2ArmGuard;

impl Sigusr2ArmGuard {
    fn new(target_tid: u64) -> Self {
        SIGUSR2_TARGET_TID.store(target_tid, Ordering::Release);
        SIGUSR2_ARMED.store(true, Ordering::Release);
        Sigusr2ArmGuard
    }
}

impl Drop for Sigusr2ArmGuard {
    fn drop(&mut self) {
        SIGUSR2_ARMED.store(false, Ordering::Release);
        SIGUSR2_TARGET_TID.store(0, Ordering::Release);
    }
}

/// Arm, deliver SIGUSR2 to `tid`, and wait for an async-signal-safe snapshot slot.
#[cfg(unix)]
pub fn capture_thread_snapshot_signal(tid: u64, timeout: Duration) -> Option<StackSnapshot> {
    use std::time::Instant;

    let _guard = Sigusr2ArmGuard::new(tid);
    let seq_before = sigusr2_capture_generation();
    let tid_i32 = tid as i32;

    #[cfg(target_os = "linux")]
    {
        let pid = nix::unistd::getpid().as_raw();
        let ret = unsafe { libc::syscall(libc::SYS_tgkill, pid, tid_i32, libc::SIGUSR2) };
        if ret != 0 {
            return None;
        }
    }

    #[cfg(target_os = "macos")]
    {
        if probing_core::signal::send_sigusr2_to_thread_id(tid_i32).is_err() {
            return None;
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (pid, tid_i32, timeout);
        return None;
    }

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(snap) = take_sigusr2_snapshot(seq_before) {
            if sigusr2_snapshot_matches_target(&snap, tid) {
                return Some(snap);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

#[cfg(not(unix))]
pub fn capture_thread_snapshot_signal(_tid: u64, _timeout: Duration) -> Option<StackSnapshot> {
    None
}

#[cfg(unix)]
pub fn install_sigusr2_handler() {
    if SIGUSR2_HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    ensure_signal_altstack();
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigusr2_stack_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART | libc::SA_ONSTACK;
        libc::sigemptyset(&mut sa.sa_mask);
        // Avoid nesting with SIGPROF while filling on the shared alt stack.
        libc::sigaddset(&mut sa.sa_mask, libc::SIGPROF);
        libc::sigaction(libc::SIGUSR2, &sa, std::ptr::null_mut());
    }
}

#[cfg(unix)]
unsafe extern "C" fn sigusr2_stack_handler(
    _sig: c_int,
    _info: *mut libc::siginfo_t,
    uctx: *mut c_void,
) {
    if !SIGUSR2_ARMED.load(Ordering::Acquire) {
        return;
    }
    // Refuse to run on the training stack — would corrupt resume into SIGILL.
    if !on_signal_altstack() {
        return;
    }
    let target = SIGUSR2_TARGET_TID.load(Ordering::Acquire);
    let slot = &mut *SIGUSR2_SNAPSHOT.0.get();
    // On-demand UI capture: PC + Python keys only (no FP walk).
    fill_raw_snapshot_with(slot, uctx, FillOpts { walk_native: false });
    slot.source = StackSource::Sigusr2;
    if !sigusr2_snapshot_matches_target(slot, target) {
        core::ptr::write_bytes(
            slot as *mut StackSnapshot as *mut u8,
            0,
            core::mem::size_of::<StackSnapshot>(),
        );
        return;
    }
    SIGUSR2_SEQ.fetch_add(1, Ordering::Release);
}

#[cfg(not(unix))]
pub fn install_sigusr2_handler() {}

/// Serialize process-global signal handler tests (SIGPROF / SIGUSR2).
#[cfg(all(test, unix))]
pub(crate) fn with_signal_test_lock<R>(f: impl FnOnce() -> R) -> R {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_with_tid(tid: u64) -> StackSnapshot {
        let mut s = StackSnapshot::zeroed();
        s.source = StackSource::Sigusr2;
        s.tid = tid;
        s.py_len = 1;
        s.py[0] = 0x1000;
        s
    }

    fn claim_test_slot(tid: u64) {
        let start = slot_hash(tid);
        for i in 0..REG_SIZE {
            let slot = &REG_TABLE[(start + i) & (REG_SIZE - 1)];
            let v = slot.tid.load(Ordering::Acquire);
            if v == tid {
                return;
            }
            if v == 0
                && slot
                    .tid
                    .compare_exchange(0, tid, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return;
            }
        }
        panic!("could not claim registry slot for tid={tid}");
    }

    #[test]
    fn register_main_os_tid_is_idempotent() {
        register_main_os_tid();
        let first = python_main_os_tid();
        register_main_os_tid();
        assert_eq!(first, python_main_os_tid());
        assert!(first.is_some());
    }

    #[test]
    fn py_frame_symbol_folded_label_uses_basename() {
        let sym = PyFrameSymbol {
            func: "main".into(),
            file: "examples/imagenet_with_span.py".into(),
            lineno: 316,
        };
        assert_eq!(sym.folded_label(), "[py] main (imagenet_with_span.py:316)");
    }

    #[test]
    fn py_frame_symbol_call_frame_keeps_full_path() {
        let sym = PyFrameSymbol {
            func: "main".into(),
            file: "examples/imagenet_with_span.py".into(),
            lineno: 316,
        };
        let frame = sym.to_call_frame();
        match frame {
            CallFrame::PyFrame {
                file, func, lineno, ..
            } => {
                assert_eq!(file, "examples/imagenet_with_span.py");
                assert_eq!(func, "main");
                assert_eq!(lineno, 316);
            }
            other => panic!("expected PyFrame, got {other:?}"),
        }
    }

    #[test]
    fn sigusr2_rejects_wrong_tid_and_empty_snapshot() {
        let good = sample_with_tid(42);
        assert!(sigusr2_snapshot_matches_target(&good, 42));
        assert!(!sigusr2_snapshot_matches_target(&good, 99));
        assert!(!sigusr2_snapshot_matches_target(&good, 0));
        assert!(!sigusr2_snapshot_matches_target(
            &StackSnapshot::zeroed(),
            42
        ));
    }

    #[test]
    fn latest_snapshot_read_rejects_torn_or_mismatched_tid() {
        let snap = sample_with_tid(7);
        assert!(latest_snapshot_read_ok(3, 3, &snap, 7));
        assert!(!latest_snapshot_read_ok(0, 0, &snap, 7));
        assert!(!latest_snapshot_read_ok(3, 4, &snap, 7));
        assert!(!latest_snapshot_read_ok(3, 3, &snap, 8));
        assert!(!latest_snapshot_read_ok(3, 3, &StackSnapshot::zeroed(), 7));
    }

    #[test]
    fn latest_snapshot_roundtrip_requires_registered_tid() {
        // High tid to avoid colliding with live process threads during tests.
        let tid = 0xC0FF_EE42u64;
        claim_test_slot(tid);
        let snap = sample_with_tid(tid);
        store_latest_snapshot(&snap);
        let got = latest_snapshot_for_tid(tid).expect("latest snapshot");
        assert_eq!(got.tid, tid);
        assert_eq!(got.py_len, 1);
        assert!(latest_snapshot_for_tid(tid + 1).is_none());
    }

    /// Real SIGUSR2 delivery → async-signal-safe fill → `native_len >= 1`.
    #[cfg(unix)]
    #[test]
    fn sigusr2_signal_path_captures_native_pc() {
        with_signal_test_lock(|| {
            install_sigusr2_handler();
            register_python_thread();
            register_main_os_tid();
            let tid = current_tid();
            let snap = capture_thread_snapshot_signal(tid, Duration::from_secs(2))
                .expect("SIGUSR2 should publish a snapshot for the current thread");
            assert_eq!(snap.tid, tid);
            assert_eq!(snap.source, StackSource::Sigusr2);
            assert!(
                snap.native_len >= 1,
                "ucontext PC should yield at least one native frame, got native_len=0 flags={:?}",
                snap.flags
            );
        });
    }
}
