//! Linux implementation of the long-running SIGPROF + SIGUSR2 stability test.

use std::hint::black_box;
use std::time::{Duration, Instant};

use probing_python::features::stacktrace::capture;
use probing_python::features::stacktrace::metrics;
use probing_python::features::stacktrace::snapshot::StackSource;
use probing_python::features::stacktrace::tracers::pprof;

const DEFAULT_DURATION_SECS: u64 = 30;
const PPROF_FREQUENCY_HZ: u64 = 100;
const SIGUSR2_INTERVAL: Duration = Duration::from_millis(20);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(30);

struct SignalMaskGuard {
    original: libc::sigset_t,
}

impl SignalMaskGuard {
    fn block_sigprof() -> Self {
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPROF);
            let mut original: libc::sigset_t = std::mem::zeroed();
            assert_eq!(
                libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut original),
                0,
                "failed to block SIGPROF before spawning the sampler"
            );
            assert_eq!(
                libc::sigismember(&original, libc::SIGPROF),
                0,
                "signal soak requires SIGPROF to be initially unblocked"
            );
            Self { original }
        }
    }

    fn unblock_sigprof(&self) {
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGPROF);
            assert_eq!(
                libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()),
                0,
                "failed to unblock SIGPROF on the registered main thread"
            );
        }
    }
}

impl Drop for SignalMaskGuard {
    fn drop(&mut self) {
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.original, std::ptr::null_mut());
        }
    }
}

struct PprofResetGuard;

impl Drop for PprofResetGuard {
    fn drop(&mut self) {
        pprof::reset();
    }
}

fn sampler_samples() -> u64 {
    metrics::fingerprint_hits().saturating_add(metrics::fingerprint_misses())
}

fn burn_cpu(state: &mut u64) {
    for _ in 0..20_000 {
        *state ^= state.wrapping_shl(13);
        *state ^= state.wrapping_shr(7);
        *state ^= state.wrapping_shl(17);
        black_box(*state);
    }
}

pub fn run() {
    if let Ok(expected) = std::env::var("PROBING_SIGNAL_SOAK_EXPECT_ARCH") {
        assert_eq!(std::env::consts::ARCH, expected);
    }
    let duration = Duration::from_secs(
        std::env::var("PROBING_SIGNAL_SOAK_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_DURATION_SECS),
    );
    eprintln!(
        "starting signal soak: arch={} duration={duration:?} pprof_hz={PPROF_FREQUENCY_HZ}",
        std::env::consts::ARCH
    );

    // Block before setup so the sampler consumer inherits SIGPROF blocked.
    // After setup only this registered thread is unblocked, making ITIMER_PROF
    // delivery deterministic and ensuring every handler runs on an alt stack.
    let signal_mask = SignalMaskGuard::block_sigprof();
    capture::install_sigusr2_handler();
    capture::register_python_thread();
    capture::register_main_os_tid();
    let target_tid = capture::current_tid();
    pprof::setup(PPROF_FREQUENCY_HZ).expect("start asynchronous SIGPROF sampler");
    let _pprof_reset = PprofResetGuard;
    signal_mask.unblock_sigprof();

    let started = Instant::now();
    let deadline = started + duration;
    let mut next_sigusr2 = started;
    let mut next_progress = started + PROGRESS_INTERVAL.min(duration);
    let mut last_pprof_samples = sampler_samples();
    let mut sigusr2_successes = 0u64;
    let mut last_sigusr2_successes = 0u64;
    let mut state = 0xA5A5_5A5A_D3C3_B1B1u64;

    while Instant::now() < deadline {
        burn_cpu(&mut state);
        let now = Instant::now();
        if now >= next_sigusr2 {
            let snapshot =
                capture::capture_thread_snapshot_signal(target_tid, Duration::from_secs(1))
                    .expect("SIGUSR2 capture must remain responsive");
            assert_eq!(snapshot.tid, target_tid);
            assert_eq!(snapshot.source, StackSource::Sigusr2);
            assert!(
                snapshot.native_len >= 1,
                "SIGUSR2 snapshot lost the interrupted native PC"
            );
            sigusr2_successes += 1;
            next_sigusr2 = now + SIGUSR2_INTERVAL;
        }

        if now >= next_progress {
            let current_pprof_samples = sampler_samples();
            assert!(
                current_pprof_samples > last_pprof_samples,
                "SIGPROF sampler made no progress during the last interval"
            );
            assert!(
                sigusr2_successes > last_sigusr2_successes,
                "SIGUSR2 capture made no progress during the last interval"
            );
            eprintln!(
                "signal soak heartbeat: elapsed={:?} pprof_samples={} sigusr2_captures={} dropped_ring={}",
                started.elapsed(),
                current_pprof_samples,
                sigusr2_successes,
                metrics::dropped_ring(),
            );
            last_pprof_samples = current_pprof_samples;
            last_sigusr2_successes = sigusr2_successes;
            next_progress = now + PROGRESS_INTERVAL;
        }
    }

    // Allow the consumer to drain the final ring entries before final assertions.
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        sampler_samples() > 0,
        "SIGPROF produced no accepted samples"
    );
    assert!(sigusr2_successes > 0, "SIGUSR2 produced no snapshots");
    assert!(
        !pprof::folded_lines_snapshot().is_empty(),
        "SIGPROF produced no exportable folded stacks"
    );
}
