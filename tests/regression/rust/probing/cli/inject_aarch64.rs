//! Regression: aarch64 ptrace inject against a live process (Linux + aarch64 only).
//!
//! Unit tests for shellcode layout and register handling live in
//! `src/inject/injection_aarch64.rs`.

#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
use probing_cli::inject::{Injector, Process};
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
use std::process::Command;
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
use std::thread;
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
use std::time::Duration;

#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
#[test]
fn injection_basic_fails_on_missing_library() {
    let mut target = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("Failed to spawn target process");

    let target_pid = target.id();
    thread::sleep(Duration::from_millis(100));

    let proc = Process::get(target_pid).expect("Failed to find target process");
    let dummy_lib = std::path::Path::new("/tmp/dummy.so");

    let result = Injector::attach(proc).and_then(|mut injector| injector.inject(dummy_lib, vec![]));

    let _ = target.kill();

    match result {
        Ok(_) => panic!("Expected injection to fail due to missing library"),
        Err(e) => {
            // `Display` only shows the outermost context ("failed to perform aarch64
            // injection"); the missing-file cause lives deeper (canonicalize or dlopen).
            let chain = format!("{e:#}");
            assert!(
                chain.contains("No such file")
                    || chain.contains("dlopen")
                    || chain.contains("couldn't get absolute path"),
                "unexpected error chain: {chain}"
            );
        }
    }
}

#[cfg(not(all(target_arch = "aarch64", target_os = "linux")))]
#[test]
fn injection_skipped_on_non_aarch64_linux() {}
