//! CPU profiling via `SIGPROF` sampling ("model two": trigger in-signal, process
//! off-signal).
//!
//! `setitimer(ITIMER_PROF)` delivers `SIGPROF` to whichever thread is burning
//! CPU (the timer counts process CPU time, so samples are CPU-weighted). The
//! signal handler is kept strictly minimal and async-signal-safe: it only
//!   * reads the interrupted thread's `pc`/`fp` from the `ucontext`,
//!   * walks the frame-pointer chain collecting raw return addresses,
//!   * snapshots this thread's thread-local `PYSTACKS` (raw `usize` pointers),
//!   * and copies that fixed-size POD snapshot into a preallocated lock-free ring
//!     buffer. No allocation, no locks, no symbolization, no `libunwind`, no libc
//!     string calls happen in the handler — that is what crashed the old `pprof`
//!     path on PyTorch (libunwind + `strlen` over Accelerate/JIT frames).
//!
//! A dedicated consumer thread drains the ring and does all the dangerous work
//! off the signal path: symbolizing native addresses (`backtrace::resolve`) and
//! resolving Python `RawCallLocation`s into file/func/line. It then produces a
//! true mixed-mode stack by splicing each Python frame into its interpreter eval
//! frame (`_PyEval_EvalFrameDefault`) position in the native stack — the eval
//! frames and `PYSTACKS` come from the same eval-hook calls, so they align
//! one-to-one. Folded counts are rendered as interactive HTML by `flamegraph()`.
//!
//! Native caveat (accepted tradeoff): frame-pointer walking is unreliable when
//! libraries omit frame pointers (BLAS/OpenMP); such stacks are truncated, and a
//! `fp` that is plausible-but-unmapped can still fault inside the handler. The
//! walk validates alignment / monotonicity / canonical range to minimize this.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::{compiler_fence, AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use core::ffi::{c_int, c_void};
use nix::libc;
use once_cell::sync::Lazy;
use serde_json::json;

use crate::features::flamegraph::{FlamegraphKind, FlamegraphOptions};
use crate::features::spy::call::RawCallLocation;
use crate::features::spy::spy_tls_addrs;

const DEFAULT_SAMPLE_FREQ: i32 = 100;
const MIN_SAMPLE_FREQ: i32 = 1;
// Upper bound is intentionally high to allow stress testing the sampler.
const MAX_SAMPLE_FREQ: i32 = 100_000;

/// Ring capacity (power of two) and per-sample depth limits. Kept small enough
/// that a `RawSample` (and the whole ring) stays cheap to construct/copy.
const RING_SIZE: usize = 512;
const RING_MASK: usize = RING_SIZE - 1;
const MAX_NATIVE: usize = 48;
const MAX_PY: usize = 128;

/// Max number of Python threads we track for signal-safe TLS access.
const REG_SIZE: usize = 1024;

/// Upper bound on distinct folded stacks kept in the aggregate map. Protects
/// against unbounded memory under high-frequency / high-cardinality workloads;
/// samples for new stacks beyond this are counted as dropped.
const MAX_FOLDED_STACKS: usize = 1 << 17;

// ---------------------------------------------------------------------------
// Raw sample (fixed-size POD, memcpy-able from the signal handler)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct RawSample {
    tid: u64,
    native_len: u32,
    py_len: u32,
    /// Native return addresses, leaf -> root.
    native: [usize; MAX_NATIVE],
    /// Callee `PyCodeObject` pointers, outermost -> innermost (natural `PYSTACKS`
    /// order). Only the pointer is captured here — it is the interner key the
    /// consumer resolves to a label — keeping the POD sample small and cheap to
    /// memcpy from the signal handler.
    py: [usize; MAX_PY],
}

impl RawSample {
    fn zeroed() -> Self {
        RawSample {
            tid: 0,
            native_len: 0,
            py_len: 0,
            native: [0usize; MAX_NATIVE],
            py: [0usize; MAX_PY],
        }
    }
}

// ---------------------------------------------------------------------------
// Lock-free bounded MPMC ring (Vyukov). Async-signal-safe producer side.
// ---------------------------------------------------------------------------

struct Cell {
    seq: AtomicUsize,
    data: UnsafeCell<RawSample>,
}

struct Ring {
    buffer: Box<[Cell]>,
    enqueue_pos: AtomicUsize,
    dequeue_pos: AtomicUsize,
}

unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    fn new() -> Ring {
        let mut v: Vec<Cell> = Vec::with_capacity(RING_SIZE);
        for i in 0..RING_SIZE {
            v.push(Cell {
                seq: AtomicUsize::new(i),
                data: UnsafeCell::new(RawSample::zeroed()),
            });
        }
        Ring {
            buffer: v.into_boxed_slice(),
            enqueue_pos: AtomicUsize::new(0),
            dequeue_pos: AtomicUsize::new(0),
        }
    }

    /// Producer (signal handler). Returns false if the ring is full.
    fn enqueue(&self, sample: &RawSample) -> bool {
        let mut pos = self.enqueue_pos.load(Ordering::Relaxed);
        loop {
            let cell = &self.buffer[pos & RING_MASK];
            let seq = cell.seq.load(Ordering::Acquire);
            let diff = seq as isize - pos as isize;
            if diff == 0 {
                if self
                    .enqueue_pos
                    .compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    unsafe { *cell.data.get() = *sample };
                    cell.seq.store(pos + 1, Ordering::Release);
                    return true;
                }
            } else if diff < 0 {
                return false;
            } else {
                pos = self.enqueue_pos.load(Ordering::Relaxed);
            }
        }
    }

    /// Consumer (sampler thread). Returns false if the ring is empty.
    fn dequeue(&self, out: &mut RawSample) -> bool {
        let mut pos = self.dequeue_pos.load(Ordering::Relaxed);
        loop {
            let cell = &self.buffer[pos & RING_MASK];
            let seq = cell.seq.load(Ordering::Acquire);
            let diff = seq as isize - (pos + 1) as isize;
            if diff == 0 {
                if self
                    .dequeue_pos
                    .compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    unsafe { *out = *cell.data.get() };
                    cell.seq.store(pos + RING_SIZE, Ordering::Release);
                    return true;
                }
            } else if diff < 0 {
                return false;
            } else {
                pos = self.dequeue_pos.load(Ordering::Relaxed);
            }
        }
    }
}

static RING_PTR: AtomicPtr<Ring> = AtomicPtr::new(std::ptr::null_mut());
static DROPPED: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Python-thread registry: lets the handler know a thread's `PYSTACKS` TLS is
// already allocated (touched by `rust_eval_frame`) and thus safe to read.
// ---------------------------------------------------------------------------

/// Per-thread registry entry. We store the *resolved* addresses of the thread's
/// thread-local `PYSTACKS` / `PYSTACK_WRITING` here, captured from normal
/// (non-signal) context in the eval hook. The signal handler then reads those
/// raw pointers directly and NEVER touches TLS / `tlv_get_addr` itself — on
/// macOS, accessing a thread-local from a signal handler can deadlock against
/// dyld's loader lock when the interrupted thread was mid dlopen / lazy-bind /
/// TLS initialization (the classic in-process sampler hang).
struct ThreadSlot {
    tid: AtomicU64, // 0 == empty
    pystacks: AtomicPtr<Vec<RawCallLocation>>,
    writing: AtomicPtr<bool>,
    /// Inclusive-exclusive stack bounds `[lo, hi)` captured in normal context,
    /// so the handler can validate frame-pointer reads against real mapped
    /// stack memory. `hi == 0` means "unknown" (fall back to heuristics).
    stack_lo: AtomicUsize,
    stack_hi: AtomicUsize,
}

static REG_TABLE: [ThreadSlot; REG_SIZE] = [const {
    ThreadSlot {
        tid: AtomicU64::new(0),
        pystacks: AtomicPtr::new(std::ptr::null_mut()),
        writing: AtomicPtr::new(std::ptr::null_mut()),
        stack_lo: AtomicUsize::new(0),
        stack_hi: AtomicUsize::new(0),
    }
}; REG_SIZE];

static REG_FULL_WARNED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static THREAD_REGISTERED: std::cell::UnsafeCell<bool> = const { std::cell::UnsafeCell::new(false) };
}

/// Open-addressing probe start for `tid` (Fibonacci hashing). REG_SIZE is a
/// power of two so the mask is exact.
#[inline]
fn slot_hash(tid: u64) -> usize {
    let h = tid.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> 40) as usize & (REG_SIZE - 1)
}

/// OS-thread names captured at registration (normal context), keyed by tid, so
/// the consumer can label thread root frames readably.
static THREAD_NAMES: Lazy<RwLock<HashMap<u64, String>>> = Lazy::new(|| RwLock::new(HashMap::new()));

/// Read the calling thread's name via `pthread_getname_np`. Normal context only.
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

fn thread_name(tid: u64) -> Option<String> {
    THREAD_NAMES.read().ok().and_then(|m| m.get(&tid).cloned())
}

/// Stack bounds `[lo, hi)` of the calling thread. Safe to call only in normal
/// context (uses `pthread_*` that may allocate / read `/proc` on Linux).
fn current_stack_bounds() -> (usize, usize) {
    #[cfg(target_os = "macos")]
    unsafe {
        let pt = libc::pthread_self();
        let base = libc::pthread_get_stackaddr_np(pt) as usize; // highest address
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

/// Called from the eval-frame hook (TLS-safe context) on every frame; the
/// thread-local fast path makes the global table insert happen once per thread.
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
    // Resolve this thread's TLS addresses and stack bounds here, in normal
    // context (none of this is async-signal-safe).
    let (ps, wr) = spy_tls_addrs();
    let (lo, hi) = current_stack_bounds();

    if let Some(name) = current_thread_name() {
        if let Ok(mut m) = THREAD_NAMES.write() {
            m.insert(tid, name); // overwrite handles tid reuse
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
            // Refresh: handles tid reuse after a previous thread with the same
            // id exited (its TLS pointers / bounds are now stale).
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
        // Slot taken by another tid (lost the race or different thread): keep
        // probing the same linear sequence.
    }

    if !REG_FULL_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "probing: pprof thread registry full ({REG_SIZE} threads); \
             Python stacks for further threads will be missing"
        );
    }
}

/// Find the registry slot for `tid`, or `None` if this thread never ran the eval
/// hook. Open-addressing lookup: an empty slot in the probe sequence proves the
/// tid is absent (entries are never deleted).
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

// ---------------------------------------------------------------------------
// Signal handler (async-signal-safe)
// ---------------------------------------------------------------------------

static SAMPLER_ENABLED: AtomicBool = AtomicBool::new(false);

/// Number of signal handlers currently executing past the enabled/ring checks.
/// `reset` uses this to quiesce before freeing the ring.
static HANDLER_ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// Whether *we* enabled the eval tracer (vs. the user enabling it independently);
/// if so, `reset` disables it again.
static PPROF_OWNS_TRACER: AtomicBool = AtomicBool::new(false);

/// RAII guard that marks a handler as in-flight for the duration of its body,
/// covering all early-return paths.
struct ActiveGuard;
impl ActiveGuard {
    #[inline]
    fn new() -> Self {
        HANDLER_ACTIVE.fetch_add(1, Ordering::Acquire);
        ActiveGuard
    }
}
impl Drop for ActiveGuard {
    #[inline]
    fn drop(&mut self) {
        HANDLER_ACTIVE.fetch_sub(1, Ordering::Release);
    }
}

#[inline]
fn current_tid() -> u64 {
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

#[inline]
fn plausible(p: usize) -> bool {
    // Canonical lower-half userspace pointer, above the first page.
    (0x1000..0x0001_0000_0000_0000).contains(&p)
}

/// Extract (pc, fp) from the signal `ucontext` for the interrupted thread.
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

/// Walk the frame-pointer chain, collecting return addresses leaf -> root.
/// Both x86_64 and aarch64 use the layout `[fp] = saved fp`, `[fp+8] = ret`.
///
/// `[lo, hi)` are the thread's stack bounds when known (`hi != 0`); reads are
/// then proven to stay inside mapped stack memory, eliminating the residual
/// "plausible but unmapped `fp`" fault. When bounds are unknown we fall back to
/// alignment / monotonicity / canonical-range heuristics only.
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
        // Frame pointers must strictly increase by a bounded step.
        if saved_fp <= fp || saved_fp - fp > 0x20_0000 {
            break;
        }
        fp = saved_fp;
    }
    count
}

unsafe extern "C" fn sigprof_handler(_sig: c_int, _info: *mut libc::siginfo_t, uctx: *mut c_void) {
    if !SAMPLER_ENABLED.load(Ordering::Acquire) {
        return;
    }
    // Mark in-flight before touching the ring so `reset` can quiesce safely.
    let _active = ActiveGuard::new();
    let ring = RING_PTR.load(Ordering::Acquire);
    if ring.is_null() {
        return;
    }
    let ring = &*ring;

    let mut sample = RawSample::zeroed();
    sample.tid = current_tid();

    // Resolve the registry slot once: its stack bounds guard the native walk and
    // its TLS pointers feed the Python snapshot.
    let slot = thread_slot(sample.tid);
    let (lo, hi) = match slot {
        Some(s) => (
            s.stack_lo.load(Ordering::Acquire),
            s.stack_hi.load(Ordering::Acquire),
        ),
        None => (0, 0),
    };

    // ---- native ----
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

    // ---- python: read this thread's PYSTACKS through the pre-resolved raw
    // pointers in the registry, never touching TLS / tlv_get_addr from here ----
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
            // If the eval hook touched PYSTACKS during the copy, discard it.
            sample.py_len = if *wr { 0 } else { n as u32 };
        }
    }

    if sample.native_len == 0 && sample.py_len == 0 {
        return;
    }
    if !ring.enqueue(&sample) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Timer + handler installation
// ---------------------------------------------------------------------------

static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

fn install_handler() {
    if HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        let handler_ptr = sigprof_handler as *const () as usize;
        sa.sa_sigaction = handler_ptr;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGPROF, &sa, std::ptr::null_mut());
    }
}

fn arm_timer(freq: i32) {
    let period_us = (1_000_000i64 / freq as i64).max(1);
    let tv = libc::timeval {
        tv_sec: (period_us / 1_000_000) as libc::time_t,
        tv_usec: (period_us % 1_000_000) as libc::suseconds_t,
    };
    let it = libc::itimerval {
        it_interval: tv,
        it_value: tv,
    };
    unsafe { libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut()) };
}

fn disarm_timer() {
    let it: libc::itimerval = unsafe { std::mem::zeroed() };
    unsafe { libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut()) };
}

// ---------------------------------------------------------------------------
// Consumer thread: symbolize + fold (all off the signal path)
// ---------------------------------------------------------------------------

struct SamplerState {
    generation: AtomicU64,
    samples: Mutex<HashMap<String, u64>>,
}

static SAMPLER: Lazy<SamplerState> = Lazy::new(|| SamplerState {
    generation: AtomicU64::new(0),
    samples: Mutex::new(HashMap::new()),
});

fn symbolize_native(addr: usize, cache: &mut HashMap<usize, String>) -> String {
    if let Some(name) = cache.get(&addr) {
        return name.clone();
    }
    let mut resolved: Option<String> = None;
    backtrace::resolve(addr as *mut c_void, |sym| {
        if resolved.is_none() {
            if let Some(name) = sym.name() {
                // `SymbolName`'s Display demangles Rust and C++ names.
                resolved = Some(name.to_string());
            }
        }
    });
    let name = resolved.unwrap_or_else(|| format!("0x{addr:x}"));
    cache.insert(addr, name.clone());
    name
}

/// Interner mapping a callee `PyCodeObject` pointer to its formatted frame label.
///
/// Entries are produced eagerly in the eval hook (`intern_py_frame`), where the
/// code object is alive under the GIL. The sampler consumer thread only ever
/// *looks up* labels here by integer pointer key — it never dereferences a
/// Python object — so a frame whose code object has since been freed (e.g.
/// `torch.compile` churn) can no longer cause a use-after-free.
static PY_SYMBOLS: Lazy<RwLock<HashMap<usize, String>>> = Lazy::new(|| RwLock::new(HashMap::new()));

/// Soft cap so heavy dynamic-codegen workloads can't grow the interner forever.
const PY_SYMBOLS_CAP: usize = 1 << 18;

/// Resolve and cache a Python frame's label. MUST be called from a normal
/// (GIL-holding) context such as the eval hook, where `loc.callee()` is alive.
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
    let label = match loc.resolve_callee() {
        Ok(sym) => {
            let base = sym.file.rsplit(['/', '\\']).next().unwrap_or(&sym.file);
            format!("[py] {} ({}:{})", sym.name, base, sym.line)
        }
        Err(_) => return,
    };
    if let Ok(mut g) = PY_SYMBOLS.write() {
        if g.len() < PY_SYMBOLS_CAP {
            g.entry(key).or_insert(label);
        }
    }
}

/// Drop all interned Python labels (called when the eval tracer is disabled).
pub fn clear_py_symbols() {
    if let Ok(mut g) = PY_SYMBOLS.write() {
        g.clear();
        g.shrink_to_fit();
    }
}

/// Look up a sampled Python frame's label by callee pointer. Pure integer-keyed
/// lookup — never touches Python memory, so it is safe on the consumer thread.
fn resolve_py_frame(key: usize) -> String {
    if key != 0 {
        if let Ok(g) = PY_SYMBOLS.read() {
            if let Some(label) = g.get(&key) {
                return label.clone();
            }
        }
    }
    "[py] <unknown>".to_string()
}

/// A CPython interpreter eval frame (`_PyEval_EvalFrameDefault` / `EvalFrameEx`,
/// with any C-ABI leading underscores). These are the splice points where a
/// Python frame replaces the native frame. Matches `stack_tracer::merge_strategy`.
#[inline]
fn is_eval_frame(name: &str) -> bool {
    let mut tokens = name.split(['_', '.']).filter(|s| !s.is_empty());
    matches!(tokens.next(), Some("PyEval"))
        && matches!(
            tokens.next(),
            Some("EvalFrameDefault") | Some("EvalFrameEx")
        )
}

/// Our own eval-frame hook trampoline; dropped from mixed stacks as noise.
#[inline]
fn is_interp_shim(name: &str) -> bool {
    name.contains("rust_eval_frame")
}

fn process_sample(s: &RawSample, cache: &mut HashMap<usize, String>) {
    let nlen = s.native_len as usize;
    let plen = s.py_len as usize;

    // Native symbols, leaf -> root. Frame 0 is the interrupted PC (exact); every
    // deeper frame is a *return address* pointing just past the call site, so we
    // symbolize `addr - 1` to get the correct line / inlined-function.
    let native_l2r: Vec<String> = (0..nlen)
        .map(|i| {
            let resolve_addr = if i == 0 {
                s.native[i]
            } else {
                s.native[i].wrapping_sub(1)
            };
            symbolize_native(resolve_addr, cache)
        })
        .collect();
    // Python frames, innermost -> outermost (PYSTACKS stores outermost -> innermost).
    let py_l2r: Vec<String> = s.py[..plen]
        .iter()
        .rev()
        .map(|&key| resolve_py_frame(key))
        .collect();

    let eval_count = native_l2r.iter().filter(|n| is_eval_frame(n)).count();

    // Build leaf -> root, reversed to root -> leaf at the end.
    let mut combined: Vec<String> = Vec::with_capacity(native_l2r.len() + py_l2r.len());

    if eval_count > 0 && !py_l2r.is_empty() {
        // True mixed mode: walking leaf -> root, replace each interpreter eval
        // frame with the corresponding Python frame. Aligning from the leaf
        // (deepest eval <-> innermost Python) keeps attribution correct even
        // when a truncated fp-walk drops the outermost eval frames.
        let mut pi = 0usize;
        for n in &native_l2r {
            if is_eval_frame(n) {
                combined.push(py_l2r.get(pi).cloned().unwrap_or_else(|| n.clone()));
                pi += 1;
            } else if is_interp_shim(n) {
                // drop our eval hook trampoline frame
            } else {
                combined.push(n.clone());
            }
        }
        // Outer Python frames whose eval frames were lost to truncation: keep
        // them toward the root so the logical context is not dropped.
        if pi < py_l2r.len() {
            combined.extend_from_slice(&py_l2r[pi..]);
        }
    } else if !native_l2r.is_empty() {
        // No interpreter frames recovered (e.g. truncated walk or pure native):
        // native tower with any Python frames hanging off the leaf.
        combined.extend(py_l2r);
        combined.extend(native_l2r.into_iter().filter(|n| !is_interp_shim(n)));
    } else {
        // Pure Python.
        combined.extend(py_l2r);
    }

    if combined.is_empty() {
        return;
    }
    combined.reverse(); // root -> leaf

    let mut line = match thread_name(s.tid) {
        Some(name) => format!("thread-{} ({})", s.tid, name),
        None => format!("thread-{}", s.tid),
    };
    for p in combined {
        line.push(';');
        line.push_str(&p);
    }

    if let Ok(mut map) = SAMPLER.samples.lock() {
        if let Some(count) = map.get_mut(&line) {
            *count += 1;
        } else if map.len() < MAX_FOLDED_STACKS {
            map.insert(line, 1);
        } else {
            // Cardinality cap hit: account the sample as dropped rather than
            // grow the map without bound.
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn consumer_loop(my_gen: u64) {
    let mut sample = RawSample::zeroed();
    let mut cache: HashMap<usize, String> = HashMap::new();
    loop {
        let stopping = SAMPLER.generation.load(Ordering::SeqCst) != my_gen;
        let ring = RING_PTR.load(Ordering::Acquire);
        let mut drained = false;
        if !ring.is_null() {
            let ring = unsafe { &*ring };
            while ring.dequeue(&mut sample) {
                drained = true;
                process_sample(&sample, &mut cache);
            }
        }
        if stopping {
            break;
        }
        if !drained {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

// ---------------------------------------------------------------------------
// Public API (stable for existing callers)
// ---------------------------------------------------------------------------

pub fn is_sampling_active() -> bool {
    SAMPLER_ENABLED.load(Ordering::Acquire)
}

pub fn setup(freq: u64) -> Result<()> {
    let freq = if freq == 0 {
        DEFAULT_SAMPLE_FREQ
    } else {
        (freq as i32).clamp(MIN_SAMPLE_FREQ, MAX_SAMPLE_FREQ)
    };

    // Allocate the ring once.
    if RING_PTR.load(Ordering::Acquire).is_null() {
        let ptr = Box::into_raw(Box::new(Ring::new()));
        if RING_PTR
            .compare_exchange(
                std::ptr::null_mut(),
                ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            unsafe { drop(Box::from_raw(ptr)) };
        }
    }

    if let Ok(mut map) = SAMPLER.samples.lock() {
        map.clear();
    }
    DROPPED.store(0, Ordering::Relaxed);

    // The sampler's Python frames come from the eval-frame hook; ensure it is on
    // (idempotent) so we don't silently degrade to native-only stacks when the
    // user enables pprof without separately enabling the tracer. Remember whether
    // *we* turned it on, so `reset` only retires a tracer it owns.
    crate::features::vm_tracer::initialize_globals();
    pyo3::Python::attach(|_py| {
        let already_on = crate::features::vm_tracer::is_tracer_enabled();
        match crate::features::vm_tracer::enable_tracer() {
            Ok(()) => {
                if !already_on {
                    PPROF_OWNS_TRACER.store(true, Ordering::Release);
                }
            }
            Err(e) => log::warn!(
                "probing: pprof could not enable the Python eval tracer ({e}); \
                 stacks will be native-only"
            ),
        }
    });

    install_handler();

    let my_gen = SAMPLER.generation.fetch_add(1, Ordering::SeqCst) + 1;
    SAMPLER_ENABLED.store(true, Ordering::Release);

    thread::Builder::new()
        .name("probing-sampler".into())
        .spawn(move || consumer_loop(my_gen))
        .context("failed to spawn sampler consumer thread")?;

    arm_timer(freq);
    log::info!("probing: SIGPROF CPU sampler started ({freq} Hz, Python+native)");
    eprintln!("probing: SIGPROF CPU sampler started ({freq} Hz)");
    Ok(())
}

pub fn reset() {
    disarm_timer();
    SAMPLER_ENABLED.store(false, Ordering::Release);
    SAMPLER.generation.fetch_add(1, Ordering::SeqCst);

    // Take the ring out of circulation, wait for any in-flight handler to drain,
    // then free it. If we can't confirm quiescence (practically never, the
    // handler is bounded), put it back rather than risk a use-after-free.
    let ring = RING_PTR.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !ring.is_null() {
        let mut drained = false;
        for _ in 0..10_000_000 {
            if HANDLER_ACTIVE.load(Ordering::Acquire) == 0 {
                drained = true;
                break;
            }
            std::hint::spin_loop();
        }
        if drained {
            unsafe { drop(Box::from_raw(ring)) };
        } else {
            RING_PTR.store(ring, Ordering::Release);
        }
    }

    // Retire the eval tracer only if pprof was the one that enabled it.
    if PPROF_OWNS_TRACER.swap(false, Ordering::AcqRel) {
        pyo3::Python::attach(|_py| {
            let _ = crate::features::vm_tracer::disable_tracer();
        });
    }

    clear_py_symbols();
    if let Ok(mut m) = THREAD_NAMES.write() {
        m.clear();
    }
}

pub fn pprof_handler() {
    let _ = setup(DEFAULT_SAMPLE_FREQ as u64);
}

fn pprof_flamegraph_options() -> FlamegraphOptions {
    FlamegraphOptions {
        title: "CPU sampling".to_string(),
        count_name: "samples".to_string(),
        kind: FlamegraphKind::TorchModule,
        subtitle: "SIGPROF weighted stack samples".to_string(),
        metric: None,
    }
}

/// Snapshot the aggregate map into folded `"stack count"` lines under a single
/// lock acquisition (no double-fold / TOCTOU between HTML and JSON paths).
fn folded_lines() -> Vec<String> {
    match SAMPLER.samples.lock() {
        Ok(map) => map
            .iter()
            .map(|(stack, count)| format!("{stack} {count}"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn flamegraph() -> Result<String> {
    let lines = folded_lines();
    if lines.is_empty() {
        return Err(anyhow!(
            "no samples collected yet; enable CPU sampling and let it run"
        ));
    }

    let dropped = DROPPED.load(Ordering::Relaxed);
    if dropped > 0 {
        log::warn!("probing: {dropped} CPU samples dropped (ring full or cardinality cap)");
    }

    let fg = crate::features::flamegraph::Flamegraph::from_folded_lines(&lines)
        .ok_or_else(|| anyhow!("no valid folded stacks"))?;
    Ok(fg.render_html(&pprof_flamegraph_options()))
}

/// JSON payload for the web UI (`GET /apis/pprofextension/flamegraph/json`).
pub fn flamegraph_json() -> String {
    let dropped = DROPPED.load(Ordering::Relaxed);
    let empty = |msg: String| {
        json!({
            "profile": "cpu-stack",
            "title": "CPU sampling",
            "subtitle": "SIGPROF weighted stack samples",
            "countName": "samples",
            "total": 0,
            "width": 1400.0,
            "frameHeight": 32.0,
            "frames": [],
            "dropped": dropped,
            "emptyMessage": msg,
        })
        .to_string()
    };

    let lines = folded_lines();
    if lines.is_empty() {
        return empty("no samples collected yet; enable CPU sampling and let it run".to_string());
    }

    match crate::features::flamegraph::Flamegraph::from_folded_lines(&lines) {
        Some(fg) => {
            let payload = fg.json_payload(&pprof_flamegraph_options());
            // Splice in the dropped-sample counter so the UI can warn on it.
            match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(mut v) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("dropped".to_string(), json!(dropped));
                    }
                    v.to_string()
                }
                Err(_) => payload,
            }
        }
        None => empty("no valid folded stacks".to_string()),
    }
}
