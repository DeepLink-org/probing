//! Async-signal-safe fill of [`StackSnapshot`] for SIGPROF and SIGUSR2.
//!
//! Handlers only copy raw PCs and eval-hook keys. Symbolize / merge / fold live
//! in [`crate::features::stacktrace::parse`] and [`crate::features::stacktrace::fold`].

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::Duration;

use core::ffi::{c_int, c_void};
use nix::libc;
use once_cell::sync::Lazy;
use probing_proto::prelude::CallFrame;

use crate::features::stacktrace::merge::demangle_native_symbol;
use crate::features::stacktrace::spy::call::RawCallLocation;
use crate::features::stacktrace::spy::PublishedPyStack;

pub use crate::features::stacktrace::snapshot::{
    RawStackSnapshot, StackFlags, StackSnapshot, StackSource, MAX_NATIVE, MAX_PY,
};

const REG_SIZE: usize = 1024;
const LATEST_BUFFERS: usize = 2;
const NO_LATEST: u64 = u64::MAX;
const REGISTRY_BUSY: u64 = 1;

struct LatestSnapshots {
    buffers: [UnsafeCell<StackSnapshot>; LATEST_BUFFERS],
    readers: [AtomicUsize; LATEST_BUFFERS],
    published: AtomicU64,
    writer: AtomicBool,
}

// SAFETY: writers only mutate an unpublished buffer with no registered readers;
// readers pin the published buffer before copying it.
unsafe impl Sync for LatestSnapshots {}

struct LatestWriterGuard<'a>(&'a AtomicBool);

impl Drop for LatestWriterGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl LatestSnapshots {
    const fn new() -> Self {
        Self {
            buffers: [const { UnsafeCell::new(StackSnapshot::zeroed()) }; LATEST_BUFFERS],
            readers: [const { AtomicUsize::new(0) }; LATEST_BUFFERS],
            published: AtomicU64::new(NO_LATEST),
            writer: AtomicBool::new(false),
        }
    }

    fn try_write_with(&self, fill: impl FnOnce(&mut StackSnapshot) -> bool) -> bool {
        if self
            .writer
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        let _guard = LatestWriterGuard(&self.writer);
        let current = self.published.load(Ordering::Acquire);
        let current_index = (current != NO_LATEST).then_some((current as usize) & 1);
        let Some(index) = (0..LATEST_BUFFERS).find(|index| {
            Some(*index) != current_index && self.readers[*index].load(Ordering::SeqCst) == 0
        }) else {
            return false;
        };

        let output = unsafe { &mut *self.buffers[index].get() };
        if !fill(output) {
            return false;
        }
        let generation = if current == NO_LATEST {
            1
        } else {
            (current >> 1).wrapping_add(1)
        };
        self.published
            .store((generation << 1) | index as u64, Ordering::Release);
        true
    }

    fn read(&self) -> Option<(StackSnapshot, u64)> {
        for _ in 0..4 {
            let token = self.published.load(Ordering::Acquire);
            if token == NO_LATEST {
                return None;
            }
            let index = (token as usize) & 1;
            self.readers[index].fetch_add(1, Ordering::SeqCst);
            if self.published.load(Ordering::Acquire) != token {
                self.readers[index].fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let snapshot = unsafe { *self.buffers[index].get() };
            self.readers[index].fetch_sub(1, Ordering::SeqCst);
            if self.published.load(Ordering::Acquire) == token {
                return Some((snapshot, token >> 1));
            }
        }
        None
    }

    fn reset_after_fork(&self) {
        // Called only while REGISTRY_STATE is busy: after fork the caller is
        // the sole surviving thread, and at startup no signal reader can pass
        // `registry_ready` until the reset is published.
        self.writer.store(false, Ordering::Relaxed);
        for readers in &self.readers {
            readers.store(0, Ordering::Relaxed);
        }
        self.published.store(NO_LATEST, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Python-thread registry (process-owned storage, registered in normal context)
// ---------------------------------------------------------------------------

struct ThreadSlot {
    tid: AtomicU64,
    ready: AtomicBool,
    pystack: PublishedPyStack,
    stack_lo: AtomicUsize,
    stack_hi: AtomicUsize,
    latest: LatestSnapshots,
    sigusr2_reply: LatestSnapshots,
}

static REG_TABLE: [ThreadSlot; REG_SIZE] = [const {
    ThreadSlot {
        tid: AtomicU64::new(0),
        ready: AtomicBool::new(false),
        pystack: PublishedPyStack::new(),
        stack_lo: AtomicUsize::new(0),
        stack_hi: AtomicUsize::new(0),
        latest: LatestSnapshots::new(),
        sigusr2_reply: LatestSnapshots::new(),
    }
}; REG_SIZE];

static REG_FULL_WARNED: AtomicBool = AtomicBool::new(false);
static REGISTRY_STATE: AtomicU64 = AtomicU64::new(0);
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

#[cfg(unix)]
struct ThreadAltStack {
    ready: bool,
    owned: Option<Box<[u8]>>,
}

#[cfg(unix)]
impl Drop for ThreadAltStack {
    fn drop(&mut self) {
        let Some(buffer) = self.owned.take() else {
            return;
        };
        unsafe {
            let mut current: libc::stack_t = std::mem::zeroed();
            if libc::sigaltstack(std::ptr::null(), &mut current) != 0 {
                std::mem::forget(buffer);
                return;
            }
            if current.ss_sp == buffer.as_ptr() as *mut c_void {
                if (current.ss_flags & libc::SS_ONSTACK) != 0 {
                    std::mem::forget(buffer);
                    return;
                }
                let mut signals: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut signals);
                libc::sigaddset(&mut signals, libc::SIGPROF);
                libc::sigaddset(&mut signals, libc::SIGUSR2);
                if libc::pthread_sigmask(libc::SIG_BLOCK, &signals, std::ptr::null_mut()) != 0 {
                    std::mem::forget(buffer);
                    return;
                }
                let disabled = libc::stack_t {
                    ss_sp: std::ptr::null_mut(),
                    ss_size: 0,
                    ss_flags: libc::SS_DISABLE,
                };
                if libc::sigaltstack(&disabled, std::ptr::null_mut()) != 0 {
                    std::mem::forget(buffer);
                }
            }
        }
    }
}

thread_local! {
    /// Per-thread signal alt stack installed (`sigaltstack` is per-thread on Darwin/Linux).
    #[cfg(unix)]
    static THREAD_ALTSTACK: std::cell::UnsafeCell<ThreadAltStack> =
        const { std::cell::UnsafeCell::new(ThreadAltStack { ready: false, owned: None }) };
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
    THREAD_NAMES
        .try_read()
        .ok()
        .and_then(|m| m.get(&tid).cloned())
}

fn registry_token() -> u64 {
    #[cfg(unix)]
    let pid = unsafe { libc::getpid() as u32 };
    #[cfg(not(unix))]
    let pid = std::process::id();
    (pid as u64) << 1
}

fn registry_ready() -> bool {
    REGISTRY_STATE.load(Ordering::Acquire) == registry_token()
}

fn ensure_registry_process() {
    let target = registry_token();
    loop {
        let state = REGISTRY_STATE.load(Ordering::Acquire);
        if state == target {
            return;
        }
        if state == (target | REGISTRY_BUSY) {
            std::hint::spin_loop();
            continue;
        }
        if REGISTRY_STATE
            .compare_exchange(
                state,
                target | REGISTRY_BUSY,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            continue;
        }

        // Startup/fork-child path: only the winning normal-context thread
        // rebuilds inherited process state. Signal readers stay disabled until
        // the final Release store publishes `target`.
        for slot in &REG_TABLE {
            slot.ready.store(false, Ordering::Relaxed);
            slot.tid.store(0, Ordering::Relaxed);
            slot.stack_lo.store(0, Ordering::Relaxed);
            slot.stack_hi.store(0, Ordering::Relaxed);
            slot.pystack.clear();
            slot.latest.reset_after_fork();
            slot.sigusr2_reply.reset_after_fork();
        }
        MAIN_OS_TID.store(0, Ordering::Relaxed);
        REG_FULL_WARNED.store(false, Ordering::Relaxed);
        SIGUSR2_ARMED.store(false, Ordering::Relaxed);
        SIGUSR2_TARGET_TID.store(0, Ordering::Relaxed);
        SIGUSR2_CAPTURE_BUSY.store(false, Ordering::Relaxed);
        REGISTRY_STATE.store(target, Ordering::Release);
        return;
    }
}

pub(crate) fn register_python_thread_slot() -> Option<&'static PublishedPyStack> {
    ensure_registry_process();
    ensure_signal_altstack();
    let tid = current_tid();
    if probing_core::is_python_main_thread() {
        register_main_os_tid();
    }
    let (lo, hi) = current_stack_bounds();

    if let Some(name) = current_thread_name() {
        if let Ok(mut m) = THREAD_NAMES.try_write() {
            m.insert(tid, name);
        }
    }

    let publish = |slot: &'static ThreadSlot| -> &'static PublishedPyStack {
        slot.ready.store(false, Ordering::Release);
        slot.stack_lo.store(lo, Ordering::Release);
        slot.stack_hi.store(hi, Ordering::Release);
        slot.ready.store(true, Ordering::Release);
        &slot.pystack
    };

    let start = slot_hash(tid);
    for i in 0..REG_SIZE {
        let slot = &REG_TABLE[(start + i) & (REG_SIZE - 1)];
        let v = slot.tid.load(Ordering::Acquire);
        if v == tid {
            return Some(publish(slot));
        }
        if v == 0
            && slot
                .tid
                .compare_exchange(0, tid, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            slot.pystack.clear();
            slot.latest.reset_after_fork();
            slot.sigusr2_reply.reset_after_fork();
            return Some(publish(slot));
        }
    }

    if !REG_FULL_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "probing: stack thread registry full ({REG_SIZE} threads); \
             Python stacks for further threads will be missing"
        );
    }
    None
}

pub fn register_python_thread() {
    let _ = register_python_thread_slot();
}

fn thread_slot(tid: u64) -> Option<&'static ThreadSlot> {
    if !registry_ready() {
        return None;
    }
    let start = slot_hash(tid);
    for i in 0..REG_SIZE {
        let slot = &REG_TABLE[(start + i) & (REG_SIZE - 1)];
        let v = slot.tid.load(Ordering::Acquire);
        if v == tid {
            return slot.ready.load(Ordering::Acquire).then_some(slot);
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

    let (n, truncated, stable) = slot.pystack.copy_into(&mut sample.py);
    if !stable {
        sample.flags.insert(StackFlags::PY_TORN);
        return None;
    }
    sample.py_len = n as u32;
    if truncated {
        sample.flags.insert(StackFlags::PY_TRUNCATED);
    }
    if sample.is_empty() {
        sample.flags.insert(StackFlags::PY_ABSENT);
        None
    } else {
        Some(sample)
    }
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
    if !slot.latest.try_write_with(|output| {
        unsafe {
            core::ptr::copy_nonoverlapping(snapshot, output, 1);
        }
        true
    }) {
        crate::features::stacktrace::metrics::inc_dropped_publish();
    }
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
    let published = slot.latest.try_write_with(|latest| {
        fill_raw_snapshot_with(latest, uctx, opts);
        latest.source = source;
        !latest.is_empty()
    });
    if !published {
        crate::features::stacktrace::metrics::inc_dropped_publish();
    }
    published
}

/// Reuse the latest SIGPROF snapshot for `tid` when CPU sampling is active.
pub fn latest_snapshot_for_tid(tid: u64) -> Option<StackSnapshot> {
    latest_snapshot_with_seq(tid).map(|(snap, _)| snap)
}

/// Like [`latest_snapshot_for_tid`], also returning the slot generation for view caches.
pub fn latest_snapshot_with_seq(tid: u64) -> Option<(StackSnapshot, u64)> {
    let slot = thread_slot(tid)?;
    let (snapshot, generation) = slot.latest.read()?;
    (snapshot.tid == tid && !snapshot.is_empty()).then_some((snapshot, generation))
}

pub fn intern_py_frame(loc: &RawCallLocation) {
    let key = loc.callee();
    if key == 0 {
        return;
    }
    if let Ok(g) = PY_SYMBOLS.try_read() {
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
    if let Ok(mut g) = PY_SYMBOLS.try_write() {
        if g.len() < PY_SYMBOLS_CAP {
            g.entry(key).or_insert(entry);
        }
    }
}

pub fn clear_py_symbols() {
    if let Ok(mut g) = PY_SYMBOLS.try_write() {
        g.clear();
        g.shrink_to_fit();
    }
}

pub(crate) fn resolve_py_label(key: usize) -> String {
    if key != 0 {
        if let Ok(g) = PY_SYMBOLS.try_read() {
            if let Some(sym) = g.get(&key) {
                return sym.folded_label();
            }
        }
    }
    "[py] <unknown>".to_string()
}

pub(crate) fn resolve_py_call_frame(key: usize) -> CallFrame {
    if key != 0 {
        if let Ok(g) = PY_SYMBOLS.try_read() {
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
        let (n, truncated, stable) = slot.pystack.copy_into(&mut out.py);
        if !stable {
            out.py_len = 0;
            out.flags.insert(StackFlags::PY_TORN);
        } else {
            out.py_len = n as u32;
            if truncated {
                out.flags.insert(StackFlags::PY_TRUNCATED);
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
    THREAD_ALTSTACK.with(|state| {
        let state = unsafe { &mut *state.get() };
        if state.ready {
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
                state.ready = true;
                return;
            }
        }
        let mut buffer = vec![0u8; SIGNAL_ALTSTACK_BYTES].into_boxed_slice();
        let sp = buffer.as_mut_ptr() as *mut c_void;
        let size = buffer.len();
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
        state.owned = Some(buffer);
        state.ready = true;
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
// macOS on-demand capture without delivering a signal to the target thread
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod mach_suspend {
    use super::*;

    const KERN_SUCCESS: libc::kern_return_t = 0;

    #[cfg(target_arch = "aarch64")]
    const THREAD_STATE_FLAVOR: libc::c_int = 6; // ARM_THREAD_STATE64
    #[cfg(target_arch = "x86_64")]
    const THREAD_STATE_FLAVOR: libc::c_int = 4; // x86_THREAD_STATE64

    #[cfg(target_arch = "aarch64")]
    #[repr(C)]
    #[derive(Default)]
    struct ThreadState64 {
        x: [u64; 29],
        fp: u64,
        lr: u64,
        sp: u64,
        pc: u64,
        cpsr: u32,
        pad: u32,
    }

    #[cfg(target_arch = "x86_64")]
    #[repr(C)]
    #[derive(Default)]
    struct ThreadState64 {
        rax: u64,
        rbx: u64,
        rcx: u64,
        rdx: u64,
        rdi: u64,
        rsi: u64,
        rbp: u64,
        rsp: u64,
        r8: u64,
        r9: u64,
        r10: u64,
        r11: u64,
        r12: u64,
        r13: u64,
        r14: u64,
        r15: u64,
        rip: u64,
        rflags: u64,
        cs: u64,
        fs: u64,
        gs: u64,
    }

    impl ThreadState64 {
        fn count() -> libc::mach_msg_type_number_t {
            (core::mem::size_of::<Self>() / core::mem::size_of::<libc::natural_t>())
                as libc::mach_msg_type_number_t
        }

        #[cfg(target_arch = "aarch64")]
        fn pc_fp(&self) -> (usize, usize) {
            (
                strip_ptr_tag(self.pc as usize),
                strip_ptr_tag(self.fp as usize),
            )
        }

        #[cfg(target_arch = "x86_64")]
        fn pc_fp(&self) -> (usize, usize) {
            (self.rip as usize, self.rbp as usize)
        }
    }

    unsafe extern "C" {
        fn thread_suspend(thread: libc::thread_act_t) -> libc::kern_return_t;
        fn thread_resume(thread: libc::thread_act_t) -> libc::kern_return_t;
        fn thread_get_state(
            thread: libc::thread_act_t,
            flavor: libc::c_int,
            state: *mut libc::natural_t,
            count: *mut libc::mach_msg_type_number_t,
        ) -> libc::kern_return_t;
        fn mach_vm_read_overwrite(
            task: libc::vm_map_t,
            address: libc::mach_vm_address_t,
            size: libc::mach_vm_size_t,
            data: libc::mach_vm_address_t,
            out_size: *mut libc::mach_vm_size_t,
        ) -> libc::kern_return_t;
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
    }

    struct ThreadList {
        task: libc::mach_port_t,
        ports: libc::thread_act_array_t,
        count: libc::mach_msg_type_number_t,
    }

    impl Drop for ThreadList {
        fn drop(&mut self) {
            unsafe {
                for index in 0..self.count as usize {
                    let _ = mach_port_deallocate(self.task, *self.ports.add(index));
                }
                let bytes = self.count as usize * core::mem::size_of::<libc::thread_act_t>();
                let _ = libc::vm_deallocate(self.task, self.ports as usize, bytes);
            }
        }
    }

    impl ThreadList {
        fn find(tid: u64) -> Option<(Self, libc::thread_act_t)> {
            #[allow(deprecated)]
            let task = unsafe { libc::mach_task_self() };
            let mut ports: libc::thread_act_array_t = core::ptr::null_mut();
            let mut count = 0;
            if unsafe { libc::task_threads(task, &mut ports, &mut count) } != KERN_SUCCESS
                || ports.is_null()
            {
                return None;
            }
            let list = Self { task, ports, count };
            for index in 0..count as usize {
                let port = unsafe { *ports.add(index) };
                let mut info: libc::thread_identifier_info_data_t = unsafe { core::mem::zeroed() };
                let mut info_count = libc::THREAD_IDENTIFIER_INFO_COUNT;
                let rc = unsafe {
                    libc::thread_info(
                        port,
                        libc::THREAD_IDENTIFIER_INFO as libc::thread_flavor_t,
                        (&mut info as *mut libc::thread_identifier_info_data_t).cast(),
                        &mut info_count,
                    )
                };
                if rc == KERN_SUCCESS && info.thread_id == tid {
                    return Some((list, port));
                }
            }
            None
        }
    }

    struct SuspendGuard(libc::thread_act_t);

    impl SuspendGuard {
        fn new(thread: libc::thread_act_t) -> Option<Self> {
            (unsafe { thread_suspend(thread) } == KERN_SUCCESS).then_some(Self(thread))
        }
    }

    impl Drop for SuspendGuard {
        fn drop(&mut self) {
            // Keep this path allocation/lock-free too. In particular, logging
            // a resume failure could deadlock if the stopped target owns a
            // logger or allocator lock.
            let _ = unsafe { thread_resume(self.0) };
        }
    }

    fn read_pair(task: libc::vm_map_t, fp: usize) -> Option<[usize; 2]> {
        let mut pair = [0usize; 2];
        let mut read = 0;
        let rc = unsafe {
            mach_vm_read_overwrite(
                task,
                fp as libc::mach_vm_address_t,
                core::mem::size_of_val(&pair) as libc::mach_vm_size_t,
                pair.as_mut_ptr() as libc::mach_vm_address_t,
                &mut read,
            )
        };
        (rc == KERN_SUCCESS && read as usize == core::mem::size_of_val(&pair)).then_some(pair)
    }

    pub(super) fn capture(tid: u64) -> Option<StackSnapshot> {
        if tid == 0 || tid == current_tid() {
            return None;
        }
        let slot = thread_slot(tid)?;
        let (threads, thread) = ThreadList::find(tid)?;
        let mut native = [0usize; MAX_NATIVE];
        let mut native_len = 0usize;

        {
            // Do not allocate, symbolize, or take userspace locks while the
            // target is stopped: it may own the allocator or loader lock.
            let _suspended = SuspendGuard::new(thread)?;
            let mut state = ThreadState64::default();
            let mut state_count = ThreadState64::count();
            let rc = unsafe {
                thread_get_state(
                    thread,
                    THREAD_STATE_FLAVOR,
                    (&mut state as *mut ThreadState64).cast(),
                    &mut state_count,
                )
            };
            if rc != KERN_SUCCESS {
                return None;
            }

            let (pc, mut fp) = state.pc_fp();
            if plausible(pc) {
                native[0] = pc;
                native_len = 1;
            }
            let lo = slot.stack_lo.load(Ordering::Acquire);
            let hi = slot.stack_hi.load(Ordering::Acquire);
            while native_len < MAX_NATIVE
                && plausible(fp)
                && (fp & (core::mem::align_of::<usize>() - 1)) == 0
                && lo != 0
                && fp >= lo
                && fp + 2 * core::mem::size_of::<usize>() <= hi
            {
                let Some(pair) = read_pair(threads.task, fp) else {
                    break;
                };
                let next_fp = strip_ptr_tag(pair[0]);
                let ret = strip_ptr_tag(pair[1]);
                if !plausible(ret) {
                    break;
                }
                native[native_len] = ret;
                native_len += 1;
                if next_fp <= fp || next_fp - fp > 0x20_0000 {
                    break;
                }
                fp = next_fp;
            }
        }

        let py = copy_registered_py_snapshot(tid);
        let (py_keys, mut flags) = py
            .as_ref()
            .map_or((&[][..], StackFlags::PY_ABSENT), |snapshot| {
                (&snapshot.py[..snapshot.py_len as usize], snapshot.flags)
            });
        if native_len == MAX_NATIVE {
            flags.insert(StackFlags::NATIVE_TRUNCATED);
        }
        Some(StackSnapshot::from_parts(
            tid,
            StackSource::MachSuspend,
            &native[..native_len],
            py_keys,
            flags,
        ))
    }
}

/// Capture a macOS thread from a control thread without injecting a signal.
#[cfg(target_os = "macos")]
pub fn capture_thread_snapshot_suspended(tid: u64) -> Option<StackSnapshot> {
    mach_suspend::capture(tid)
}

#[cfg(not(target_os = "macos"))]
pub fn capture_thread_snapshot_suspended(_tid: u64) -> Option<StackSnapshot> {
    None
}

// ---------------------------------------------------------------------------
// SIGUSR2 on-demand capture (same safe handler body as SIGPROF)
// ---------------------------------------------------------------------------

static SIGUSR2_HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);
static SIGUSR2_ARMED: AtomicBool = AtomicBool::new(false);
static SIGUSR2_CAPTURE_BUSY: AtomicBool = AtomicBool::new(false);
/// When armed, only this OS thread id may publish into its registry reply slot.
static SIGUSR2_TARGET_TID: AtomicU64 = AtomicU64::new(0);
static SIGUSR2_SEQ: AtomicU64 = AtomicU64::new(0);

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
    let target_tid = SIGUSR2_TARGET_TID.load(Ordering::Acquire);
    let slot = thread_slot(target_tid)?;
    let (snapshot, _) = slot.sigusr2_reply.read()?;
    (snapshot.source == StackSource::Sigusr2
        && sigusr2_snapshot_matches_target(&snapshot, target_tid))
    .then_some(snapshot)
}

struct Sigusr2CaptureGuard;

impl Sigusr2CaptureGuard {
    fn try_new() -> Option<Self> {
        SIGUSR2_CAPTURE_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for Sigusr2CaptureGuard {
    fn drop(&mut self) {
        SIGUSR2_CAPTURE_BUSY.store(false, Ordering::Release);
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

    if !SIGUSR2_HANDLER_INSTALLED.load(Ordering::Acquire) {
        return None;
    }
    let _capture_guard = Sigusr2CaptureGuard::try_new()?;
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
    if SIGUSR2_HANDLER_INSTALLED.load(Ordering::Acquire) {
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
        if libc::sigaction(libc::SIGUSR2, &sa, std::ptr::null_mut()) == 0 {
            SIGUSR2_HANDLER_INSTALLED.store(true, Ordering::Release);
        } else {
            log::warn!(
                "probing: failed to install SIGUSR2 stack handler: {}",
                std::io::Error::last_os_error()
            );
        }
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
    if target == 0 || current_tid() != target {
        return;
    }
    let Some(thread_slot) = thread_slot(target) else {
        return;
    };
    let published = thread_slot.sigusr2_reply.try_write_with(|snapshot| {
        // Darwin keeps signal capture to the interrupted PC; the default main
        // path uses Mach suspension instead. Linux can safely walk the bounded
        // registered stack and needs the full C/C++ tower for on-demand UI.
        fill_raw_snapshot_with(
            snapshot,
            uctx,
            FillOpts {
                walk_native: !cfg!(target_os = "macos"),
            },
        );
        snapshot.source = StackSource::Sigusr2;
        sigusr2_snapshot_matches_target(snapshot, target)
    });
    if published {
        SIGUSR2_SEQ.fetch_add(1, Ordering::Release);
    } else {
        crate::features::stacktrace::metrics::inc_dropped_publish();
    }
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
        ensure_registry_process();
        let start = slot_hash(tid);
        for i in 0..REG_SIZE {
            let slot = &REG_TABLE[(start + i) & (REG_SIZE - 1)];
            let v = slot.tid.load(Ordering::Acquire);
            if v == tid {
                slot.ready.store(true, Ordering::Release);
                return;
            }
            if v == 0
                && slot
                    .tid
                    .compare_exchange(0, tid, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                slot.ready.store(true, Ordering::Release);
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
            file: "examples/imagenet/imagenet_with_span.py".into(),
            lineno: 316,
        };
        assert_eq!(sym.folded_label(), "[py] main (imagenet_with_span.py:316)");
    }

    #[test]
    fn py_frame_symbol_call_frame_keeps_full_path() {
        let sym = PyFrameSymbol {
            func: "main".into(),
            file: "examples/imagenet/imagenet_with_span.py".into(),
            lineno: 316,
        };
        let frame = sym.to_call_frame();
        match frame {
            CallFrame::PyFrame {
                file, func, lineno, ..
            } => {
                assert_eq!(file, "examples/imagenet/imagenet_with_span.py");
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
    fn sigusr2_capture_guard_rejects_concurrent_request() {
        with_signal_test_lock(|| {
            let first = Sigusr2CaptureGuard::try_new().expect("first capture guard");
            assert!(Sigusr2CaptureGuard::try_new().is_none());
            drop(first);
            assert!(Sigusr2CaptureGuard::try_new().is_some());
        });
    }

    #[test]
    fn latest_snapshot_double_buffer_roundtrip() {
        let latest = LatestSnapshots::new();
        let first = sample_with_tid(7);
        assert!(latest.try_write_with(|output| {
            *output = first;
            true
        }));
        let (snapshot, generation) = latest.read().expect("first snapshot");
        assert_eq!(snapshot.tid, 7);
        assert_eq!(generation, 1);

        let second = sample_with_tid(8);
        assert!(latest.try_write_with(|output| {
            *output = second;
            true
        }));
        let (snapshot, next_generation) = latest.read().expect("second snapshot");
        assert_eq!(snapshot.tid, 8);
        assert!(next_generation > generation);
    }

    #[test]
    fn latest_snapshot_double_buffer_stays_consistent_under_contention() {
        use std::sync::Arc;

        let latest = Arc::new(LatestSnapshots::new());
        let done = Arc::new(AtomicBool::new(false));
        let writer_latest = Arc::clone(&latest);
        let writer_done = Arc::clone(&done);
        let writer = std::thread::spawn(move || {
            for value in 1..=10_000usize {
                writer_latest.try_write_with(|output| {
                    *output = StackSnapshot::from_parts(
                        value as u64,
                        StackSource::Vm,
                        &[],
                        &[value],
                        StackFlags::empty(),
                    );
                    true
                });
            }
            writer_done.store(true, Ordering::Release);
        });

        while !done.load(Ordering::Acquire) {
            if let Some((snapshot, _)) = latest.read() {
                assert_eq!(snapshot.py[0] as u64, snapshot.tid);
            }
        }
        writer.join().expect("writer");
    }

    #[test]
    fn latest_snapshot_reset_recovers_inherited_busy_state() {
        let latest = LatestSnapshots::new();
        latest.writer.store(true, Ordering::Relaxed);
        latest.readers[0].store(1, Ordering::Relaxed);
        latest.readers[1].store(2, Ordering::Relaxed);
        latest.published.store(3, Ordering::Relaxed);

        latest.reset_after_fork();

        assert!(!latest.writer.load(Ordering::Relaxed));
        assert_eq!(latest.readers[0].load(Ordering::Relaxed), 0);
        assert_eq!(latest.readers[1].load(Ordering::Relaxed), 0);
        assert_eq!(latest.published.load(Ordering::Relaxed), NO_LATEST);
        assert!(latest.try_write_with(|output| {
            *output = sample_with_tid(9);
            true
        }));
        assert_eq!(latest.read().expect("snapshot after reset").0.tid, 9);
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

    #[test]
    fn published_stack_remains_owned_by_registry_after_thread_exit() {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let stack = register_python_thread_slot().expect("registry slot");
            stack.enter(1, 0xCAFE);
            tx.send(current_tid()).expect("send tid");
        });
        let tid = rx.recv().expect("receive tid");
        thread.join().expect("registered thread");

        let snapshot = copy_registered_py_snapshot(tid).expect("global published stack");
        assert_eq!(snapshot.py_len, 1);
        assert_eq!(snapshot.py[0], 0xCAFE);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mach_suspend_captures_other_thread_native_stack_and_resumes_it() {
        use std::sync::Arc;

        let running = Arc::new(AtomicBool::new(true));
        let worker_running = Arc::clone(&running);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            register_python_thread();
            tx.send(current_tid()).expect("send tid");
            while worker_running.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
        });
        let tid = rx.recv().expect("receive tid");

        let snapshot = capture_thread_snapshot_suspended(tid);
        running.store(false, Ordering::Release);
        worker.join().expect("worker resumes after capture");

        let snapshot = snapshot.expect("Mach suspended capture");
        assert_eq!(snapshot.tid, tid);
        assert_eq!(snapshot.source, StackSource::MachSuspend);
        assert!(snapshot.native_len >= 1);
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

    #[cfg(unix)]
    #[test]
    fn delayed_sigusr2_for_old_target_cannot_publish() {
        with_signal_test_lock(|| {
            install_sigusr2_handler();
            register_python_thread();
            let seq_before = sigusr2_capture_generation();
            let _guard = Sigusr2ArmGuard::new(current_tid().wrapping_add(1));

            unsafe {
                libc::raise(libc::SIGUSR2);
            }

            assert_eq!(sigusr2_capture_generation(), seq_before);
            assert!(take_sigusr2_snapshot(seq_before).is_none());
        });
    }
}
