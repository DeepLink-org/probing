//! CPU profiling via `SIGPROF` sampling ("model two": trigger in-signal, process
//! off-signal).
//!
//! Raw capture and Python/native merge live in [`crate::features::stacktrace::capture`]
//! / [`crate::features::stacktrace::merge`]; this module owns the ring buffer,
//! timer, and folded output. Python frames always come from the [`super::vm`] tracer.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use core::ffi::{c_int, c_void};
use nix::libc;
use once_cell::sync::Lazy;
use serde_json::json;

use crate::features::flamegraph::{FlamegraphKind, FlamegraphOptions};
use crate::features::stacktrace::capture;
use crate::features::stacktrace::compact::CompactStack;
use crate::features::stacktrace::fingerprint;
use crate::features::stacktrace::fold::{fold_snapshot, FoldOptions};
use crate::features::stacktrace::metrics;
use crate::features::stacktrace::snapshot::{StackFlags, StackSnapshot, StackSource};

const DEFAULT_SAMPLE_FREQ: i32 = 100;
const MIN_SAMPLE_FREQ: i32 = 1;
const MAX_SAMPLE_FREQ: i32 = 100_000;

const RING_SIZE: usize = 512;
const RING_MASK: usize = RING_SIZE - 1;
const MAX_FOLDED_STACKS: usize = 1 << 17;

// ---------------------------------------------------------------------------
// Lock-free bounded MPMC ring (Vyukov). Async-signal-safe producer side.
// ---------------------------------------------------------------------------

struct Cell {
    seq: AtomicUsize,
    data: UnsafeCell<StackSnapshot>,
}

struct Ring {
    buffer: Box<[Cell]>,
    enqueue_pos: AtomicUsize,
    dequeue_pos: AtomicUsize,
}

unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    fn new() -> Ring {
        let mut v: Vec<Cell> = Vec::with_capacity(RING_SIZE);
        for i in 0..RING_SIZE {
            v.push(Cell {
                seq: AtomicUsize::new(i),
                data: UnsafeCell::new(StackSnapshot::zeroed()),
            });
        }
        Ring {
            buffer: v.into_boxed_slice(),
            enqueue_pos: AtomicUsize::new(0),
            dequeue_pos: AtomicUsize::new(0),
        }
    }

    /// Claim a ring cell and fill it in place (keeps large snapshots off the
    /// interrupted thread stack — important under `SIGPROF` + deep frames).
    ///
    /// `fill` should return `false` to abort the claim (cell is released unused).
    fn enqueue_with(&self, fill: impl FnOnce(&mut StackSnapshot) -> bool) -> bool {
        let mut pos = self.enqueue_pos.load(Ordering::Relaxed);
        loop {
            let cell = &self.buffer[pos & RING_MASK];
            let seq = cell.seq.load(Ordering::Acquire);
            let diff = seq as isize - pos as isize;
            if diff == 0 {
                if self
                    .enqueue_pos
                    .compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    let dst = unsafe { &mut *cell.data.get() };
                    if fill(dst) {
                        cell.seq.store(pos + 1, Ordering::Release);
                        return true;
                    }
                    // Release unused claim; zero in place (no StackSnapshot temporary).
                    unsafe {
                        core::ptr::write_bytes(
                            dst as *mut StackSnapshot as *mut u8,
                            0,
                            core::mem::size_of::<StackSnapshot>(),
                        );
                    }
                    cell.seq.store(pos + 1, Ordering::Release);
                    return false;
                }
            } else if diff < 0 {
                return false;
            } else {
                pos = self.enqueue_pos.load(Ordering::Relaxed);
            }
        }
    }

    fn dequeue(&self, out: &mut StackSnapshot) -> bool {
        let mut pos = self.dequeue_pos.load(Ordering::Relaxed);
        loop {
            let cell = &self.buffer[pos & RING_MASK];
            let seq = cell.seq.load(Ordering::Acquire);
            let diff = seq as isize - (pos + 1) as isize;
            if diff == 0 {
                if self
                    .dequeue_pos
                    .compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    unsafe { *out = *cell.data.get() };
                    cell.seq.store(pos + RING_SIZE, Ordering::Release);
                    return true;
                }
            } else if diff < 0 {
                return false;
            } else {
                pos = self.dequeue_pos.load(Ordering::Relaxed);
            }
        }
    }
}

static RING_PTR: AtomicPtr<Ring> = AtomicPtr::new(std::ptr::null_mut());
static SAMPLER_ENABLED: AtomicBool = AtomicBool::new(false);
static HANDLER_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static PPROF_OWNS_TRACER: AtomicBool = AtomicBool::new(false);
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

struct ActiveGuard;
impl ActiveGuard {
    #[inline]
    fn new() -> Self {
        HANDLER_ACTIVE.fetch_add(1, Ordering::Acquire);
        ActiveGuard
    }
}
impl Drop for ActiveGuard {
    #[inline]
    fn drop(&mut self) {
        HANDLER_ACTIVE.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(unix)]
unsafe extern "C" fn sigprof_handler(_sig: c_int, _info: *mut libc::siginfo_t, uctx: *mut c_void) {
    if !SAMPLER_ENABLED.load(Ordering::Acquire) {
        return;
    }
    let _active = ActiveGuard::new();
    if !capture::on_signal_altstack() {
        // No per-thread alt stack — do nothing rather than smash the training stack.
        return;
    }
    let ring = RING_PTR.load(Ordering::Acquire);
    if ring.is_null() {
        return;
    }
    let ring = &*ring;

    let opts = capture::FillOpts {
        walk_native: true,
    };
    let mut filled = false;
    if ring.enqueue_with(|dst| {
        filled = true;
        capture::fill_raw_snapshot_with(dst, uctx, opts);
        dst.source = StackSource::Sigprof;
        if dst.is_empty() {
            return false;
        }
        capture::store_latest_snapshot(dst);
        true
    }) {
        return;
    }
    if filled {
        return;
    }

    if capture::fill_latest_from_uctx_with(uctx, StackSource::Sigprof, opts) {
        metrics::inc_dropped_ring();
    }
}

#[cfg(unix)]
fn install_handler() {
    if HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    capture::ensure_signal_altstack();
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigprof_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART | libc::SA_ONSTACK;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaddset(&mut sa.sa_mask, libc::SIGUSR2);
        libc::sigaction(libc::SIGPROF, &sa, std::ptr::null_mut());
    }
}

#[cfg(not(unix))]
fn install_handler() {}

#[cfg(unix)]
fn arm_timer(freq: i32) {
    let period_us = (1_000_000i64 / freq as i64).max(1);
    let tv = libc::timeval {
        tv_sec: (period_us / 1_000_000) as libc::time_t,
        tv_usec: (period_us % 1_000_000) as libc::suseconds_t,
    };
    let it = libc::itimerval {
        it_interval: tv,
        it_value: tv,
    };
    unsafe { libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut()) };
}

#[cfg(not(unix))]
fn arm_timer(_freq: i32) {}

#[cfg(unix)]
fn disarm_timer() {
    let it: libc::itimerval = unsafe { std::mem::zeroed() };
    unsafe { libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut()) };
}

#[cfg(not(unix))]
fn disarm_timer() {}

/// Aggregated bucket: count + one compact representative for later fold.
///
/// Fingerprint includes `tid`; this consumer still **filters to the Python main
/// OS tid only** so distributed main-thread flamegraphs stay clean.
struct AggregatedSample {
    count: u64,
    representative: CompactStack,
}

struct SamplerState {
    generation: AtomicU64,
    samples: Mutex<HashMap<u64, AggregatedSample>>,
}

static SAMPLER: Lazy<SamplerState> = Lazy::new(|| SamplerState {
    generation: AtomicU64::new(0),
    samples: Mutex::new(HashMap::new()),
});

/// Keep only samples from the registered Python main OS tid.
///
/// When the main tid is unknown, drop the sample — otherwise worker threads can
/// leak into the distributed main-thread flamegraph.
fn accepts_main_thread_sample(sample_tid: u64, main_tid: Option<u64>) -> bool {
    match main_tid {
        Some(main) => sample_tid == main,
        None => false,
    }
}

fn process_sample(s: &StackSnapshot) {
    if s.flags.contains(StackFlags::PY_TORN) {
        metrics::inc_dropped_torn();
        return;
    }
    // Contract: only Python main OS tid enters the CPU flamegraph map.
    if !accepts_main_thread_sample(s.tid, capture::python_main_os_tid()) {
        metrics::inc_dropped_not_main();
        return;
    }
    if s.is_empty() {
        return;
    }
    let fp = fingerprint::fingerprint(s);
    if let Ok(mut map) = SAMPLER.samples.lock() {
        if let Some(entry) = map.get_mut(&fp) {
            entry.count = entry.count.saturating_add(1);
            metrics::inc_fingerprint_hit();
        } else if map.len() < MAX_FOLDED_STACKS {
            map.insert(
                fp,
                AggregatedSample {
                    count: 1,
                    representative: CompactStack::from_snapshot(s),
                },
            );
            metrics::inc_fingerprint_miss();
        } else {
            metrics::inc_dropped_capacity();
        }
    }
}

fn consumer_loop(my_gen: u64) {
    let mut sample = StackSnapshot::zeroed();
    loop {
        let stopping = SAMPLER.generation.load(Ordering::SeqCst) != my_gen;
        let ring = RING_PTR.load(Ordering::Acquire);
        let mut drained = false;
        if !ring.is_null() {
            let ring = unsafe { &*ring };
            while ring.dequeue(&mut sample) {
                drained = true;
                process_sample(&sample);
            }
        }
        if stopping {
            break;
        }
        if !drained {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

pub fn is_sampling_active() -> bool {
    SAMPLER_ENABLED.load(Ordering::Acquire)
}

pub fn setup(freq: u64) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = freq;
        return Err(anyhow::anyhow!(
            "CPU profiling (SIGPROF) is not supported on this platform"
        ));
    }
    #[cfg(unix)]
    setup_unix(freq)
}

#[cfg(unix)]
fn setup_unix(freq: u64) -> Result<()> {
    let freq = if freq == 0 {
        DEFAULT_SAMPLE_FREQ
    } else {
        (freq as i32).clamp(MIN_SAMPLE_FREQ, MAX_SAMPLE_FREQ)
    };

    if RING_PTR.load(Ordering::Acquire).is_null() {
        let ptr = Box::into_raw(Box::new(Ring::new()));
        if RING_PTR
            .compare_exchange(
                std::ptr::null_mut(),
                ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            unsafe { drop(Box::from_raw(ptr)) };
        }
    }

    if let Ok(mut map) = SAMPLER.samples.lock() {
        map.clear();
    }
    metrics::reset_sampler_counters();

    crate::features::stacktrace::tracers::vm::initialize_globals();
    pyo3::Python::attach(|_py| {
        let already_on = crate::features::stacktrace::tracers::vm::is_tracer_enabled();
        match crate::features::stacktrace::tracers::vm::enable_tracer() {
            Ok(()) => {
                if !already_on {
                    PPROF_OWNS_TRACER.store(true, Ordering::Release);
                }
            }
            Err(e) => log::warn!(
                "probing: pprof could not enable the Python eval tracer ({e}); \
                 stacks will be native-only"
            ),
        }
    });

    install_handler();

    let my_gen = SAMPLER.generation.fetch_add(1, Ordering::SeqCst) + 1;
    capture::set_pprof_sampling_active(true);
    SAMPLER_ENABLED.store(true, Ordering::Release);

    thread::Builder::new()
        .name("probing-sampler".into())
        .spawn(move || consumer_loop(my_gen))
        .context("failed to spawn sampler consumer thread")?;

    arm_timer(freq);
    log::info!("probing: SIGPROF CPU sampler started ({freq} Hz, Python+native)");
    Ok(())
}

pub fn reset() {
    disarm_timer();
    capture::set_pprof_sampling_active(false);
    SAMPLER_ENABLED.store(false, Ordering::Release);
    SAMPLER.generation.fetch_add(1, Ordering::SeqCst);

    let ring = RING_PTR.swap(std::ptr::null_mut(), Ordering::AcqRel);
    if !ring.is_null() {
        let mut drained = false;
        for _ in 0..10_000_000 {
            if HANDLER_ACTIVE.load(Ordering::Acquire) == 0 {
                drained = true;
                break;
            }
            std::hint::spin_loop();
        }
        if drained {
            unsafe { drop(Box::from_raw(ring)) };
        } else {
            RING_PTR.store(ring, Ordering::Release);
        }
    }

    if PPROF_OWNS_TRACER.swap(false, Ordering::AcqRel) {
        pyo3::Python::attach(|_py| {
            let _ = crate::features::stacktrace::tracers::vm::disable_tracer();
        });
    }

    capture::clear_py_symbols();
}

pub fn pprof_handler() {
    let _ = setup(DEFAULT_SAMPLE_FREQ as u64);
}

fn pprof_flamegraph_options() -> FlamegraphOptions {
    FlamegraphOptions {
        title: "CPU sampling".to_string(),
        count_name: "samples".to_string(),
        kind: FlamegraphKind::Classic,
        subtitle: "SIGPROF weighted stack samples".to_string(),
        metric: None,
        profile: Some("cpu-stack".to_string()),
    }
}

fn folded_lines_from_sampler() -> Vec<String> {
    let buckets = match SAMPLER.samples.lock() {
        Ok(map) => map
            .values()
            .map(|e| (e.representative.clone(), e.count))
            .collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    let mut cache = HashMap::new();
    let mut lines = Vec::with_capacity(buckets.len());
    for (compact, count) in buckets {
        let snap = compact.to_snapshot();
        let folded = fold_snapshot(
            &snap,
            &mut cache,
            &FoldOptions {
                thread_prefix: true,
                canonicalize: true,
                count,
            },
        );
        if !folded.is_empty() {
            lines.push(folded.to_folded_line());
        }
    }
    lines
}

/// Local folded stacks for Distributed / pprof export.
///
/// When `sample_freq` is on, **only** return aggregated SIGPROF buckets — never
/// fall back to on-demand `SIGUSR2` / one-shot PYSTACKS (empty buckets → empty
/// flamegraph until samples arrive). That fallback was racing training and
/// resuming as `SIGILL` on Darwin when refreshing Distributed stacks.
fn local_folded_lines() -> Vec<String> {
    if is_sampling_active() {
        return folded_lines_from_sampler();
    }
    crate::features::stacktrace::tracers::dynamic::main_thread_on_demand_folded_lines()
}

fn folded_lines() -> Vec<String> {
    local_folded_lines()
}

/// Snapshot of local SIGPROF folded stacks (`"path count"` per line).
pub fn folded_lines_snapshot() -> Vec<String> {
    folded_lines()
}

/// Build distributed stack flamegraph JSON from attributed folded lines across ranks.
pub fn distributed_stack_flamegraph_json(
    lines: &[crate::features::flamegraph::AttributedFoldedLine],
    rank_count: usize,
    nodes_failed: &[String],
    mode: &str,
) -> String {
    let python_only = mode == "py";
    let profile = if python_only {
        "cpu-stack-distributed-py"
    } else {
        "cpu-stack-distributed"
    };
    let dropped = metrics::dropped_ring();
    let empty = |msg: &str| {
        json!({
            "profile": profile,
            "title": if python_only { "Distributed Python stacks" } else { "Distributed CPU stacks" },
            "subtitle": format!(
                "SIGPROF mixed-mode stacks · {rank_count} ranks · merge identical paths"
            ),
            "countName": "samples",
            "total": 0,
            "width": 1400.0,
            "frameHeight": 32.0,
            "frames": [],
            "dropped": dropped,
            "emptyMessage": msg,
            "nodesFailed": nodes_failed,
            "rankCount": rank_count,
        })
        .to_string()
    };

    if lines.is_empty() {
        return empty(if python_only {
            "no Python stack samples yet; set probing.pprof.sample_freq=50 on each rank (Distributed refresh no longer SIGUSR2s the main thread)"
        } else {
            "no mixed CPU stacks yet; set probing.pprof.sample_freq=50 on each rank — without SIGPROF only PYSTACKS (Python-only) is available"
        });
    }

    let subtitle = if python_only {
        if is_sampling_active() {
            format!("Python frames only · SIGPROF main-thread samples · {rank_count} ranks merged")
        } else {
            format!("Python frames only · one-shot PYSTACKS snapshot · {rank_count} ranks merged")
        }
    } else if is_sampling_active() {
        format!("Full mixed stack · SIGPROF main-thread samples · {rank_count} ranks merged")
    } else {
        format!(
            "Full mixed stack · one-shot main-thread snapshot (enable SIGPROF for accumulation) · {rank_count} ranks merged"
        )
    };
    let opts = FlamegraphOptions {
        title: if python_only {
            "Distributed Python stacks".to_string()
        } else {
            "Distributed CPU stacks".to_string()
        },
        count_name: "samples".to_string(),
        kind: FlamegraphKind::Classic,
        subtitle,
        metric: None,
        profile: Some(profile.to_string()),
    };

    match crate::features::flamegraph::Flamegraph::from_attributed_folded_lines(lines) {
        Some(fg) => {
            let payload = fg.json_payload(&opts);
            match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(mut v) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("dropped".to_string(), json!(dropped));
                        obj.insert("rankCount".to_string(), json!(rank_count));
                        if !nodes_failed.is_empty() {
                            obj.insert("nodesFailed".to_string(), json!(nodes_failed));
                        }
                    }
                    v.to_string()
                }
                Err(_) => payload,
            }
        }
        None => empty("no valid folded stacks after merge"),
    }
}

fn local_training_rank() -> Option<i32> {
    std::env::var("RANK")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .or_else(|| {
            std::env::var("LOCAL_RANK")
                .ok()
                .and_then(|s| s.trim().parse().ok())
        })
}

fn remote_pprof_folded_lines_blocking(addr: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("http://{addr}/apis/pprofextension/flamegraph/folded/json");
    let timeout = std::time::Duration::from_secs(
        std::env::var("PROBING_CLUSTER_QUERY_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10),
    );
    let text = ureq::get(&url)
        .config()
        .timeout_global(Some(timeout))
        .build()
        .call()?
        .body_mut()
        .read_to_string()?;
    #[derive(serde::Deserialize)]
    struct Payload {
        lines: Vec<String>,
    }
    Ok(serde_json::from_str::<Payload>(&text)?.lines)
}

fn remote_pprof_flamegraph_json_blocking(addr: &str) -> anyhow::Result<String> {
    let url = format!("http://{addr}/apis/pprofextension/flamegraph/json");
    let timeout = std::time::Duration::from_secs(
        std::env::var("PROBING_CLUSTER_QUERY_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10),
    );
    let text = ureq::get(&url)
        .config()
        .timeout_global(Some(timeout))
        .build()
        .call()?
        .body_mut()
        .read_to_string()?;
    Ok(text)
}

fn remote_pprof_folded_lines_fallback(addr: &str) -> anyhow::Result<Vec<String>> {
    remote_pprof_folded_lines_blocking(addr).or_else(|_| {
        let json = remote_pprof_flamegraph_json_blocking(addr)?;
        Ok(crate::features::flamegraph::folded_lines_from_flamegraph_json(&json))
    })
}

/// Cluster fan-out + merge for distributed CPU stack flamegraph JSON.
///
/// `mode`: `mixed` (Python + native) or `py` (Python frames only).
///
/// Returns `(json_body, partial)` where `partial` is true when some peers failed.
pub async fn collect_distributed_stack_flamegraph_json(
    cluster: bool,
    mode: &str,
) -> (String, bool) {
    let mode = if mode == "py" { "py" } else { "mixed" };
    use probing_core::core::cluster::{get_nodes, is_node_alive, local_listen_addrs};

    let mut line_sets: Vec<(Option<i32>, Vec<String>)> =
        vec![(local_training_rank(), folded_lines_snapshot())];
    let mut nodes_failed = Vec::new();
    let mut rank_count = 1usize;

    if cluster {
        let local_addrs = local_listen_addrs();
        let peers: Vec<_> = get_nodes()
            .into_iter()
            .filter(is_node_alive)
            .filter(|node| !local_addrs.iter().any(|local| local == &node.addr))
            .collect();

        for node in peers {
            let addr = node.addr.clone();
            let peer_rank = node.rank;
            match tokio::task::spawn_blocking(move || remote_pprof_folded_lines_fallback(&addr))
                .await
            {
                Ok(Ok(lines)) => {
                    line_sets.push((peer_rank, lines));
                    rank_count += 1;
                }
                Ok(Err(err)) => {
                    log::warn!(
                        "distributed stack flamegraph fan-out {} failed: {err:#}",
                        node.addr
                    );
                    nodes_failed.push(format!("{}: {err:#}", node.addr));
                }
                Err(err) => {
                    nodes_failed.push(format!("{}: task join failed: {err}", node.addr));
                }
            }
        }
    }

    let mut merged = crate::features::flamegraph::merge_distributed_stack_attributed(&line_sets);
    if mode == "py" {
        merged = crate::features::flamegraph::filter_attributed_folded_lines_python_only(&merged);
    }
    let body = distributed_stack_flamegraph_json(&merged, rank_count, &nodes_failed, mode);
    (body, !nodes_failed.is_empty())
}

/// Raw folded stack lines for cluster fan-out (`{"lines":["stack count", ...]}`).
pub fn folded_lines_json() -> String {
    json!({ "lines": folded_lines_snapshot() }).to_string()
}

pub fn flamegraph() -> Result<String> {
    let lines = folded_lines();
    if lines.is_empty() {
        return Err(anyhow!(
            "no samples collected yet; enable CPU sampling and let it run"
        ));
    }

    let dropped = metrics::dropped_ring();
    if dropped > 0 {
        log::warn!("probing: {dropped} CPU samples dropped (ring full or cardinality cap)");
    }

    let fg = crate::features::flamegraph::Flamegraph::from_folded_lines(&lines)
        .ok_or_else(|| anyhow!("no valid folded stacks"))?;
    Ok(fg.render_html(&pprof_flamegraph_options()))
}

pub fn flamegraph_json() -> String {
    let dropped = metrics::dropped_ring();
    let empty = |msg: String| {
        json!({
            "profile": "cpu-stack",
            "title": "CPU sampling",
            "subtitle": "SIGPROF weighted stack samples",
            "countName": "samples",
            "total": 0,
            "width": 1400.0,
            "frameHeight": 32.0,
            "frames": [],
            "dropped": dropped,
            "metrics": metrics::snapshot_json(),
            "emptyMessage": msg,
        })
        .to_string()
    };

    let lines = folded_lines();
    if lines.is_empty() {
        return empty(
            "no samples collected yet; enable CPU sampling or use Stacks → Default".to_string(),
        );
    }

    match crate::features::flamegraph::Flamegraph::from_folded_lines(&lines) {
        Some(fg) => {
            let payload = fg.json_payload(&pprof_flamegraph_options());
            match serde_json::from_str::<serde_json::Value>(&payload) {
                Ok(mut v) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("dropped".to_string(), json!(dropped));
                        obj.insert("metrics".to_string(), metrics::snapshot_json());
                    }
                    v.to_string()
                }
                Err(_) => payload,
            }
        }
        None => empty("no valid folded stacks".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::stacktrace::metrics;
    use crate::features::stacktrace::snapshot::{StackFlags, StackSnapshot, StackSource};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Serialize sampler-map mutation tests (non-signal).
    static SAMPLER_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_sampler_lock<R>(f: impl FnOnce() -> R) -> R {
        let _g = SAMPLER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        f()
    }

    fn clear_sampler_buckets() {
        if let Ok(mut map) = SAMPLER.samples.lock() {
            map.clear();
        }
        metrics::reset_sampler_counters();
    }

    #[cfg(unix)]
    fn ensure_ring() {
        if RING_PTR.load(Ordering::Acquire).is_null() {
            let ptr = Box::into_raw(Box::new(Ring::new()));
            let _ = RING_PTR.compare_exchange(
                std::ptr::null_mut(),
                ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            if RING_PTR.load(Ordering::Acquire) != ptr {
                unsafe { drop(Box::from_raw(ptr)) };
            }
        }
    }

    #[test]
    fn accepts_only_registered_python_main_tid() {
        assert!(accepts_main_thread_sample(10, Some(10)));
        assert!(!accepts_main_thread_sample(11, Some(10)));
        // Unknown main tid: drop sample so worker threads cannot pollute flamegraph.
        assert!(!accepts_main_thread_sample(10, None));
    }

    #[test]
    fn fingerprint_hit_defers_fold_until_export() {
        with_sampler_lock(|| {
            capture::register_main_os_tid();
            let main = capture::python_main_os_tid().expect("main tid");
            clear_sampler_buckets();

            let snap = StackSnapshot::from_parts(
                main,
                StackSource::Sigprof,
                &[0x1111_aaa0, 0x2222_bbb0],
                &[],
                StackFlags::PY_ABSENT,
            );
            process_sample(&snap);
            process_sample(&snap);
            process_sample(&snap);

            assert_eq!(metrics::fingerprint_misses(), 1);
            assert_eq!(metrics::fingerprint_hits(), 2);
            assert_eq!(
                metrics::fold_calls(),
                0,
                "lazy pipeline: fold must wait for export"
            );

            let folds_before = metrics::fold_calls();
            let lines = folded_lines_from_sampler();
            assert_eq!(metrics::fold_calls(), folds_before + 1);
            assert_eq!(lines.len(), 1);
            assert!(
                lines[0].ends_with(" 3"),
                "aggregated count should be 3, got {:?}",
                lines[0]
            );
        });
    }

    #[test]
    fn non_main_tid_increments_dropped_not_main() {
        with_sampler_lock(|| {
            capture::register_main_os_tid();
            let main = capture::python_main_os_tid().expect("main tid");
            clear_sampler_buckets();
            let before = metrics::dropped_not_main();

            let snap = StackSnapshot::from_parts(
                main.wrapping_add(9_001),
                StackSource::Sigprof,
                &[0x10],
                &[],
                StackFlags::PY_ABSENT,
            );
            process_sample(&snap);
            assert!(metrics::dropped_not_main() > before);
            assert_eq!(metrics::fingerprint_misses(), 0);
            assert!(SAMPLER.samples.lock().unwrap().is_empty());
        });
    }

    /// Real `SIGPROF` → handler → `latest` slot with a native PC.
    #[cfg(unix)]
    #[test]
    fn sigprof_signal_publishes_latest_native_pc() {
        capture::with_signal_test_lock(|| {
            ensure_ring();
            install_handler();
            capture::register_python_thread();
            capture::register_main_os_tid();
            let tid = capture::current_tid();

            SAMPLER_ENABLED.store(true, Ordering::Release);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    unsafe {
                        libc::raise(libc::SIGPROF);
                    }
                    if let Some((snap, _)) = capture::latest_snapshot_with_seq(tid) {
                        if snap.source == StackSource::Sigprof && snap.native_len >= 1 {
                            return;
                        }
                    }
                    if Instant::now() >= deadline {
                        panic!(
                            "SIGPROF handler did not publish a native PC on tid={tid} within timeout"
                        );
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            }));
            SAMPLER_ENABLED.store(false, Ordering::Release);
            result.expect("sigprof integration");
        });
    }

    #[test]
    fn sampling_active_folded_export_skips_on_demand_fallback() {
        with_sampler_lock(|| {
            clear_sampler_buckets();
            SAMPLER_ENABLED.store(true, Ordering::Release);
            let lines = local_folded_lines();
            SAMPLER_ENABLED.store(false, Ordering::Release);
            assert!(
                lines.is_empty(),
                "with sample_freq on and empty buckets, must not invent on-demand lines: {lines:?}"
            );
        });
    }

    /// Ring survives a real SIGPROF enqueue and consumer-side dequeue.
    #[cfg(unix)]
    #[test]
    fn sigprof_ring_enqueue_from_handler_is_dequeued() {
        capture::with_signal_test_lock(|| {
            ensure_ring();
            install_handler();
            capture::register_python_thread();
            capture::register_main_os_tid();

            let ring = unsafe { &*RING_PTR.load(Ordering::Acquire) };
            let mut junk = StackSnapshot::zeroed();
            while ring.dequeue(&mut junk) {}

            SAMPLER_ENABLED.store(true, Ordering::Release);
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut got = StackSnapshot::zeroed();
            let mut ok = false;
            while Instant::now() < deadline {
                unsafe {
                    libc::raise(libc::SIGPROF);
                }
                if ring.dequeue(&mut got)
                    && got.source == StackSource::Sigprof
                    && got.native_len >= 1
                {
                    ok = true;
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
            SAMPLER_ENABLED.store(false, Ordering::Release);
            assert!(ok, "expected SIGPROF sample on ring with native PC");
        });
    }
}
