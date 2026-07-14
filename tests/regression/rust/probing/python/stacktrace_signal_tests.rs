//! Signal-path integration for stacktrace capture (public APIs).
//!
//! Crate-local unit tests cover SIGPROF handler + ring; this binary exercises
//! the SIGUSR2 on-demand contract used by HTTP / dynamic tracers.

#![cfg(unix)]

use std::time::Duration;

use probing_python::features::stacktrace::capture;
use probing_python::features::stacktrace::snapshot::StackSource;

#[test]
fn sigusr2_public_api_captures_native_pc_on_current_thread() {
    capture::install_sigusr2_handler();
    capture::register_python_thread();
    capture::register_main_os_tid();

    let tid = capture::current_tid();
    let snap = capture::capture_thread_snapshot_signal(tid, Duration::from_secs(2))
        .expect("SIGUSR2 snapshot via public capture API");

    assert_eq!(snap.tid, tid);
    assert_eq!(snap.source, StackSource::Sigusr2);
    assert!(
        snap.native_len >= 1,
        "expected at least the interrupted PC; flags={:?} native_len={}",
        snap.flags,
        snap.native_len
    );
}
