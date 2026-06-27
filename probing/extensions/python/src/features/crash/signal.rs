//! Fatal-signal backtrace dumper.
//!
//! On `SIGSEGV` / `SIGBUS` / `SIGABRT` / `SIGILL` / `SIGFPE` we print the
//! crashing thread's native backtrace to stderr, then restore the default
//! disposition and re-raise the signal so the process still produces a core
//! dump / exits with the usual status. This is purely a debugging aid for
//! diagnosing the profiler/native crashes.
//!
//! The handler runs on a dedicated `sigaltstack` (so a stack-overflow crash can
//! still be reported) and avoids heap allocation on its hot path: integers are
//! formatted into stack buffers and symbol names are written as their raw bytes
//! via `write(2)`. Symbolization (`backtrace::*_unsynchronized`) is not strictly
//! async-signal-safe, but this only ever runs once, while the process is already
//! dying; a recursive fault is caught by a guard that immediately re-raises with
//! the default handler.

use core::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

use nix::libc;
use probing_core::trace::crash_atomic_step;

use super::context;
use super::handler;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_HANDLER: AtomicBool = AtomicBool::new(false);
static SIGNAL_SPILL_PATH_LEN: AtomicUsize = AtomicUsize::new(0);
static CACHED_RANK: AtomicI32 = AtomicI32::new(-1);
static CACHED_LOCAL_RANK: AtomicI32 = AtomicI32::new(-1);

const SIGNAL_SPILL_PATH_CAP: usize = 384;
static mut SIGNAL_SPILL_PATH: [u8; SIGNAL_SPILL_PATH_CAP] = [0u8; SIGNAL_SPILL_PATH_CAP];

const FATAL_SIGNALS: [c_int; 5] = [
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGABRT,
    libc::SIGILL,
    libc::SIGFPE,
];

const MAX_FRAMES: usize = 256;
const ALT_STACK_SIZE: usize = 256 * 1024;

static mut ALT_STACK: [u8; ALT_STACK_SIZE] = [0u8; ALT_STACK_SIZE];

fn sig_name(sig: c_int) -> &'static str {
    match sig {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGBUS => "SIGBUS",
        libc::SIGABRT => "SIGABRT",
        libc::SIGILL => "SIGILL",
        libc::SIGFPE => "SIGFPE",
        _ => "SIGNAL",
    }
}

#[inline]
unsafe fn write_str(s: &str) {
    let _ = libc::write(2, s.as_ptr() as *const c_void, s.len());
}

#[inline]
unsafe fn write_bytes(b: &[u8]) {
    let _ = libc::write(2, b.as_ptr() as *const c_void, b.len());
}

/// Write `v` as lowercase hex (no `0x` prefix), no allocation.
unsafe fn write_hex(v: usize) {
    if v == 0 {
        write_str("0");
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = buf.len();
    let mut val = v;
    while val > 0 {
        i -= 1;
        let d = (val & 0xf) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + (d - 10) };
        val >>= 4;
    }
    write_bytes(&buf[i..]);
}

/// Write `v` as decimal, no allocation.
unsafe fn write_dec(v: usize) {
    if v == 0 {
        write_str("0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut val = v;
    while val > 0 {
        i -= 1;
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    write_bytes(&buf[i..]);
}

unsafe fn current_tid() -> u64 {
    #[cfg(target_os = "linux")]
    {
        libc::syscall(libc::SYS_gettid) as u64
    }
    #[cfg(target_os = "macos")]
    {
        let mut t: u64 = 0;
        libc::pthread_threadid_np(0, &mut t);
        t
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

unsafe fn restore_and_reraise(sig: c_int) {
    let mut sa: libc::sigaction = std::mem::zeroed();
    sa.sa_sigaction = libc::SIG_DFL;
    libc::sigemptyset(&mut sa.sa_mask);
    sa.sa_flags = 0;
    libc::sigaction(sig, &sa, std::ptr::null_mut());
    libc::raise(sig);
}

unsafe extern "C" fn crash_handler(sig: c_int, info: *mut libc::siginfo_t, _uctx: *mut c_void) {
    // A fault *inside* the dump (e.g. while symbolizing) must not loop forever.
    if IN_HANDLER.swap(true, Ordering::SeqCst) {
        restore_and_reraise(sig);
        return;
    }

    write_str("\n==== probing: fatal signal ");
    write_str(sig_name(sig));
    write_str(" (");
    write_dec(sig as usize);
    write_str(") on thread ");
    write_dec(current_tid() as usize);
    write_str(" ====\n");

    if !info.is_null() {
        let addr = (*info).si_addr() as usize;
        write_str("fault address: 0x");
        write_hex(addr);
        write_str("\n");
    }

    write_str("native backtrace (crashing thread):\n");

    let mut idx = 0usize;
    backtrace::trace_unsynchronized(|frame| {
        let ip = frame.ip() as usize;
        write_str("  #");
        write_dec(idx);
        write_str("  0x");
        write_hex(ip);
        write_str("  ");

        let mut wrote_name = false;
        backtrace::resolve_frame_unsynchronized(frame, |symbol| {
            if !wrote_name {
                if let Some(name) = symbol.name() {
                    if let Some(s) = name.as_str() {
                        write_bytes(s.as_bytes());
                        wrote_name = true;
                    }
                }
            }
        });
        if !wrote_name {
            write_str("<unknown>");
        }
        write_str("\n");

        idx += 1;
        idx < MAX_FRAMES
    });

    write_str("==== end probing backtrace; re-raising ");
    write_str(sig_name(sig));
    write_str(" ====\n");

    spill_fatal_signal(
        sig,
        current_tid(),
        if info.is_null() {
            0
        } else {
            (*info).si_addr() as usize
        },
    );

    crash_grace_wait();

    restore_and_reraise(sig);
}

unsafe fn env_is_truthy(name: &str) -> bool {
    let Ok(val) = std::env::var(name) else {
        return false;
    };
    matches!(
        val.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

unsafe fn env_grace_sec() -> u64 {
    if env_is_truthy("PROBING_CRASH_NO_GRACE") {
        return 0;
    }
    match std::env::var("PROBING_CRASH_GRACE_SEC") {
        Ok(val) => val.trim().parse().unwrap_or(20),
        Err(_) => 20,
    }
}

unsafe fn env_rank(name: &str) -> i32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(-1)
}

unsafe fn should_signal_grace() -> bool {
    if env_is_truthy("PROBING_CRASH_HOLD") {
        return true;
    }
    let grace = env_grace_sec();
    if grace == 0 {
        return false;
    }
    if env_is_truthy("PROBING_CRASH_GRACE_ALL_RANKS") {
        return true;
    }
    env_rank("RANK") == 0 || env_rank("LOCAL_RANK") == 0
}

unsafe fn crash_grace_wait() {
    if !should_signal_grace() {
        return;
    }
    let pid = libc::getpid() as usize;
    write_str("\n[probing crash] fatal signal grace — attach: gdb -p ");
    write_dec(pid);
    write_str("\n");

    let grace = if env_is_truthy("PROBING_CRASH_HOLD") {
        0
    } else {
        env_grace_sec()
    };

    let deadline = if grace == 0 {
        i64::MAX
    } else {
        libc::time(std::ptr::null_mut()) + grace as i64
    };
    loop {
        if env_is_truthy("PROBING_CRASH_HOLD") {
            libc::usleep(200_000);
            continue;
        }
        let now = libc::time(std::ptr::null_mut());
        if now >= deadline {
            return;
        }
        libc::usleep(250_000);
    }
}

fn prepare_signal_spill() {
    let ctx = context::snapshot();
    CACHED_RANK.store(ctx.rank, Ordering::Relaxed);
    CACHED_LOCAL_RANK.store(ctx.local_rank, Ordering::Relaxed);

    if let Some(parent) = handler::signal_spill_path(ctx.pid).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let path = handler::signal_spill_path(ctx.pid);
    let bytes = path.as_os_str().as_encoded_bytes();
    unsafe {
        if bytes.len() + 1 >= SIGNAL_SPILL_PATH_CAP {
            return;
        }
        let p = std::ptr::addr_of_mut!(SIGNAL_SPILL_PATH).cast::<u8>();
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        p.add(bytes.len()).write(0);
        SIGNAL_SPILL_PATH_LEN.store(bytes.len(), Ordering::Relaxed);
    }
}

unsafe fn spill_fatal_signal(sig: c_int, tid: u64, fault_addr: usize) {
    let path_len = SIGNAL_SPILL_PATH_LEN.load(Ordering::Relaxed);
    if path_len == 0 {
        return;
    }

    let step = crash_atomic_step();
    let rank = CACHED_RANK.load(Ordering::Relaxed);
    let local_rank = CACHED_LOCAL_RANK.load(Ordering::Relaxed);
    let pid = libc::getpid();

    let mut buf = [0u8; 512];
    let mut n = 0usize;
    macro_rules! append {
        ($s:expr) => {{
            let b = $s.as_bytes();
            if n + b.len() < buf.len() {
                buf[n..n + b.len()].copy_from_slice(b);
                n += b.len();
            }
        }};
    }

    append!("{\"kind\":\"fatal_signal\",\"signal\":\"");
    append!(sig_name(sig));
    append!("\",\"pid\":");
    n = append_dec(&mut buf, n, pid as usize);
    append!(",\"rank\":");
    n = append_dec(&mut buf, n, rank.max(0) as usize);
    append!(",\"local_rank\":");
    n = append_dec(&mut buf, n, local_rank.max(0) as usize);
    append!(",\"global_step\":");
    n = append_dec(&mut buf, n, step.global_step as usize);
    append!(",\"micro_step\":");
    n = append_dec(&mut buf, n, step.micro_step as usize);
    append!(",\"tid\":");
    n = append_dec(&mut buf, n, tid as usize);
    append!(",\"fault_addr\":\"0x");
    n = append_hex(&mut buf, n, fault_addr);
    append!("\"}\n");

    let path_ptr = std::ptr::addr_of!(SIGNAL_SPILL_PATH).cast::<libc::c_char>();
    let fd = libc::open(
        path_ptr,
        libc::O_CREAT | libc::O_TRUNC | libc::O_WRONLY,
        0o644,
    );
    if fd >= 0 {
        let _ = libc::write(fd, buf.as_ptr() as *const c_void, n);
        let _ = libc::fsync(fd);
        let _ = libc::close(fd);
    }
}

unsafe fn append_dec(buf: &mut [u8], mut n: usize, val: usize) -> usize {
    if val == 0 {
        if n < buf.len() {
            buf[n] = b'0';
            n += 1;
        }
        return n;
    }
    let mut tmp = [0u8; 20];
    let mut i = tmp.len();
    let mut v = val;
    while v > 0 {
        i -= 1;
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let digits = &tmp[i..];
    if n + digits.len() < buf.len() {
        buf[n..n + digits.len()].copy_from_slice(digits);
        n += digits.len();
    }
    n
}

unsafe fn append_hex(buf: &mut [u8], mut n: usize, val: usize) -> usize {
    if val == 0 {
        if n < buf.len() {
            buf[n] = b'0';
            n += 1;
        }
        return n;
    }
    let mut tmp = [0u8; 16];
    let mut i = tmp.len();
    let mut v = val;
    while v > 0 {
        i -= 1;
        let d = (v & 0xf) as u8;
        tmp[i] = if d < 10 { b'0' + d } else { b'a' + (d - 10) };
        v >>= 4;
    }
    let digits = &tmp[i..];
    if n + digits.len() < buf.len() {
        buf[n..n + digits.len()].copy_from_slice(digits);
        n += digits.len();
    }
    n
}

/// Install backtrace-on-crash handlers for the common fatal signals. Idempotent.
pub fn install_crash_handler() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    prepare_signal_spill();

    unsafe {
        let mut ss: libc::stack_t = std::mem::zeroed();
        ss.ss_sp = core::ptr::addr_of_mut!(ALT_STACK) as *mut c_void;
        ss.ss_size = ALT_STACK_SIZE;
        ss.ss_flags = 0;
        libc::sigaltstack(&ss, std::ptr::null_mut());

        for &sig in FATAL_SIGNALS.iter() {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = crash_handler as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }

    log::info!("probing: crash backtrace handler installed");
}
