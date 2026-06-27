use std::collections::HashMap;
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

use super::sample::{ProcessSample, ThreadSample};
use super::sampler::host_sampler;

const CHUNK_SIZE: u32 = 4096;
const NUM_CHUNKS: u32 = 8;
const DEFAULT_SAMPLE_INTERVAL_MS: u64 = 1000;

/// Autostart interval from env, or `None` when CPU sampling is disabled.
///
/// - Default: 1000 ms (enabled).
/// - `PROBING_CPU=off` → disabled.
/// - `PROBING_CPU_SAMPLE_MS=0` → disabled; any positive value overrides interval.
pub fn autostart_interval_ms() -> Option<u64> {
    if matches!(
        std::env::var("PROBING_CPU").ok().as_deref(),
        Some(v) if matches!(v.trim(), "0" | "off" | "false" | "no")
    ) {
        return None;
    }
    let ms = std::env::var("PROBING_CPU_SAMPLE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SAMPLE_INTERVAL_MS);
    if ms == 0 {
        None
    } else {
        Some(ms)
    }
}

fn autostart_thread_top_n() -> usize {
    std::env::var("PROBING_CPU_THREAD_TOP_N")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8)
}

/// Start the background CPU collector (creates `cpu.utilization` / `cpu.tasks` memtables).
/// Idempotent: returns `Ok` if the collector is already running.
pub fn start_cpu_sampling(interval_ms: u64, thread_top_n: usize) -> Result<(), CollectorError> {
    match CpuCollector::instance().start(CpuCollectorConfig {
        interval: Duration::from_millis(interval_ms),
        thread_top_n,
        iterations: None,
    }) {
        Ok(()) | Err(CollectorError::AlreadyRunning) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Start CPU sampling from env (default on). Call once after engine init.
pub fn start_cpu_sampling_from_env() {
    let Some(interval_ms) = autostart_interval_ms() else {
        log::debug!("CPU sampling disabled (PROBING_CPU or PROBING_CPU_SAMPLE_MS=0)");
        return;
    };
    match start_cpu_sampling(interval_ms, autostart_thread_top_n()) {
        Ok(()) => log::info!("CPU sampling started (interval={interval_ms}ms)"),
        Err(CollectorError::AlreadyRunning) => {
            log::debug!("CPU sampling already running");
        }
        Err(e) => log::warn!("CPU sampling start failed: {e}"),
    }
}

fn utilization_schema() -> Schema {
    Schema::new()
        .col("ts", DType::I64)
        .col("scope", DType::Str)
        .col("platform", DType::Str)
        .col("tid", DType::I32)
        .col("comm", DType::Str)
        .col("wall_ns", DType::I64)
        .col("delta_user_ns", DType::I64)
        .col("delta_sys_ns", DType::I64)
        .col("delta_total_ns", DType::I64)
        .col("cpu_user_pct", DType::F32)
        .col("cpu_sys_pct", DType::F32)
        .col("cpu_total_pct", DType::F32)
        .col("cum_user_ns", DType::I64)
        .col("cum_sys_ns", DType::I64)
        .col("rss_kb", DType::I64)
        .col("thread_count", DType::I32)
        .col("delta_vol_ctxt", DType::I64)
        .col("delta_invol_ctxt", DType::I64)
        .col("state", DType::Str)
        .col("wchan", DType::Str)
}

fn tasks_schema() -> Schema {
    Schema::new()
        .col("ts", DType::I64)
        .col("platform", DType::Str)
        .col("tid", DType::I32)
        .col("comm", DType::Str)
        .col("state", DType::Str)
        .col("wchan", DType::Str)
        .col("wall_ns", DType::I64)
        .col("delta_user_ns", DType::I64)
        .col("delta_sys_ns", DType::I64)
        .col("delta_total_ns", DType::I64)
}

#[derive(Debug, Clone)]
pub struct CpuCollectorConfig {
    pub interval: Duration,
    pub thread_top_n: usize,
    pub iterations: Option<i64>,
}

impl Default for CpuCollectorConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            thread_top_n: 8,
            iterations: None,
        }
    }
}

#[derive(Error, Debug)]
pub enum CollectorError {
    #[error("CPU collector already running")]
    AlreadyRunning,
    #[error("Failed to open CPU memtables: {0}")]
    OpenFailed(String),
    #[error("CPU collector stop failed: {0}")]
    StopFailed(String),
}

struct SampleState {
    last_wall: Instant,
    last_process: Option<ProcessSample>,
    last_threads: HashMap<i32, ThreadSample>,
}

impl SampleState {
    fn new() -> Self {
        Self {
            last_wall: Instant::now(),
            last_process: None,
            last_threads: HashMap::new(),
        }
    }
}

fn pct(delta_ns: u64, wall_ns: u64) -> f32 {
    if wall_ns == 0 {
        return 0.0;
    }
    (delta_ns as f64 / wall_ns as f64 * 100.0) as f32
}

fn ts_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

#[allow(clippy::too_many_arguments)]
fn push_utilization_row(
    table: &mut ExposedTable,
    ts: i64,
    platform: &str,
    scope: &str,
    tid: i32,
    comm: &str,
    wall_ns: u64,
    delta_user_ns: u64,
    delta_sys_ns: u64,
    cum_user_ns: i64,
    cum_sys_ns: i64,
    rss_kb: i64,
    thread_count: i32,
    delta_vol_ctxt: i64,
    delta_invol_ctxt: i64,
    state: &str,
    wchan: &str,
) {
    let delta_total = delta_user_ns.saturating_add(delta_sys_ns);
    if !table.push_row(&[
        Value::I64(ts),
        Value::Str(scope),
        Value::Str(platform),
        Value::I32(tid),
        Value::Str(comm),
        Value::I64(wall_ns as i64),
        Value::I64(delta_user_ns as i64),
        Value::I64(delta_sys_ns as i64),
        Value::I64(delta_total as i64),
        Value::F32(pct(delta_user_ns, wall_ns)),
        Value::F32(pct(delta_sys_ns, wall_ns)),
        Value::F32(pct(delta_total, wall_ns)),
        Value::I64(cum_user_ns),
        Value::I64(cum_sys_ns),
        Value::I64(rss_kb),
        Value::I32(thread_count),
        Value::I64(delta_vol_ctxt),
        Value::I64(delta_invol_ctxt),
        Value::Str(state),
        Value::Str(wchan),
    ]) {
        log::warn!("cpu collector: push_row failed for cpu.processes");
    }
}

fn push_tasks_row(
    table: &mut ExposedTable,
    ts: i64,
    platform: &str,
    thread: &ThreadSample,
    wall_ns: u64,
    delta_user_ns: u64,
    delta_sys_ns: u64,
) {
    let state = thread.state.as_deref().unwrap_or("");
    let wchan = thread.wchan.as_deref().unwrap_or("");
    let delta_total = delta_user_ns.saturating_add(delta_sys_ns);
    if !table.push_row(&[
        Value::I64(ts),
        Value::Str(platform),
        Value::I32(thread.tid),
        Value::Str(&thread.comm),
        Value::Str(state),
        Value::Str(wchan),
        Value::I64(wall_ns as i64),
        Value::I64(delta_user_ns as i64),
        Value::I64(delta_sys_ns as i64),
        Value::I64(delta_total as i64),
    ]) {
        log::warn!("cpu collector: push_row failed for cpu.tasks");
    }
}

pub struct CpuCollector {
    running: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
    tables: Mutex<Option<Arc<CollectorTables>>>,
}

fn lock_cpu_table(m: &Mutex<ExposedTable>) -> MutexGuard<'_, ExposedTable> {
    lock_mutex(m, "cpu memtable")
}

fn lock_cpu_collector<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    lock_mutex(m, "cpu collector")
}

struct CollectorTables {
    utilization: Mutex<ExposedTable>,
    tasks: Mutex<ExposedTable>,
}

impl CollectorTables {
    fn open() -> Result<Self, std::io::Error> {
        Ok(Self {
            utilization: Mutex::new(ExposedTable::create(
                "cpu.utilization",
                &utilization_schema(),
                CHUNK_SIZE,
                NUM_CHUNKS,
            )?),
            tasks: Mutex::new(ExposedTable::create(
                "cpu.tasks",
                &tasks_schema(),
                CHUNK_SIZE,
                NUM_CHUNKS,
            )?),
        })
    }
}

impl CpuCollector {
    pub fn instance() -> &'static Self {
        static INSTANCE: Lazy<CpuCollector> = Lazy::new(|| CpuCollector {
            running: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
            tables: Mutex::new(None),
        });
        &INSTANCE
    }

    fn shared_tables(&self) -> Result<Arc<CollectorTables>, std::io::Error> {
        let mut guard = lock_cpu_collector(&self.tables);
        if guard.is_none() {
            *guard = Some(Arc::new(CollectorTables::open()?));
        }
        Ok(Arc::clone(guard.as_ref().unwrap()))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn utilization_row_count(&self) -> usize {
        let guard = lock_cpu_collector(&self.tables);
        let tables = match guard.as_ref() {
            Some(t) => Arc::clone(t),
            None => return 0,
        };
        let table = lock_cpu_table(&tables.utilization);
        let view = table.view();
        (0..view.num_chunks()).map(|c| view.num_rows(c)).sum()
    }

    pub fn start(&self, config: CpuCollectorConfig) -> Result<(), CollectorError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CollectorError::AlreadyRunning);
        }

        let running = self.running.clone();
        let tables = self
            .shared_tables()
            .map_err(|e| CollectorError::OpenFailed(e.to_string()))?;
        let handle = thread::spawn(move || {
            let sampler = host_sampler();
            let platform = sampler.platform().to_string();

            let mut state = SampleState::new();
            let mut iterations = config.iterations;

            while running.load(Ordering::SeqCst) {
                if let Some(iter) = iterations.as_mut() {
                    if *iter <= 0 {
                        break;
                    }
                    *iter -= 1;
                }

                let now = Instant::now();
                let wall_ns = now.duration_since(state.last_wall).as_nanos() as u64;
                let ts = ts_micros();

                match sampler.sample_process() {
                    Ok(curr) => {
                        if let Some(prev) = &state.last_process {
                            if wall_ns > 0 {
                                let delta_user =
                                    curr.cputime_user_ns.saturating_sub(prev.cputime_user_ns);
                                let delta_sys =
                                    curr.cputime_sys_ns.saturating_sub(prev.cputime_sys_ns);
                                let delta_vol = curr.vol_ctxt.saturating_sub(prev.vol_ctxt) as i64;
                                let delta_invol =
                                    curr.invol_ctxt.saturating_sub(prev.invol_ctxt) as i64;
                                push_utilization_row(
                                    &mut lock_cpu_table(&tables.utilization),
                                    ts,
                                    &platform,
                                    "process",
                                    0,
                                    "",
                                    wall_ns,
                                    delta_user,
                                    delta_sys,
                                    curr.cputime_user_ns as i64,
                                    curr.cputime_sys_ns as i64,
                                    (curr.rss_bytes / 1024) as i64,
                                    curr.thread_count as i32,
                                    delta_vol,
                                    delta_invol,
                                    "",
                                    "",
                                );
                            }
                        }
                        state.last_process = Some(curr);
                    }
                    Err(e) => log::warn!("cpu process sample failed: {e}"),
                }

                match sampler.sample_threads(config.thread_top_n) {
                    Ok(threads) => {
                        if wall_ns > 0 {
                            for thread in &threads {
                                let prev = state.last_threads.get(&thread.tid);
                                let delta_user = thread
                                    .cputime_user_ns
                                    .saturating_sub(prev.map(|p| p.cputime_user_ns).unwrap_or(0));
                                let delta_sys = thread
                                    .cputime_sys_ns
                                    .saturating_sub(prev.map(|p| p.cputime_sys_ns).unwrap_or(0));

                                push_utilization_row(
                                    &mut lock_cpu_table(&tables.utilization),
                                    ts,
                                    &platform,
                                    "thread",
                                    thread.tid,
                                    &thread.comm,
                                    wall_ns,
                                    delta_user,
                                    delta_sys,
                                    thread.cputime_user_ns as i64,
                                    thread.cputime_sys_ns as i64,
                                    0,
                                    0,
                                    0,
                                    0,
                                    thread.state.as_deref().unwrap_or(""),
                                    thread.wchan.as_deref().unwrap_or(""),
                                );
                                push_tasks_row(
                                    &mut lock_cpu_table(&tables.tasks),
                                    ts,
                                    &platform,
                                    thread,
                                    wall_ns,
                                    delta_user,
                                    delta_sys,
                                );
                            }
                        }
                        state.last_threads = threads.into_iter().map(|t| (t.tid, t)).collect();
                    }
                    Err(e) => log::warn!("cpu thread sample failed: {e}"),
                }

                state.last_wall = now;
                thread::sleep(config.interval);
            }

            running.store(false, Ordering::SeqCst);
        });

        *lock_cpu_collector(&self.handle) = Some(handle);
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn stop(&self) -> Result<(), CollectorError> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }

        if let Some(handle) = lock_cpu_collector(&self.handle).take() {
            handle
                .join()
                .map_err(|_| CollectorError::StopFailed("thread join failed".into()))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn autostart_defaults_to_one_second() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("PROBING_CPU");
        std::env::remove_var("PROBING_CPU_SAMPLE_MS");
        assert_eq!(autostart_interval_ms(), Some(1000));
    }

    #[test]
    fn autostart_respects_disable_env() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::set_var("PROBING_CPU", "off");
        assert_eq!(autostart_interval_ms(), None);
        std::env::remove_var("PROBING_CPU");
    }

    #[test]
    fn autostart_creates_cpu_memtable_files() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        use probing_memtable::discover::default_dir;

        let dir = std::env::temp_dir().join(format!(
            "probing_cpu_autostart_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("PROBING_DATA_DIR", &dir);
        std::env::remove_var("PROBING_CPU");

        let _ = super::super::collector::CpuCollector::instance().stop();
        start_cpu_sampling_from_env();

        let util = default_dir()
            .join(std::process::id().to_string())
            .join("cpu.utilization");
        assert!(
            util.is_file(),
            "expected cpu.utilization at {}",
            util.display()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collector_writes_bounded_utilization_rows() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        let collector = CpuCollector::instance();
        let _ = collector.stop();

        let iterations = 100_i64;
        collector
            .start(CpuCollectorConfig {
                interval: Duration::from_millis(1),
                thread_top_n: 0,
                iterations: Some(iterations),
            })
            .expect("start collector");

        std::thread::sleep(Duration::from_secs(2));
        collector.stop().expect("stop collector");

        let rows = collector.utilization_row_count();
        assert!(
            rows >= (iterations - 1) as usize,
            "expected at least {} utilization rows, got {rows}",
            iterations - 1
        );
    }
}
