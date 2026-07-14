//! Grace period and interactive hold before process exit (non-signal path).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::config;
use super::context;

static HELD: AtomicBool = AtomicBool::new(false);
static RELEASED: AtomicBool = AtomicBool::new(false);
static EXIT_AFTER_RELEASE: Mutex<bool> = Mutex::new(true);
static SIGNALS_REGISTERED: AtomicBool = AtomicBool::new(false);

pub fn is_held() -> bool {
    HELD.load(Ordering::SeqCst)
}

pub fn request_hold() {
    HELD.store(true, Ordering::SeqCst);
}

pub fn request_release(exit_after: bool) {
    if let Ok(mut guard) = EXIT_AFTER_RELEASE.lock() {
        *guard = exit_after;
    }
    RELEASED.store(true, Ordering::SeqCst);
    HELD.store(false, Ordering::SeqCst);
}

pub fn should_run_grace() -> bool {
    if config::force_hold() {
        return true;
    }
    if config::grace_sec() == 0 {
        return false;
    }
    if config::grace_all_ranks() {
        return true;
    }
    let ctx = context::snapshot();
    ctx.rank == 0 || ctx.local_rank == 0
}

pub fn grace_and_maybe_hold(exit_code: i32) -> i32 {
    register_signals();
    let ctx = context::snapshot();

    if config::force_hold() {
        request_hold();
    }

    if !should_run_grace() && !is_held() {
        return exit_code;
    }

    let deadline = Instant::now() + Duration::from_secs(config::grace_sec());
    loop {
        if RELEASED.load(Ordering::SeqCst) {
            let exit_after = EXIT_AFTER_RELEASE.lock().map(|g| *g).unwrap_or(true);
            return if exit_after { exit_code } else { 0 };
        }
        if hold_triggered(ctx.pid) {
            request_hold();
            print_hold_banner(ctx.pid, ctx.rank);
            while !RELEASED.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(200));
            }
            let exit_after = EXIT_AFTER_RELEASE.lock().map(|g| *g).unwrap_or(true);
            return if exit_after { exit_code } else { 0 };
        }
        if !is_held() && config::grace_sec() > 0 && Instant::now() >= deadline {
            return exit_code;
        }
        if !is_held() && config::grace_sec() == 0 && !config::force_hold() {
            return exit_code;
        }
        if !is_held() && config::grace_sec() > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
            eprint!(
                "\r[probing crash] exiting in {remaining}s — ENTER/touch/kill -USR1 to hold..."
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn register_signals() {
    if SIGNALS_REGISTERED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, SigHandler, Signal};
        unsafe {
            // SIGUSR1 = hold. Release is HTTP-only (`POST /apis/pythonext/crash/release`)
            // so SIGUSR2 remains dedicated to stack capture (installed at process start).
            let _ = signal::signal(Signal::SIGUSR1, SigHandler::Handler(on_hold_signal));
        }
    }
}

#[cfg(unix)]
extern "C" fn on_hold_signal(_: nix::libc::c_int) {
    request_hold();
}

fn hold_triggered(pid: i32) -> bool {
    if config::force_hold() {
        return true;
    }
    if std::path::Path::new(&context::hold_file_path(pid)).exists() {
        return true;
    }
    if is_held() {
        return true;
    }
    false
}

fn print_hold_banner(pid: i32, rank: i32) {
    eprintln!(
        "\nProcess HELD for debugging (pid={pid}, rank={rank}).\n\
           attach : gdb -p {pid}\n\
           python : py-spy dump --pid {pid}\n\
           release: curl -X POST http://127.0.0.1:<port>/apis/pythonext/crash/release\n\
                    rm {}\n",
        context::hold_file_path(pid)
    );
}
