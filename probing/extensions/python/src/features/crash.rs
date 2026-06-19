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
use std::sync::atomic::{AtomicBool, Ordering};

use nix::libc;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_HANDLER: AtomicBool = AtomicBool::new(false);

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

    restore_and_reraise(sig);
}

/// Install backtrace-on-crash handlers for the common fatal signals. Idempotent.
pub fn install_crash_handler() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

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
