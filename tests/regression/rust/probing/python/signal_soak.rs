//! Long-running SIGPROF + SIGUSR2 stability test.
//!
//! CI runs this binary for 30 minutes on native x86_64 and aarch64 Linux
//! runners. For a short local Linux run:
//!
//! ```text
//! PROBING_SIGNAL_SOAK_SECS=30 cargo run --release \
//!   -p probing-rust-regression --bin python-signal-soak
//! ```

#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod signal_soak_linux;

#[cfg(target_os = "linux")]
fn main() {
    signal_soak_linux::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("python-signal-soak is Linux-only");
}
