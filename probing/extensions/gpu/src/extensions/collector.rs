use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use probing_core::sync::lock_mutex;
use probing_memtable::discover::ExposedTable;
use probing_memtable::{DType, Schema, Value};
use thiserror::Error;

use super::backend::{discover_backends, selected_backends, GpuBackend, GpuMemorySample};

#[cfg(feature = "cuda")]
use super::backend::read_utilization_by_index;

const CHUNK_SIZE: u32 = 4096;
const NUM_CHUNKS: u32 = 8;
const DEFAULT_SAMPLE_INTERVAL_MS: u64 = 1000;

/// Autostart interval from env, or `None` when GPU sampling should stay off.
///
/// Priority (aligned with `cpu.*` knobs):
/// - `PROBING_GPU=off` → disabled.
/// - `PROBING_GPU_SAMPLE_MS=0` → disabled; any positive value forces sampling at that interval.
/// - `PROBING_GPU=on` → force enable (even if backend probe fails at start).
/// - Otherwise: **auto** — enabled at 1000 ms when a GPU backend is detected; silent skip when none.
pub fn autostart_interval_ms() -> Option<u64> {
    if matches!(
        std::env::var("PROBING_GPU").ok().as_deref(),
        Some(v) if matches!(v.trim(), "0" | "off" | "false" | "no")
    ) {
        return None;
    }

    let forced_on = matches!(
        std::env::var("PROBING_GPU").ok().as_deref(),
        Some(v) if matches!(v.trim(), "1" | "on" | "true" | "yes")
    );

    let ms = std::env::var("PROBING_GPU_SAMPLE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());

    if ms == Some(0) {
        return None;
    }
    if let Some(interval) = ms {
        return Some(interval);
    }
    if forced_on {
        return Some(DEFAULT_SAMPLE_INTERVAL_MS);
    }

    if discover_backends().is_empty() {
        return None;
    }

    Some(DEFAULT_SAMPLE_INTERVAL_MS)
}

/// Start the background GPU collector (creates `gpu.utilization` memtable).
/// Idempotent: returns `Ok` if the collector is already running.
pub fn start_gpu_sampling(interval_ms: u64) -> Result<(), CollectorError> {
    match GpuCollector::instance().start(GpuCollectorConfig {
        interval: Duration::from_millis(interval_ms),
        iterations: None,
    }) {
        Ok(()) | Err(CollectorError::AlreadyRunning) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Start GPU sampling from env. Call once after engine init.
pub fn start_gpu_sampling_from_env() {
    let Some(interval_ms) = autostart_interval_ms() else {
        log::debug!(
            "GPU sampling not started (no backend or disabled via PROBING_GPU / PROBING_GPU_SAMPLE_MS=0)"
        );
        return;
    };
    match start_gpu_sampling(interval_ms) {
        Ok(()) => {
            log::info!("GPU sampling started (interval={interval_ms}ms, auto-detected backend)")
        }
        Err(CollectorError::AlreadyRunning) => log::debug!("GPU sampling already running"),
        Err(CollectorError::NoBackend) => {
            log::info!(
                "GPU sampling was requested (PROBING_GPU=on or PROBING_GPU_SAMPLE_MS) but no GPU backend is available"
            );
        }
        Err(e) => log::warn!("GPU sampling start failed: {e}"),
    }
}

fn utilization_schema() -> Schema {
    Schema::new()
        .col("ts", DType::I64)
        .col("backend", DType::Str)
        .col("device_id", DType::I32)
        .col("name", DType::Str)
        .col("memory_model", DType::Str)
        .col("chip", DType::Str)
        .col("free_bytes", DType::I64)
        .col("total_bytes", DType::I64)
        .col("used_bytes", DType::I64)
        .col("mem_used_pct", DType::F32)
        .col("gpu_util_pct", DType::F32)
        .col("mem_controller_util_pct", DType::F32)
        .col("renderer_util_pct", DType::F32)
        .col("tiler_util_pct", DType::F32)
        .col("driver_mem_bytes", DType::I64)
        .col("wall_ns", DType::I64)
}

#[derive(Debug, Clone)]
pub struct GpuCollectorConfig {
    pub interval: Duration,
    pub iterations: Option<i64>,
}

#[derive(Error, Debug)]
pub enum CollectorError {
    #[error("GPU collector already running")]
    AlreadyRunning,
    #[error("No GPU backend available")]
    NoBackend,
    #[error("Failed to open GPU memtable")]
    OpenFailed(#[from] probing_memtable::MemtableError),
    #[error("GPU collector stop failed: {0}")]
    StopFailed(String),
}

fn ts_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

fn push_utilization_row(table: &mut ExposedTable, ts: i64, wall_ns: u64, sample: &GpuMemorySample) {
    let used = sample.used_bytes();
    if !table.push_row(&[
        Value::I64(ts),
        Value::Str(sample.backend.as_str()),
        Value::I32(sample.ordinal),
        Value::Str(&sample.name),
        Value::Str(sample.memory_model.as_str()),
        Value::Str(sample.chip.as_deref().unwrap_or("")),
        Value::I64(sample.free_bytes as i64),
        Value::I64(sample.total_bytes as i64),
        Value::I64(used as i64),
        Value::F32(sample.used_pct()),
        Value::F32(opt_f32(sample.gpu_util_pct)),
        Value::F32(opt_f32(sample.mem_controller_util_pct)),
        Value::F32(opt_f32(sample.renderer_util_pct)),
        Value::F32(opt_f32(sample.tiler_util_pct)),
        Value::I64(sample.driver_mem_bytes.unwrap_or(0) as i64),
        Value::I64(wall_ns as i64),
    ]) {
        log::warn!("gpu collector: push_row failed for gpu.utilization");
    }
}

fn opt_f32(v: Option<f32>) -> f32 {
    v.unwrap_or(-1.0)
}

fn sample_all(backends: &[Box<dyn GpuBackend>]) -> Vec<GpuMemorySample> {
    #[cfg(feature = "cuda")]
    let nvidia_utils = read_utilization_by_index();

    let mut samples = Vec::new();
    for backend in backends {
        for device in backend.probe_devices() {
            if let Some(sample) = backend.sample_memory(device.ordinal) {
                let sample = {
                    #[cfg(feature = "cuda")]
                    {
                        let mut s = sample;
                        if s.backend == super::backend::GpuBackendKind::Cuda {
                            if let Some(u) = nvidia_utils.get(&s.ordinal) {
                                s.gpu_util_pct = Some(u.gpu_util_pct);
                                s.mem_controller_util_pct = Some(u.mem_controller_util_pct);
                            }
                        }
                        s
                    }
                    #[cfg(not(feature = "cuda"))]
                    {
                        sample
                    }
                };
                samples.push(sample);
            }
        }
    }
    samples
}

fn lock_gpu_table(m: &Mutex<ExposedTable>) -> MutexGuard<'_, ExposedTable> {
    lock_mutex(m, "gpu memtable")
}

fn lock_gpu_collector<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    lock_mutex(m, "gpu collector")
}

pub struct GpuCollector {
    running: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
    table: Mutex<Option<Arc<Mutex<ExposedTable>>>>,
}

impl GpuCollector {
    pub fn instance() -> &'static Self {
        static INSTANCE: Lazy<GpuCollector> = Lazy::new(|| GpuCollector {
            running: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
            table: Mutex::new(None),
        });
        &INSTANCE
    }

    fn shared_table(&self) -> probing_memtable::MemtableResult<Arc<Mutex<ExposedTable>>> {
        let mut guard = lock_gpu_collector(&self.table);
        if guard.is_none() {
            *guard = Some(Arc::new(Mutex::new(ExposedTable::create(
                "gpu.utilization",
                &utilization_schema(),
                CHUNK_SIZE,
                NUM_CHUNKS,
            )?)));
        }
        guard.as_ref().cloned().ok_or_else(|| {
            log::error!("gpu collector: shared table slot empty after init");
            probing_memtable::MemtableError::InvalidBuffer("gpu table unavailable")
        })
    }

    pub fn start(&self, config: GpuCollectorConfig) -> Result<(), CollectorError> {
        let backends = selected_backends();
        if backends.is_empty() {
            return Err(CollectorError::NoBackend);
        }

        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CollectorError::AlreadyRunning);
        }

        let running = self.running.clone();
        let table = self.shared_table()?;

        let handle = thread::spawn(move || {
            let panicked = catch_unwind(AssertUnwindSafe(|| {
                let mut iterations = config.iterations;
                while running.load(Ordering::SeqCst) {
                    if let Some(iter) = iterations.as_mut() {
                        if *iter <= 0 {
                            break;
                        }
                        *iter -= 1;
                    }

                    let wall_start = Instant::now();
                    let ts = ts_micros();
                    let samples = sample_all(&backends);
                    let wall_ns = wall_start.elapsed().as_nanos() as u64;

                    let mut exposed = lock_gpu_table(&table);
                    for sample in &samples {
                        push_utilization_row(&mut exposed, ts, wall_ns, sample);
                    }

                    thread::sleep(config.interval);
                }
            }))
            .is_err();
            if panicked {
                log::error!("gpu collector thread panicked");
            } else {
                log::warn!("gpu collector thread exited");
            }
            running.store(false, Ordering::SeqCst);
        });

        *lock_gpu_collector(&self.handle) = Some(handle);
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn stop(&self) -> Result<(), CollectorError> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }

        if let Some(handle) = lock_gpu_collector(&self.handle).take() {
            handle
                .join()
                .map_err(|_| CollectorError::StopFailed("thread join failed".into()))?;
        }

        Ok(())
    }
}

/// Cached device list for the `gpu.devices` table and diagnostics.
pub fn cached_devices() -> Vec<super::backend::GpuDeviceInfo> {
    discover_backends()
        .into_iter()
        .flat_map(|b| b.probe_devices())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn clear_gpu_env() {
        std::env::remove_var("PROBING_GPU");
        std::env::remove_var("PROBING_GPU_SAMPLE_MS");
    }

    #[test]
    fn autostart_respects_explicit_off() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_gpu_env();
        std::env::set_var("PROBING_GPU", "off");
        assert!(autostart_interval_ms().is_none());
        clear_gpu_env();
    }

    #[test]
    fn autostart_force_on_without_backend_probe() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_gpu_env();
        std::env::set_var("PROBING_GPU", "on");
        assert_eq!(autostart_interval_ms(), Some(DEFAULT_SAMPLE_INTERVAL_MS));
        clear_gpu_env();
    }

    #[test]
    fn autostart_honors_sample_ms_override() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_gpu_env();
        std::env::set_var("PROBING_GPU_SAMPLE_MS", "250");
        assert_eq!(autostart_interval_ms(), Some(250));
        clear_gpu_env();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn autostart_auto_when_backend_present() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_gpu_env();
        if discover_backends().is_empty() {
            return;
        }
        assert_eq!(autostart_interval_ms(), Some(DEFAULT_SAMPLE_INTERVAL_MS));
        clear_gpu_env();
    }

    #[test]
    #[cfg(all(not(target_os = "macos"), not(feature = "cuda")))]
    fn autostart_auto_off_without_backend() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        clear_gpu_env();
        assert!(autostart_interval_ms().is_none());
    }

    #[test]
    fn stop_is_idempotent_when_not_running() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        let collector = GpuCollector::instance();
        let _ = collector.stop();
        collector.stop().expect("stop when idle");
    }

    #[test]
    fn collector_stop_after_bounded_iterations() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        let collector = GpuCollector::instance();
        let _ = collector.stop();

        if discover_backends().is_empty() {
            return;
        }

        collector
            .start(GpuCollectorConfig {
                interval: Duration::from_millis(1),
                iterations: Some(5),
            })
            .expect("start collector");

        std::thread::sleep(Duration::from_millis(50));
        collector.stop().expect("stop collector");
    }
}
