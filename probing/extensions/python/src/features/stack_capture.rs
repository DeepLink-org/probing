//! Async-signal-safe stack snapshotting shared by SIGPROF and SIGUSR2.
//!
//! Signal handlers only copy raw PCs and eval-hook frame keys into a fixed POD
//! struct. Symbolization and Python/native merge happen off the signal path via
//! [`snapshot_to_merged_frames`].

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::{compiler_fence, AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::Duration;

use core::ffi::{c_int, c_void};
use nix::libc;
use once_cell::sync::Lazy;
use probing_proto::prelude::CallFrame;

use crate::features::spy::call::RawCallLocation;
use crate::features::spy::spy_tls_addrs;
use crate::features::stack_merge::{demangle_native_symbol, merge_python_native_stacks};

pub const MAX_NATIVE: usize = 48;
pub const MAX_PY: usize = 128;
const REG_SIZE: usize = 1024;

/// Fixed-size POD snapshot copied from signal handlers.
#[derive(Clone, Copy)]
pub struct RawStackSnapshot {
    pub tid: u64,
    pub native_len: u32,
    pub py_len: u32,
    /// Native return addresses, leaf -> root.
    pub native: [usize; MAX_NATIVE],
    /// Callee `PyCodeObject` pointers, outermost -> innermost (`PYSTACKS` order).
    pub py: [usize; MAX_PY],
}

impl RawStackSnapshot {
    pub fn zeroed() -> Self {
        RawStackSnapshot {
            tid: 0,
            native_len: 0,
            py_len: 0,
            native: [0usize; MAX_NATIVE],
            py: [0usize; MAX_PY],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.native_len == 0 && self.py_len == 0
    }
}

// ---------------------------------------------------------------------------
// Python-thread registry (TLS pointers resolved in normal context)
// ---------------------------------------------------------------------------

struct ThreadSlot {
    tid: AtomicU64,
    pystacks: AtomicPtr<Vec<RawCallLocation>>,
    writing: AtomicPtr<bool>,
    stack_lo: AtomicUsize,
    stack_hi: AtomicUsize,
    latest: UnsafeCell<RawStackSnapshot>,
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
        latest: UnsafeCell::new(RawStackSnapshot {
            tid: 0,
            native_len: 0,
            py_len: 0,
            native: [0usize; MAX_NATIVE],
            py: [0usize; MAX_PY],
        }),
        latest_seq: AtomicU64::new(0),
    }
}; REG_SIZE];

static REG_FULL_WARNED: AtomicBool = AtomicBool::new(false);
static MAIN_OS_TID: AtomicU64 = AtomicU64::new(0);
static PPROF_SAMPLING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Record the Python main thread's OS tid (pthread id on macOS, gettid on Linux).
pub fn register_main_os_tid() {
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
pub fn copy_registered_py_snapshot(tid: u64) -> Option<RawStackSnapshot> {
    let slot = thread_slot(tid)?;
    let mut sample = RawStackSnapshot::zeroed();
    sample.tid = tid;

    let wr = slot.writing.load(Ordering::Acquire);
    let ps = slot.pystacks.load(Ordering::Acquire);
    if wr.is_null() || ps.is_null() || unsafe { *wr } {
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
        return None;
    }
    sample.py_len = n as u32;
    if sample.is_empty() {
        None
    } else {
        Some(sample)
    }
}

/// Store the latest SIGPROF snapshot for a thread so on-demand SIGUSR2 capture
/// can reuse it without delivering another signal.
pub fn store_latest_snapshot(snapshot: &RawStackSnapshot) {
    if snapshot.is_empty() {
        return;
    }
    let Some(slot) = thread_slot(snapshot.tid) else {
        return;
    };
    unsafe {
        *slot.latest.get() = *snapshot;
    }
    slot.latest_seq.fetch_add(1, Ordering::Release);
}

/// Reuse the latest SIGPROF snapshot for `tid` when CPU sampling is active.
pub fn latest_snapshot_for_tid(tid: u64) -> Option<RawStackSnapshot> {
    let slot = thread_slot(tid)?;
    for _ in 0..4 {
        let seq_before = slot.latest_seq.load(Ordering::Acquire);
        if seq_before == 0 {
            return None;
        }
        let snap = unsafe { *slot.latest.get() };
        let seq_after = slot.latest_seq.load(Ordering::Acquire);
        if seq_before == seq_after && snap.tid == tid && !snap.is_empty() {
            return Some(snap);
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

fn resolve_py_label(key: usize) -> String {
    if key != 0 {
        if let Ok(g) = PY_SYMBOLS.read() {
            if let Some(sym) = g.get(&key) {
                return sym.folded_label();
            }
        }
    }
    "[py] <unknown>".to_string()
}

fn resolve_py_call_frame(key: usize) -> CallFrame {
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

#[inline]
fn plausible(p: usize) -> bool {
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
        (pc, fp)
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        let uc = uctx as *const libc::ucontext_t;
        let mc = &(*uc).uc_mcontext;
        (mc.pc as usize, mc.regs[29] as usize)
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        let uc = uctx as *const libc::ucontext_t;
        let ss = &(*(*uc).uc_mcontext).__ss;
        (ss.__rip as usize, ss.__rbp as usize)
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let uc = uctx as *const libc::ucontext_t;
        let ss = &(*(*uc).uc_mcontext).__ss;
        (ss.__pc as usize, ss.__fp as usize)
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
    let in_stack =
        |fp: usize| !bounded || (fp >= lo && fp + 2 * std::mem::size_of::<usize>() <= hi);

    let mut fp = start_fp;
    let mut count = 0usize;
    while count < out.len() {
        if !plausible(fp) || (fp & 0x7) != 0 || !in_stack(fp) {
            break;
        }
        let saved_fp = *(fp as *const usize);
        let ret = *((fp + std::mem::size_of::<usize>()) as *const usize);
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

/// Async-signal-safe snapshot of the interrupted thread's native + Python stacks.
///
/// # Safety
///
/// `uctx` must be a valid `ucontext_t` pointer from a signal handler (or null,
/// in which case native registers are skipped). Must only be called from an
/// async-signal-safe context on the interrupted thread; the returned snapshot
/// must not be shared with concurrent writers without synchronization.
pub unsafe fn capture_raw_snapshot(uctx: *mut c_void) -> RawStackSnapshot {
    let mut sample = RawStackSnapshot::zeroed();
    sample.tid = current_tid();

    let slot = thread_slot(sample.tid);
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
        sample.native[nlen] = pc;
        nlen += 1;
    }
    if nlen < MAX_NATIVE {
        nlen += walk_frame_pointers(fp, &mut sample.native[nlen..], lo, hi);
    }
    sample.native_len = nlen as u32;

    if let Some(slot) = slot {
        let wr = slot.writing.load(Ordering::Acquire);
        let ps = slot.pystacks.load(Ordering::Acquire);
        if !wr.is_null() && !ps.is_null() && !*wr {
            compiler_fence(Ordering::SeqCst);
            let stacks = &*ps;
            let n = stacks.len().min(MAX_PY);
            for (i, stack) in stacks.iter().enumerate().take(n) {
                sample.py[i] = stack.callee();
            }
            compiler_fence(Ordering::SeqCst);
            sample.py_len = if *wr { 0 } else { n as u32 };
        }
    }

    sample
}

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

/// Best-effort symbolize + merge off the signal path. Never propagates errors.
pub fn snapshot_to_merged_frames(
    snapshot: &RawStackSnapshot,
    cache: &mut HashMap<usize, CallFrame>,
) -> Vec<CallFrame> {
    let nlen = snapshot.native_len as usize;
    let plen = snapshot.py_len as usize;

    let native_leaf_to_root: Vec<CallFrame> = (0..nlen)
        .map(|i| {
            let resolve_addr = if i == 0 {
                snapshot.native[i]
            } else {
                snapshot.native[i].wrapping_sub(1)
            };
            symbolize_native_addr(resolve_addr, cache)
        })
        .collect();

    let python_outer_to_inner: Vec<CallFrame> = snapshot.py[..plen]
        .iter()
        .map(|&key| resolve_py_call_frame(key))
        .collect();

    merge_python_native_stacks(&python_outer_to_inner, &native_leaf_to_root)
}

pub fn snapshot_to_folded_line(
    snapshot: &RawStackSnapshot,
    cache: &mut HashMap<usize, CallFrame>,
) -> String {
    let merged = snapshot_to_merged_frames(snapshot, cache);
    if merged.is_empty() {
        return String::new();
    }
    let segments = crate::features::stack_merge::merged_frames_to_folded_segments(&merged);
    let mut line = match thread_name(snapshot.tid) {
        Some(name) => format!("thread-{} ({})", snapshot.tid, name),
        None => format!("thread-{}", snapshot.tid),
    };
    for seg in segments {
        line.push(';');
        line.push_str(&seg);
    }
    line
}

// ---------------------------------------------------------------------------
// SIGUSR2 on-demand capture (same safe handler body as SIGPROF)
// ---------------------------------------------------------------------------

struct Sigusr2SnapshotSlot(UnsafeCell<RawStackSnapshot>);
unsafe impl Sync for Sigusr2SnapshotSlot {}

const ZERO_SNAPSHOT: RawStackSnapshot = RawStackSnapshot {
    tid: 0,
    native_len: 0,
    py_len: 0,
    native: [0usize; MAX_NATIVE],
    py: [0usize; MAX_PY],
};

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

pub fn take_sigusr2_snapshot(after_seq: u64) -> Option<RawStackSnapshot> {
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
pub fn capture_thread_snapshot_signal(tid: u64, timeout: Duration) -> Option<RawStackSnapshot> {
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
            if snap.tid == tid {
                return Some(snap);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

#[cfg(not(unix))]
pub fn capture_thread_snapshot_signal(_tid: u64, _timeout: Duration) -> Option<RawStackSnapshot> {
    None
}

#[cfg(unix)]
pub fn install_sigusr2_handler() {
    if SIGUSR2_HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigusr2_stack_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
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
    let target = SIGUSR2_TARGET_TID.load(Ordering::Acquire);
    if target == 0 {
        return;
    }
    let snapshot = capture_raw_snapshot(uctx);
    if snapshot.is_empty() || snapshot.tid != target {
        return;
    }
    unsafe {
        *SIGUSR2_SNAPSHOT.0.get() = snapshot;
    }
    SIGUSR2_SEQ.fetch_add(1, Ordering::Release);
}

#[cfg(not(unix))]
pub fn install_sigusr2_handler() {}

#[cfg(test)]
mod tests {
    use super::*;

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
}
