use anyhow::Result;

use once_cell::sync::Lazy;
use pprof::ProfilerGuard;
use pprof::ProfilerGuardBuilder;
use probing_core::run_on_native_thread;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// Set while a `ProfilerGuard` is held (ITIMER_PROF / SIGPROF sampling active).
static SAMPLING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Libraries whose frames are skipped during SIGPROF sampling (reduces handler work).
const PPROF_BLOCKLIST: &[&str] = &[
    "libc",
    "libsystem",
    "libpthread",
    "CoreFoundation",
    "libgcc",
    "vdso",
    "libtorch",
    "libtorch_cpu",
    "libpython",
    "Python",
];

#[cfg(target_os = "macos")]
const MAX_SAMPLE_FREQ: i32 = 50;

#[cfg(not(target_os = "macos"))]
const MAX_SAMPLE_FREQ: i32 = 1000;

pub struct PprofHolder {
    guard: Mutex<Option<ProfilerGuard<'static>>>,
    frequency: Mutex<Option<i32>>,
}

impl PprofHolder {
    pub fn reset(&self) {
        let _ = self.guard.lock().map(|mut holder| {
            if holder.take().is_some() {
                SAMPLING_ACTIVE.store(false, Ordering::Release);
            }
        });
    }

    fn start_on_current_thread(&self, freq: i32) {
        log::info!("starting pprof CPU sampling at {freq} Hz");
        let _ = self.guard.lock().map(|mut holder| {
            if holder.take().is_some() {
                SAMPLING_ACTIVE.store(false, Ordering::Release);
            }
            match ProfilerGuardBuilder::default()
                .frequency(freq)
                .blocklist(PPROF_BLOCKLIST)
                .build()
            {
                Ok(ph) => {
                    SAMPLING_ACTIVE.store(true, Ordering::Release);
                    if let Ok(mut freq_slot) = self.frequency.lock() {
                        *freq_slot = Some(freq);
                    }
                    eprintln!("probing: pprof sampling started ({freq} Hz)");
                    holder.replace(ph);
                }
                Err(e) => {
                    log::error!(
                        "pprof ProfilerGuard build failed (freq={freq}): {e}; profiling unavailable"
                    );
                    eprintln!("probing: pprof failed to start: {e}");
                }
            };
        });
    }

    pub fn setup(&self, freq: i32) {
        let requested = freq;
        let freq = freq.clamp(1, MAX_SAMPLE_FREQ);
        #[cfg(target_os = "macos")]
        if requested > MAX_SAMPLE_FREQ {
            eprintln!(
                "probing: pprof frequency capped from {requested} to {freq} Hz on macOS (SIGPROF + PyTorch is fragile)"
            );
        }

        // Never start ITIMER_PROF from a Tokio worker while holding engine locks (SET path).
        // Spawn a dedicated thread, then hop to the native bridge thread before arming signals.
        if let Err(e) = thread::Builder::new()
            .name("probing-pprof".into())
            .spawn(move || {
                thread::sleep(Duration::from_millis(100));
                run_on_native_thread(move || PPROF_HOLDER.start_on_current_thread(freq));
            })
        {
            eprintln!("probing: failed to spawn pprof thread: {e}");
        }
    }

    pub fn flamegraph(&self) -> Result<String> {
        let holder = self
            .guard
            .lock()
            .map_err(|e| anyhow::anyhow!("pprof lock poisoned: {e}"))?;

        if let Some(pp) = holder.as_ref() {
            let report = pp.report().build()?;
            let mut graph: Vec<u8> = vec![];
            report
                .flamegraph(&mut graph)
                .map_err(|e| anyhow::anyhow!("pprof flamegraph write failed: {e}"))?;
            let graph = String::from_utf8(graph)?;
            Ok(graph)
        } else {
            Err(anyhow::anyhow!("no pprof"))
        }
    }
}

impl Drop for PprofHolder {
    fn drop(&mut self) {
        SAMPLING_ACTIVE.store(false, Ordering::Release);
    }
}

pub static PPROF_HOLDER: Lazy<PprofHolder> = Lazy::new(|| PprofHolder {
    guard: Mutex::new(None),
    frequency: Mutex::new(None),
});

/// True while ITIMER_PROF sampling is active. Stack tracing must not deliver
/// SIGUSR2 during this window (handler is not async-signal-safe).
pub fn is_sampling_active() -> bool {
    SAMPLING_ACTIVE.load(Ordering::Acquire)
}

pub fn pprof_handler() {
    PPROF_HOLDER.setup(100);
}

pub fn setup(freq: u64) -> Result<()> {
    PPROF_HOLDER.setup(freq as i32);
    Ok(())
}

pub fn flamegraph() -> Result<String> {
    PPROF_HOLDER.flamegraph()
}
