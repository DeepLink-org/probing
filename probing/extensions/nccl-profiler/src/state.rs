//! Plugin runtime: slot pools, event hierarchy, batch flush.
//!
//! # Timing model (follows the official NCCL ext-profiler guidance)
//!
//! NCCL's coll/p2p `stopEvent` only marks the end of the **host-side
//! enqueue** — the GPU kernel and proxy threads keep working after it. Real
//! execution time is reconstructed by reference-counting child events, as
//! recommended by NCCL's ext-profiler docs:
//!
//! * every child (`ProxyOp`, `KernelCh`) bumps `live_children` on start and
//!   drops it on stop, folding its `[start, stop]` window into the parent;
//! * the coll completes when it is stopped *and* `live_children == 0`;
//! * `exec_time_ns` then prefers the kernel-channel window (NCCL's own
//!   signal for kernel activity), falling back to the proxy window, then to
//!   the enqueue window (`timing_source` records which one was used).
//!
//! # Locking
//!
//! Slot pools are sharded by communicator hash (see [`crate::shard`]); each
//! shard has its own mutex so concurrent NCCL callbacks on different comms
//! rarely contend. The mmap [`NcclWriter`] keeps a separate lock; completed
//! rows are collected under a shard lock and written after it is released.

use std::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;

use parking_lot::Mutex;

use once_cell::sync::Lazy;

use crate::abi::net_ib_v1::{net_plugin_type, read_ib_qp, NCCL_PROFILER_NET_TYPE_IB};
use crate::abi::{
    NcclProfilerEventDescrV3, NcclProfilerEventDescrV4, NcclProfilerEventState, NCCL_PROFILE_COLL,
    NCCL_PROFILE_KERNEL_CH, NCCL_PROFILE_NET_PLUGIN, NCCL_PROFILE_P2P, NCCL_PROFILE_PROXY_OP,
    NCCL_PROFILE_PROXY_STEP,
};
use crate::events::{
    copy_func_name, dtype_bytes, event_type, now_ns, proxy_step_state_index, CollContext,
    CollPerfMeta, CollSlot, CompletedCollPerf, CompletedProxyOp, EventCounters, InflightOp,
    KernelChSlot, NetPluginSlot, ProfilerCounterSnapshot, ProxyOpData, ProxyOpSlot, ProxyStepData,
    ProxyStepSlot, ShortName, EVT_COLL, EVT_KERNEL_CH, EVT_NET_PLUGIN, EVT_PROXY_OP,
    EVT_PROXY_STEP,
};
use crate::pool::{Indexed, SlotPool, INVALID_IDX};
use crate::pool_config::{per_shard_limits, pool_limits, PoolLimits};
use crate::pool_pressure::{check_pool_usage, warn_pool_exhausted};
use crate::shard::{pack_slot_id, pool_shard_count, shard_for_comm, shard_for_handle};
use crate::writer::{CompletedNetQp, NcclWriter};

/// Per-communicator metadata from the v4 `init` callback (empty on v3).
#[derive(Clone, Copy, Debug)]
pub struct CommInfo {
    pub comm_hash: u64,
    pub n_ranks: i32,
}

impl Default for CommInfo {
    fn default() -> Self {
        Self {
            comm_hash: 0,
            n_ranks: -1,
        }
    }
}

/// ABI-version-independent view of a `startEvent` descriptor. The v3/v4
/// callbacks parse their own descriptor layout into this, keeping the slot
/// pools free of version-specific code.
pub enum ParsedEvent {
    /// Collective or P2P host call (`meta.is_p2p` distinguishes them).
    Coll {
        ctx: CollContext,
        meta: CollPerfMeta,
    },
    ProxyOp {
        parent_obj: *mut c_void,
        rank: i32,
        channel_id: i32,
        peer: i32,
        n_steps: i32,
        is_send: i32,
    },
    ProxyStep {
        parent_obj: *mut c_void,
        step: i32,
    },
    KernelCh {
        parent_obj: *mut c_void,
        channel_id: i32,
        /// GPU globaltimer at kernel start (v4 `pTimer`; 0 on v3).
        gpu_start_ptimer: u64,
    },
    NetPlugin {
        rank: i32,
        id: i64,
        data: *mut c_void,
    },
    Ignored,
}

/// ABI-version-independent `recordEventState` payload.
#[derive(Clone, Copy, Debug)]
pub enum StateArgs {
    None,
    /// v3: transferred bytes + step count reported on the proxy-op event.
    ProxyOpV3 {
        trans_size: u64,
        steps: i32,
    },
    /// v4: this step's transferred bytes, captured only at SendWait /
    /// RecvFlushWait where NCCL has just assigned it (other states carry a
    /// stale value from another in-flight step).
    ProxyStepV4 {
        trans_size: u64,
    },
    /// v4 `KernelChStop`: GPU globaltimer stop timestamp.
    KernelChStop {
        p_timer: u64,
    },
}

/// Ops smaller than this (bytes) are not recorded (0 = record everything).
/// Mirrors NCCL Inspector's `NCCL_INSPECTOR_DUMP_MIN_SIZE_BYTES` / CoMMA's
/// `NCCL_PROFILER_SMALL_MSG_THRESHOLD`.
static MIN_MSG_BYTES: Lazy<u64> = Lazy::new(|| {
    std::env::var("PROBING_NCCL_MIN_MSG_BYTES")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
});

/// Rows produced under the pools lock, written after it is released.
#[derive(Default)]
pub struct FlushBatch {
    coll_perf: Vec<CompletedCollPerf>,
    proxy_ops: Vec<CompletedProxyOp>,
    net_qp: Vec<CompletedNetQp>,
}

impl FlushBatch {
    fn is_empty(&self) -> bool {
        self.coll_perf.is_empty() && self.proxy_ops.is_empty() && self.net_qp.is_empty()
    }

    fn write(&self, writer: &mut NcclWriter, counters: &EventCounters) {
        for row in &self.coll_perf {
            writer.append_coll_perf(row, counters);
        }
        writer.flush_proxy_ops(&self.proxy_ops, counters);
        for row in &self.net_qp {
            writer.append_net_qp(row, counters);
        }
    }
}

/// Partitioned slot pools — one mutex per shard (by comm hash).
struct ShardedPools {
    shards: Vec<Mutex<Pools>>,
}

impl ShardedPools {
    fn new() -> Self {
        let n = pool_shard_count();
        let total = pool_limits();
        let per = per_shard_limits(total, n);
        Self {
            shards: (0..n).map(|_| Mutex::new(Pools::new(per))).collect(),
        }
    }

    fn start_event(&self, handle: *mut *mut c_void, event: ParsedEvent, counters: &EventCounters) {
        let shard = shard_for_start(&event);
        self.shards[shard]
            .lock()
            .start_event(handle, event, counters, shard);
    }

    fn stop_event(&self, handle: *mut c_void, counters: &EventCounters) -> FlushBatch {
        let shard = shard_for_handle(handle).unwrap_or(0);
        self.shards[shard].lock().stop_event(handle, counters)
    }

    fn record_state(&self, handle: *mut c_void, state: NcclProfilerEventState, args: StateArgs) {
        let shard = shard_for_handle(handle).unwrap_or(0);
        self.shards[shard].lock().record_state(handle, state, args);
    }

    /// Watchdog path: skip shards held by NCCL callbacks so we never block comm.
    fn try_collect_inflight(&self, min_age_ns: i64) -> (Vec<InflightOp>, FlushBatch) {
        let mut rows = Vec::new();
        let mut batch = FlushBatch::default();
        for shard in &self.shards {
            let Some(mut pools) = shard.try_lock() else {
                continue;
            };
            let (partial_rows, partial_batch) = pools.collect_inflight(min_age_ns);
            rows.extend(partial_rows);
            batch.coll_perf.extend(partial_batch.coll_perf);
            batch.proxy_ops.extend(partial_batch.proxy_ops);
            batch.net_qp.extend(partial_batch.net_qp);
        }
        (rows, batch)
    }

    fn drain_all_pending(&self) -> FlushBatch {
        let mut batch = FlushBatch::default();
        for shard in &self.shards {
            let partial = shard.lock().drain_pending();
            batch.coll_perf.extend(partial.coll_perf);
            batch.proxy_ops.extend(partial.proxy_ops);
            batch.net_qp.extend(partial.net_qp);
        }
        batch
    }

    fn counter_snapshot(&self, counters: &EventCounters) -> ProfilerCounterSnapshot {
        let Some(first) = self.shards.first() else {
            return ProfilerCounterSnapshot::default();
        };
        let mut snap = first.lock().counter_snapshot(counters);
        for shard in self.shards.iter().skip(1) {
            let p = shard.lock().counter_snapshot(counters);
            snap.coll_live += p.coll_live;
            snap.proxy_live += p.proxy_live;
            snap.step_live += p.step_live;
            snap.kch_live += p.kch_live;
            snap.net_live += p.net_live;
            snap.coll_cap += p.coll_cap;
            snap.proxy_cap += p.proxy_cap;
            snap.step_cap += p.step_cap;
            snap.kch_cap += p.kch_cap;
            snap.net_cap += p.net_cap;
        }
        snap
    }
}

fn shard_for_start(event: &ParsedEvent) -> usize {
    match event {
        ParsedEvent::Coll { ctx, .. } => shard_for_comm(ctx.comm_hash),
        ParsedEvent::ProxyOp {
            parent_obj, rank, ..
        } => shard_for_handle(*parent_obj).unwrap_or_else(|| shard_for_comm(*rank as u64)),
        ParsedEvent::ProxyStep { parent_obj, .. } | ParsedEvent::KernelCh { parent_obj, .. } => {
            shard_for_handle(*parent_obj).unwrap_or(0)
        }
        ParsedEvent::NetPlugin { rank, .. } => shard_for_comm(*rank as u64),
        ParsedEvent::Ignored => 0,
    }
}

pub struct PluginState {
    pools: ShardedPools,
    writer: Mutex<NcclWriter>,
    pub counters: EventCounters,
}

pub struct Pools {
    pub coll_pool: SlotPool<CollSlot>,
    pub proxy_pool: SlotPool<ProxyOpSlot>,
    pub step_pool: SlotPool<ProxyStepSlot>,
    pub kch_pool: SlotPool<KernelChSlot>,
    pub net_pool: SlotPool<NetPluginSlot>,
}

impl Default for PluginState {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginState {
    pub fn new() -> Self {
        Self {
            pools: ShardedPools::new(),
            writer: Mutex::new(NcclWriter::new()),
            counters: EventCounters::new(),
        }
    }

    /// v3 entry point: parse the v3 descriptor, then track.
    pub fn start_event(&self, handle: *mut *mut c_void, descr: &NcclProfilerEventDescrV3) {
        self.start_parsed(handle, parse_descr_v3(descr));
    }

    /// v4 entry point: parse the v4 descriptor with communicator metadata.
    pub fn start_event_v4(
        &self,
        handle: *mut *mut c_void,
        descr: &NcclProfilerEventDescrV4,
        comm: CommInfo,
    ) {
        self.start_parsed(handle, parse_descr_v4(descr, comm));
    }

    pub fn start_parsed(&self, handle: *mut *mut c_void, event: ParsedEvent) {
        self.pools.start_event(handle, event, &self.counters);
    }

    pub fn stop_event(&self, handle: *mut c_void) {
        let batch = self.pools.stop_event(handle, &self.counters);
        self.flush(&batch);
    }

    pub fn record_state(
        &self,
        handle: *mut c_void,
        state: NcclProfilerEventState,
        args: StateArgs,
    ) {
        self.pools.record_state(handle, state, args);
    }

    pub fn snapshot_inflight(&self, min_age_ns: i64) -> usize {
        let (rows, batch) = self.pools.try_collect_inflight(min_age_ns);
        {
            let mut writer = self.writer.lock();
            for row in &rows {
                writer.append_inflight(row, &self.counters);
            }
            batch.write(&mut writer, &self.counters);
        }
        self.publish_profiler_counters();
        rows.len()
    }

    pub fn finalize_flush(&self) {
        let batch = self.pools.drain_all_pending();
        self.flush(&batch);
        self.publish_profiler_counters();
    }

    /// Append a counter/pool-usage snapshot row to `nccl.profiler_counters`.
    pub fn publish_profiler_counters(&self) {
        let snapshot = self.pools.counter_snapshot(&self.counters);
        self.writer
            .lock()
            .append_profiler_counters(&snapshot, &self.counters);
    }

    fn flush(&self, batch: &FlushBatch) {
        if batch.is_empty() {
            return;
        }
        batch.write(&mut self.writer.lock(), &self.counters);
    }
}

fn cstr_opt(p: *const c_char) -> Option<&'static str> {
    if p.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(p) }.to_str().ok()
}

#[inline]
fn passes_filter(bytes: u64) -> bool {
    let min = *MIN_MSG_BYTES;
    min == 0 || bytes >= min
}

#[allow(clippy::too_many_arguments)]
fn build_coll_context(
    rank: i32,
    comm_hash: u64,
    n_ranks: i32,
    seq: u64,
    func: Option<&str>,
) -> CollContext {
    let mut ctx = CollContext {
        rank,
        comm_hash,
        n_ranks,
        seq,
        ..Default::default()
    };
    copy_func_name(&mut ctx, func);
    ctx
}

/// Parse a v3 descriptor (comm metadata lives in the coll/p2p body).
pub fn parse_descr_v3(descr: &NcclProfilerEventDescrV3) -> ParsedEvent {
    let t = descr.type_ as i32;
    match t {
        x if x == NCCL_PROFILE_COLL => {
            let c = unsafe { descr.body.coll };
            let dtype_name = cstr_opt(c.datatype);
            ParsedEvent::Coll {
                ctx: build_coll_context(
                    descr.rank,
                    c.comm_hash,
                    -1,
                    c.seq_number,
                    cstr_opt(c.func),
                ),
                meta: CollPerfMeta {
                    is_p2p: false,
                    peer: -1,
                    count: c.count as u64,
                    msg_bytes: c.count as u64 * dtype_bytes(dtype_name),
                    dtype: ShortName::from_str(dtype_name),
                    algo: ShortName::from_str(cstr_opt(c.algo)),
                    proto: ShortName::from_str(cstr_opt(c.proto)),
                    n_channels: c.n_max_channels as i32,
                },
            }
        }
        x if x == NCCL_PROFILE_P2P => {
            let p = unsafe { descr.body.p2p };
            let dtype_name = cstr_opt(p.datatype);
            ParsedEvent::Coll {
                // v3 P2P descriptor carries no sequence number
                ctx: build_coll_context(descr.rank, p.comm_hash, -1, 0, cstr_opt(p.func)),
                meta: CollPerfMeta {
                    is_p2p: true,
                    peer: p.peer,
                    count: p.count as u64,
                    msg_bytes: p.count as u64 * dtype_bytes(dtype_name),
                    dtype: ShortName::from_str(dtype_name),
                    algo: ShortName::default(),
                    proto: ShortName::default(),
                    n_channels: 0,
                },
            }
        }
        x if x == NCCL_PROFILE_PROXY_OP => {
            let p = unsafe { descr.body.proxy_op };
            ParsedEvent::ProxyOp {
                parent_obj: descr.parent_obj,
                rank: descr.rank,
                channel_id: p.channel_id as i32,
                peer: p.peer,
                n_steps: p.n_steps,
                is_send: p.is_send,
            }
        }
        x if x == NCCL_PROFILE_PROXY_STEP => ParsedEvent::ProxyStep {
            parent_obj: descr.parent_obj,
            step: unsafe { descr.body.proxy_step.step },
        },
        x if x == NCCL_PROFILE_KERNEL_CH => ParsedEvent::KernelCh {
            parent_obj: descr.parent_obj,
            channel_id: unsafe { descr.body.kernel_ch.channel_id } as i32,
            gpu_start_ptimer: 0, // v3 has no GPU timer
        },
        x if x == NCCL_PROFILE_NET_PLUGIN => ParsedEvent::NetPlugin {
            rank: descr.rank,
            id: unsafe { descr.body.net_plugin.id },
            data: unsafe { descr.body.net_plugin.data },
        },
        _ => ParsedEvent::Ignored,
    }
}

/// Parse a v4 descriptor (comm metadata comes from the per-comm `init`).
pub fn parse_descr_v4(descr: &NcclProfilerEventDescrV4, comm: CommInfo) -> ParsedEvent {
    let t = descr.type_ as i32;
    match t {
        x if x == NCCL_PROFILE_COLL => {
            let c = unsafe { descr.body.coll };
            let dtype_name = cstr_opt(c.datatype);
            ParsedEvent::Coll {
                ctx: build_coll_context(
                    descr.rank,
                    comm.comm_hash,
                    comm.n_ranks,
                    c.seq_number,
                    cstr_opt(c.func),
                ),
                meta: CollPerfMeta {
                    is_p2p: false,
                    peer: -1,
                    count: c.count as u64,
                    msg_bytes: c.count as u64 * dtype_bytes(dtype_name),
                    dtype: ShortName::from_str(dtype_name),
                    algo: ShortName::from_str(cstr_opt(c.algo)),
                    proto: ShortName::from_str(cstr_opt(c.proto)),
                    n_channels: c.n_channels as i32,
                },
            }
        }
        x if x == NCCL_PROFILE_P2P => {
            let p = unsafe { descr.body.p2p };
            let dtype_name = cstr_opt(p.datatype);
            ParsedEvent::Coll {
                ctx: build_coll_context(
                    descr.rank,
                    comm.comm_hash,
                    comm.n_ranks,
                    0,
                    cstr_opt(p.func),
                ),
                meta: CollPerfMeta {
                    is_p2p: true,
                    peer: p.peer,
                    count: p.count as u64,
                    msg_bytes: p.count as u64 * dtype_bytes(dtype_name),
                    dtype: ShortName::from_str(dtype_name),
                    algo: ShortName::default(),
                    proto: ShortName::default(),
                    n_channels: p.n_channels as i32,
                },
            }
        }
        x if x == NCCL_PROFILE_PROXY_OP => {
            let p = unsafe { descr.body.proxy_op };
            ParsedEvent::ProxyOp {
                parent_obj: descr.parent_obj,
                rank: descr.rank,
                channel_id: p.channel_id as i32,
                peer: p.peer,
                n_steps: p.n_steps,
                is_send: p.is_send,
            }
        }
        x if x == NCCL_PROFILE_PROXY_STEP => ParsedEvent::ProxyStep {
            parent_obj: descr.parent_obj,
            step: unsafe { descr.body.proxy_step.step },
        },
        x if x == NCCL_PROFILE_KERNEL_CH => {
            let k = unsafe { descr.body.kernel_ch };
            ParsedEvent::KernelCh {
                parent_obj: descr.parent_obj,
                channel_id: k.channel_id as i32,
                gpu_start_ptimer: k.p_timer,
            }
        }
        x if x == NCCL_PROFILE_NET_PLUGIN => ParsedEvent::NetPlugin {
            rank: descr.rank,
            id: unsafe { descr.body.net_plugin.id },
            data: unsafe { descr.body.net_plugin.data },
        },
        _ => ParsedEvent::Ignored,
    }
}

fn coll_context_fallback(rank: i32) -> CollContext {
    let mut ctx = CollContext {
        rank,
        ..Default::default()
    };
    copy_func_name(&mut ctx, None);
    ctx
}

impl Pools {
    pub fn new(limits: PoolLimits) -> Self {
        Self {
            coll_pool: SlotPool::with_capacity(limits.coll),
            proxy_pool: SlotPool::with_capacity(limits.proxy_op),
            step_pool: SlotPool::with_capacity(limits.proxy_step),
            kch_pool: SlotPool::with_capacity(limits.kernel_ch),
            net_pool: SlotPool::with_capacity(limits.net),
        }
    }

    fn note_pool_exhausted(
        &mut self,
        pool_name: &str,
        usage_pct: u8,
        counters: &EventCounters,
        parent_coll: u32,
    ) {
        counters.pool_exhausted.fetch_add(1, Ordering::Relaxed);
        warn_pool_exhausted(pool_name, usage_pct);
        if parent_coll != INVALID_IDX {
            if let Some(coll) = self.coll_pool.get_mut(parent_coll) {
                coll.pool_events_dropped = coll.pool_events_dropped.saturating_add(1);
            }
        }
    }

    fn counter_snapshot(&self, counters: &EventCounters) -> ProfilerCounterSnapshot {
        ProfilerCounterSnapshot {
            ts_ns: now_ns(),
            rank: crate::role::training_rank(),
            coll_events: counters.coll.load(Ordering::Relaxed),
            p2p_events: counters.p2p.load(Ordering::Relaxed),
            proxy_op_events: counters.proxy_op.load(Ordering::Relaxed),
            proxy_step_events: counters.proxy_step.load(Ordering::Relaxed),
            kernel_ch_events: counters.kernel_ch.load(Ordering::Relaxed),
            net_events: counters.net_plugin.load(Ordering::Relaxed),
            rows_written: counters.rows_written.load(Ordering::Relaxed),
            pool_exhausted: counters.pool_exhausted.load(Ordering::Relaxed),
            write_errors: counters.write_errors.load(Ordering::Relaxed),
            filtered: counters.filtered.load(Ordering::Relaxed),
            coll_live: self.coll_pool.live_count() as i32,
            proxy_live: self.proxy_pool.live_count() as i32,
            step_live: self.step_pool.live_count() as i32,
            kch_live: self.kch_pool.live_count() as i32,
            net_live: self.net_pool.live_count() as i32,
            coll_cap: self.coll_pool.capacity() as i32,
            proxy_cap: self.proxy_pool.capacity() as i32,
            step_cap: self.step_pool.capacity() as i32,
            kch_cap: self.kch_pool.capacity() as i32,
            net_cap: self.net_pool.capacity() as i32,
            ring_chunks_recycled: 0,
            ring_rows_overwritten: 0,
        }
    }

    fn start_event(
        &mut self,
        handle: *mut *mut c_void,
        event: ParsedEvent,
        counters: &EventCounters,
        shard: usize,
    ) {
        match event {
            ParsedEvent::Coll { ctx, meta } => {
                if meta.is_p2p {
                    counters.p2p.fetch_add(1, Ordering::Relaxed);
                } else {
                    counters.coll.fetch_add(1, Ordering::Relaxed);
                }
                let ts = now_ns();
                if let Some((ptr, idx)) = self.coll_pool.alloc(|| CollSlot::new(ctx, meta, ts)) {
                    unsafe {
                        (*ptr).set_self_idx(pack_slot_id(shard, idx));
                        *handle = ptr as *mut c_void;
                    }
                    check_pool_usage("coll", &self.coll_pool);
                } else {
                    check_pool_usage("coll", &self.coll_pool);
                    self.note_pool_exhausted(
                        "coll",
                        self.coll_pool.usage_pct(),
                        counters,
                        INVALID_IDX,
                    );
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            ParsedEvent::ProxyOp {
                parent_obj,
                rank,
                channel_id,
                peer,
                n_steps,
                is_send,
            } => {
                counters.proxy_op.fetch_add(1, Ordering::Relaxed);
                let (coll, parent_coll) = self.resolve_parent_coll(parent_obj, rank);

                if let Some((ptr, idx)) = self.proxy_pool.alloc(|| ProxyOpSlot {
                    tag: EVT_PROXY_OP,
                    self_idx: INVALID_IDX,
                    op: ProxyOpData {
                        channel_id,
                        peer,
                        is_send,
                        n_steps,
                        coll,
                        start_ns: now_ns(),
                        parent_coll,
                        ..Default::default()
                    },
                }) {
                    if let Some(coll_slot) = self.coll_pool.get_mut(parent_coll) {
                        coll_slot.live_children = coll_slot.live_children.saturating_add(1);
                    }
                    unsafe {
                        (*ptr).set_self_idx(pack_slot_id(shard, idx));
                        *handle = ptr as *mut c_void;
                    }
                    check_pool_usage("proxy_op", &self.proxy_pool);
                } else {
                    check_pool_usage("proxy_op", &self.proxy_pool);
                    self.note_pool_exhausted(
                        "proxy_op",
                        self.proxy_pool.usage_pct(),
                        counters,
                        parent_coll,
                    );
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            ParsedEvent::KernelCh {
                parent_obj,
                channel_id,
                gpu_start_ptimer,
            } => {
                counters.kernel_ch.fetch_add(1, Ordering::Relaxed);
                let parent_coll = if parent_obj.is_null() {
                    INVALID_IDX
                } else {
                    // SAFETY: parent_obj is a coll handle we returned to NCCL.
                    unsafe { self.coll_pool.index_of(parent_obj as *mut CollSlot) }
                        .unwrap_or(INVALID_IDX)
                };
                if parent_coll == INVALID_IDX {
                    // Without a parent there is nothing to attribute the
                    // kernel window to; skip tracking.
                    unsafe { *handle = std::ptr::null_mut() };
                    return;
                }
                if let Some((ptr, idx)) = self.kch_pool.alloc(|| KernelChSlot {
                    tag: EVT_KERNEL_CH,
                    self_idx: INVALID_IDX,
                    parent_coll,
                    channel_id,
                    start_ns: now_ns(),
                    gpu_start_ptimer,
                    gpu_stop_ptimer: 0,
                }) {
                    if let Some(coll_slot) = self.coll_pool.get_mut(parent_coll) {
                        coll_slot.live_children = coll_slot.live_children.saturating_add(1);
                    }
                    unsafe {
                        (*ptr).set_self_idx(pack_slot_id(shard, idx));
                        *handle = ptr as *mut c_void;
                    }
                    check_pool_usage("kernel_ch", &self.kch_pool);
                } else {
                    check_pool_usage("kernel_ch", &self.kch_pool);
                    self.note_pool_exhausted(
                        "kernel_ch",
                        self.kch_pool.usage_pct(),
                        counters,
                        parent_coll,
                    );
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            ParsedEvent::ProxyStep { parent_obj, step } => {
                counters.proxy_step.fetch_add(1, Ordering::Relaxed);
                let parent_proxy = if parent_obj.is_null() {
                    INVALID_IDX
                } else {
                    // SAFETY: parent_obj is a proxy-op handle we returned to NCCL.
                    unsafe { self.proxy_pool.index_of(parent_obj as *mut ProxyOpSlot) }
                        .unwrap_or(INVALID_IDX)
                };
                let is_send = if parent_proxy != INVALID_IDX {
                    self.proxy_pool
                        .get_mut(parent_proxy)
                        .map(|s| s.op.is_send)
                        .unwrap_or(0)
                } else {
                    0
                };
                if let Some((ptr, idx)) = self.step_pool.alloc(|| ProxyStepSlot {
                    tag: EVT_PROXY_STEP,
                    self_idx: INVALID_IDX,
                    parent_proxy,
                    step: ProxyStepData {
                        step,
                        is_send,
                        start_ns: now_ns(),
                        ..Default::default()
                    },
                }) {
                    unsafe {
                        (*ptr).set_self_idx(pack_slot_id(shard, idx));
                        *handle = ptr as *mut c_void;
                    }
                    check_pool_usage("proxy_step", &self.step_pool);
                } else {
                    check_pool_usage("proxy_step", &self.step_pool);
                    self.note_pool_exhausted(
                        "proxy_step",
                        self.step_pool.usage_pct(),
                        counters,
                        INVALID_IDX,
                    );
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            ParsedEvent::NetPlugin { rank, id, data } => {
                counters.net_plugin.fetch_add(1, Ordering::Relaxed);
                let mut device = 0i32;
                let mut qp_num = 0i32;
                let mut wr_id = 0u64;
                let mut opcode = 0i32;
                let mut length = 0u64;
                if net_plugin_type(id) == NCCL_PROFILER_NET_TYPE_IB {
                    if let Some(qp) = unsafe { read_ib_qp(data) } {
                        device = qp.device;
                        qp_num = qp.qp_num;
                        wr_id = qp.wr_id;
                        opcode = qp.opcode;
                        length = qp.length as u64;
                    }
                }
                if let Some((ptr, idx)) = self.net_pool.alloc(|| NetPluginSlot {
                    tag: EVT_NET_PLUGIN,
                    self_idx: INVALID_IDX,
                    start_ns: now_ns(),
                    rank,
                    device,
                    qp_num,
                    wr_id,
                    opcode,
                    length,
                    stop_ns: 0,
                }) {
                    unsafe {
                        (*ptr).set_self_idx(pack_slot_id(shard, idx));
                        *handle = ptr as *mut c_void;
                    }
                    check_pool_usage("net", &self.net_pool);
                } else {
                    check_pool_usage("net", &self.net_pool);
                    self.note_pool_exhausted(
                        "net",
                        self.net_pool.usage_pct(),
                        counters,
                        INVALID_IDX,
                    );
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            ParsedEvent::Ignored => unsafe { *handle = std::ptr::null_mut() },
        }
    }

    fn resolve_parent_coll(&mut self, parent_obj: *mut c_void, rank: i32) -> (CollContext, u32) {
        if parent_obj.is_null() {
            return (coll_context_fallback(rank), INVALID_IDX);
        }
        // SAFETY: parent_obj is a coll handle we returned to NCCL.
        let resolved = unsafe { self.coll_pool.index_of(parent_obj as *mut CollSlot) };
        if let Some(coll_idx) = resolved {
            if let Some(coll_slot) = self.coll_pool.get_mut(coll_idx) {
                return (coll_slot.ctx, coll_idx);
            }
        }
        (coll_context_fallback(rank), INVALID_IDX)
    }

    fn stop_event(&mut self, handle: *mut c_void, counters: &EventCounters) -> FlushBatch {
        let mut batch = FlushBatch::default();
        if handle.is_null() {
            return batch;
        }
        match unsafe { event_type(handle) } {
            EVT_PROXY_STEP => {
                // SAFETY: handle tag was checked; it is a slot we handed to NCCL.
                let Some(step_idx) =
                    (unsafe { self.step_pool.index_of(handle as *mut ProxyStepSlot) })
                else {
                    return batch;
                };
                let (parent_proxy, step) = {
                    let Some(slot) = self.step_pool.get_mut(step_idx) else {
                        return batch;
                    };
                    slot.step.stop_ns = now_ns();
                    (slot.parent_proxy, slot.step)
                };
                if parent_proxy != INVALID_IDX {
                    if let Some(proxy) = self.proxy_pool.get_mut(parent_proxy) {
                        proxy.op.push_step(step);
                    }
                }
                self.step_pool.free_idx(step_idx);
            }
            EVT_PROXY_OP => {
                // SAFETY: handle tag was checked; it is a slot we handed to NCCL.
                let Some(proxy_idx) =
                    (unsafe { self.proxy_pool.index_of(handle as *mut ProxyOpSlot) })
                else {
                    return batch;
                };
                let (parent_coll, start_ns, stop_ns, row) = {
                    let Some(slot) = self.proxy_pool.get_mut(proxy_idx) else {
                        return batch;
                    };
                    slot.op.stop_ns = now_ns();
                    let parent = slot.op.parent_coll;
                    let (start, stop) = (slot.op.start_ns, slot.op.stop_ns);
                    let row = std::mem::take(&mut slot.op).into_completed();
                    (parent, start, stop, row)
                };
                self.flush_proxy_row(parent_coll, row, counters, &mut batch);
                self.child_stopped(parent_coll, |coll| coll.observe_proxy(start_ns, stop_ns));
                self.maybe_complete_coll(parent_coll, counters, &mut batch);
                self.proxy_pool.free_idx(proxy_idx);
            }
            EVT_KERNEL_CH => {
                // SAFETY: handle tag was checked; it is a slot we handed to NCCL.
                let Some(kch_idx) =
                    (unsafe { self.kch_pool.index_of(handle as *mut KernelChSlot) })
                else {
                    return batch;
                };
                let (parent_coll, start_ns, gpu_start, gpu_stop) = {
                    let Some(slot) = self.kch_pool.get_mut(kch_idx) else {
                        return batch;
                    };
                    (
                        slot.parent_coll,
                        slot.start_ns,
                        slot.gpu_start_ptimer,
                        slot.gpu_stop_ptimer,
                    )
                };
                let stop_ns = now_ns();
                self.child_stopped(parent_coll, |coll| {
                    coll.observe_kch(start_ns, stop_ns);
                    // v4: also fold the GPU-globaltimer window when reported.
                    coll.observe_kch_gpu(gpu_start, gpu_stop);
                });
                self.maybe_complete_coll(parent_coll, counters, &mut batch);
                self.kch_pool.free_idx(kch_idx);
            }
            EVT_COLL => {
                // SAFETY: handle tag was checked; it is a slot we handed to NCCL.
                let Some(coll_idx) = (unsafe { self.coll_pool.index_of(handle as *mut CollSlot) })
                else {
                    return batch;
                };
                if let Some(slot) = self.coll_pool.get_mut(coll_idx) {
                    slot.enqueue_stop_ns = now_ns();
                    slot.stopped = true;
                }
                self.maybe_complete_coll(coll_idx, counters, &mut batch);
            }
            EVT_NET_PLUGIN => {
                // SAFETY: handle tag was checked; it is a slot we handed to NCCL.
                let Some(net_idx) =
                    (unsafe { self.net_pool.index_of(handle as *mut NetPluginSlot) })
                else {
                    return batch;
                };
                if let Some(slot) = self.net_pool.get_mut(net_idx) {
                    slot.stop_ns = now_ns();
                    batch.net_qp.push(CompletedNetQp {
                        ts_ns: slot.stop_ns,
                        rank: slot.rank,
                        device: slot.device,
                        qp_num: slot.qp_num,
                        wr_id: slot.wr_id,
                        opcode: slot.opcode,
                        length: slot.length,
                        duration_ns: slot.stop_ns.saturating_sub(slot.start_ns),
                    });
                }
                self.net_pool.free_idx(net_idx);
            }
            _ => {}
        }
        batch
    }

    /// Fold a finished child event into its parent coll and drop the refcount.
    fn child_stopped(&mut self, parent_coll: u32, observe: impl FnOnce(&mut CollSlot)) {
        if parent_coll == INVALID_IDX {
            return;
        }
        if let Some(coll) = self.coll_pool.get_mut(parent_coll) {
            observe(coll);
            coll.live_children = coll.live_children.saturating_sub(1);
        }
    }

    /// Emit the coll_perf row and free the slot once enqueue is stopped and
    /// all child events have finished (NCCL refcount completion).
    fn maybe_complete_coll(
        &mut self,
        coll_idx: u32,
        counters: &EventCounters,
        batch: &mut FlushBatch,
    ) {
        if coll_idx == INVALID_IDX {
            return;
        }
        let Some(slot) = self.coll_pool.get_mut(coll_idx) else {
            return;
        };
        if !slot.is_complete() {
            return;
        }
        let perf_row = slot.coll_perf_row();
        let pending = &slot.pending[..slot.pending_len as usize];
        if passes_filter(perf_row.msg_size_bytes) {
            batch.coll_perf.push(perf_row);
            batch.proxy_ops.extend_from_slice(pending);
        } else {
            counters
                .filtered
                .fetch_add(1 + pending.len() as u64, Ordering::Relaxed);
        }
        self.coll_pool.free_idx(coll_idx);
    }

    fn flush_proxy_row(
        &mut self,
        parent_coll: u32,
        row: CompletedProxyOp,
        counters: &EventCounters,
        batch: &mut FlushBatch,
    ) {
        if parent_coll != INVALID_IDX {
            if let Some(coll) = self.coll_pool.get_mut(parent_coll) {
                let coll_bytes = coll.meta.msg_bytes;
                if !passes_filter(coll_bytes) {
                    counters.filtered.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if coll.push_pending(row) {
                    return;
                }
                // pending full — flush coll batch early
                batch
                    .proxy_ops
                    .extend_from_slice(&coll.pending[..coll.pending_len as usize]);
                coll.pending_len = 0;
                if !coll.push_pending(row) {
                    batch.proxy_ops.push(row);
                }
                return;
            }
        }
        if !passes_filter(row.trans_bytes) {
            counters.filtered.fetch_add(1, Ordering::Relaxed);
            return;
        }
        batch.proxy_ops.push(row);
    }

    fn record_state(
        &mut self,
        handle: *mut c_void,
        state: NcclProfilerEventState,
        args: StateArgs,
    ) {
        if handle.is_null() {
            return;
        }
        let ts = now_ns();
        match unsafe { event_type(handle) } {
            EVT_PROXY_STEP => {
                // SAFETY: handle tag was checked; it is a slot we handed to NCCL.
                let Some(step_idx) =
                    (unsafe { self.step_pool.index_of(handle as *mut ProxyStepSlot) })
                else {
                    return;
                };
                let Some(slot) = self.step_pool.get_mut(step_idx) else {
                    return;
                };
                if let Some(idx) = proxy_step_state_index(state) {
                    // First entry wins: NCCL re-reports some states on every
                    // progress-loop retry (e.g. SendPeerWait while waiting
                    // for receiver credits); overwriting would collapse the
                    // dwell time to the last retry interval.
                    if slot.step.state_ts[idx] == 0 {
                        slot.step.state_ts[idx] = ts;
                    }
                }
                if let StateArgs::ProxyStepV4 { trans_size } = args {
                    // Only sent for SendWait / RecvFlushWait, where the value
                    // is authoritative for this step (see plugin.rs).
                    slot.step.trans_bytes = trans_size;
                }
            }
            EVT_PROXY_OP => {
                // SAFETY: handle tag was checked; it is a slot we handed to NCCL.
                let Some(proxy_idx) =
                    (unsafe { self.proxy_pool.index_of(handle as *mut ProxyOpSlot) })
                else {
                    return;
                };
                let Some(slot) = self.proxy_pool.get_mut(proxy_idx) else {
                    return;
                };
                if let StateArgs::ProxyOpV3 { trans_size, steps } = args {
                    // Modern NCCL reports some proxy-op states (InProgress)
                    // with zeroed args — never clobber a good value with 0,
                    // and the v3 counter is cumulative, so keep the max.
                    if trans_size > 0 {
                        slot.op.trans_bytes = slot.op.trans_bytes.max(trans_size);
                    }
                    if steps > 0 {
                        slot.op.n_steps = steps;
                    }
                }
            }
            EVT_KERNEL_CH => {
                // v4 `KernelChStop`: GPU globaltimer stop timestamp.
                let Some(kch_idx) =
                    (unsafe { self.kch_pool.index_of(handle as *mut KernelChSlot) })
                else {
                    return;
                };
                let Some(slot) = self.kch_pool.get_mut(kch_idx) else {
                    return;
                };
                if let StateArgs::KernelChStop { p_timer } = args {
                    slot.gpu_stop_ptimer = p_timer;
                }
            }
            _ => {}
        }
    }

    /// Collect one `nccl.inflight_ops` row per live op older than `min_age_ns`.
    ///
    /// A hung collective never reaches completion, so it would otherwise be
    /// invisible in `nccl.proxy_ops` / `nccl.coll_perf`. Called periodically
    /// from the watchdog thread (see `plugin::spawn_inflight_watchdog`).
    fn collect_inflight(&mut self, min_age_ns: i64) -> (Vec<InflightOp>, FlushBatch) {
        let now = now_ns();
        let mut rows: Vec<InflightOp> = Vec::new();

        self.coll_pool.for_each_live(|_, slot| {
            let age = now - slot.start_ns;
            if age >= min_age_ns {
                rows.push(InflightOp {
                    ts_ns: now,
                    rank: slot.ctx.rank,
                    comm_hash: slot.ctx.comm_hash,
                    coll_func: slot.ctx.func,
                    coll_func_len: slot.ctx.func_len,
                    seq: slot.ctx.seq,
                    kind: if slot.meta.is_p2p { "p2p" } else { "coll" },
                    channel_id: -1,
                    peer: slot.meta.peer,
                    is_send: -1,
                    start_ns: slot.start_ns,
                    age_ns: age,
                });
            }
        });
        self.proxy_pool.for_each_live(|_, slot| {
            let age = now - slot.op.start_ns;
            if age >= min_age_ns {
                rows.push(InflightOp {
                    ts_ns: now,
                    rank: slot.op.coll.rank,
                    comm_hash: slot.op.coll.comm_hash,
                    coll_func: slot.op.coll.func,
                    coll_func_len: slot.op.coll.func_len,
                    seq: slot.op.coll.seq,
                    kind: "proxy_op",
                    channel_id: slot.op.channel_id,
                    peer: slot.op.peer,
                    is_send: slot.op.is_send,
                    start_ns: slot.op.start_ns,
                    age_ns: age,
                });
            }
        });
        // Drain completed proxy rows still buffered under over-age colls.
        let mut batch = FlushBatch::default();
        for idx in 0..self.coll_pool.capacity() as u32 {
            if let Some(coll) = self.coll_pool.get_mut(idx) {
                if coll.pending_len == 0 || now - coll.start_ns < min_age_ns {
                    continue;
                }
                batch
                    .proxy_ops
                    .extend_from_slice(&coll.pending[..coll.pending_len as usize]);
                coll.pending_len = 0;
            }
        }
        (rows, batch)
    }

    /// Drain buffered proxy rows of still-live colls (finalize path).
    fn drain_pending(&mut self) -> FlushBatch {
        let mut batch = FlushBatch::default();
        for idx in 0..self.coll_pool.capacity() as u32 {
            if let Some(coll) = self.coll_pool.get_mut(idx) {
                if coll.pending_len == 0 {
                    continue;
                }
                batch
                    .proxy_ops
                    .extend_from_slice(&coll.pending[..coll.pending_len as usize]);
                coll.pending_len = 0;
            }
        }
        batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::profiler_v4::{
        NcclProfilerCollDescrV4, NcclProfilerKernelChDescrV4, NcclProfilerP2pDescrV4,
    };
    use crate::abi::NcclProfilerEventBodyV4;

    const COMM: CommInfo = CommInfo {
        comm_hash: 0xFEED,
        n_ranks: 8,
    };

    #[test]
    fn parse_v4_coll_uses_comm_metadata() {
        let descr = NcclProfilerEventDescrV4 {
            type_: NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 5,
            body: NcclProfilerEventBodyV4 {
                coll: {
                    let mut c: NcclProfilerCollDescrV4 = unsafe { std::mem::zeroed() };
                    c.seq_number = 3;
                    c.func = c"AllGather".as_ptr();
                    c.count = 1024;
                    c.datatype = c"ncclFloat16".as_ptr();
                    c.n_channels = 2;
                    c
                },
            },
        };
        let ParsedEvent::Coll { ctx, meta } = parse_descr_v4(&descr, COMM) else {
            panic!("expected Coll");
        };
        // v4 descriptors carry no comm hash — it must come from init metadata.
        assert_eq!(ctx.comm_hash, 0xFEED);
        assert_eq!(ctx.n_ranks, 8);
        assert_eq!(ctx.rank, 5);
        assert_eq!(ctx.seq, 3);
        assert_eq!(meta.msg_bytes, 1024 * 2);
        assert_eq!(meta.n_channels, 2);
        assert!(!meta.is_p2p);
    }

    #[test]
    fn parse_v4_p2p_has_channels() {
        let descr = NcclProfilerEventDescrV4 {
            type_: NCCL_PROFILE_P2P as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 1,
            body: NcclProfilerEventBodyV4 {
                p2p: {
                    let mut p: NcclProfilerP2pDescrV4 = unsafe { std::mem::zeroed() };
                    p.func = c"Send".as_ptr();
                    p.count = 64;
                    p.datatype = c"ncclInt64".as_ptr();
                    p.peer = 7;
                    p.n_channels = 3;
                    p
                },
            },
        };
        let ParsedEvent::Coll { ctx, meta } = parse_descr_v4(&descr, COMM) else {
            panic!("expected Coll(p2p)");
        };
        assert!(meta.is_p2p);
        assert_eq!(meta.peer, 7);
        assert_eq!(meta.n_channels, 3); // v4 exposes p2p channels (v3: 0)
        assert_eq!(ctx.comm_hash, 0xFEED);
    }

    #[test]
    fn parse_v4_kernel_ch_carries_gpu_ptimer() {
        let descr = NcclProfilerEventDescrV4 {
            type_: NCCL_PROFILE_KERNEL_CH as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 0,
            body: NcclProfilerEventBodyV4 {
                kernel_ch: NcclProfilerKernelChDescrV4 {
                    channel_id: 4,
                    p_timer: 123_456,
                },
            },
        };
        let ParsedEvent::KernelCh {
            channel_id,
            gpu_start_ptimer,
            ..
        } = parse_descr_v4(&descr, COMM)
        else {
            panic!("expected KernelCh");
        };
        assert_eq!(channel_id, 4);
        assert_eq!(gpu_start_ptimer, 123_456);
    }

    #[test]
    fn proxy_step_state_ts_first_entry_wins() {
        use crate::abi::STATE_PROXY_STEP_SEND_PEER_WAIT_V4;
        use crate::events::{ProxyStepSlot, IDX_SEND_PEER_WAIT};

        let counters = EventCounters::new();
        let mut pools = Pools::new(PoolLimits {
            coll: 4,
            proxy_op: 4,
            proxy_step: 4,
            kernel_ch: 4,
            net: 4,
        });
        let mut op_h: *mut c_void = std::ptr::null_mut();
        pools.start_event(
            &mut op_h,
            ParsedEvent::ProxyOp {
                parent_obj: std::ptr::null_mut(),
                rank: 0,
                channel_id: 0,
                peer: 1,
                n_steps: 1,
                is_send: 1,
            },
            &counters,
            0,
        );
        let mut st_h: *mut c_void = std::ptr::null_mut();
        pools.start_event(
            &mut st_h,
            ParsedEvent::ProxyStep {
                parent_obj: op_h,
                step: 0,
            },
            &counters,
            0,
        );
        let read_ts = |pools: &mut Pools| {
            let idx = unsafe { pools.step_pool.index_of(st_h as *mut ProxyStepSlot) }.unwrap();
            pools.step_pool.get_mut(idx).unwrap().step.state_ts[IDX_SEND_PEER_WAIT]
        };

        pools.record_state(st_h, STATE_PROXY_STEP_SEND_PEER_WAIT_V4, StateArgs::None);
        let first = read_ts(&mut pools);
        assert!(first > 0);
        std::thread::sleep(std::time::Duration::from_millis(2));
        pools.record_state(st_h, STATE_PROXY_STEP_SEND_PEER_WAIT_V4, StateArgs::None);
        assert_eq!(read_ts(&mut pools), first);
    }

    #[test]
    fn sharded_pools_default_one_shard_in_tests() {
        assert_eq!(pool_shard_count(), 1);
        let _pools = ShardedPools::new();
    }

    #[test]
    fn parse_v3_coll_has_no_comm_size() {
        let descr = NcclProfilerEventDescrV3 {
            type_: NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 2,
            body: crate::abi::NcclProfilerEventBodyV3 {
                coll: {
                    let mut c: crate::abi::NcclProfilerCollDescr = unsafe { std::mem::zeroed() };
                    c.comm_hash = 42;
                    c.seq_number = 1;
                    c.func = c"AllReduce".as_ptr();
                    c
                },
            },
        };
        let ParsedEvent::Coll { ctx, .. } = parse_descr_v3(&descr) else {
            panic!("expected Coll");
        };
        assert_eq!(ctx.comm_hash, 42);
        assert_eq!(ctx.n_ranks, -1, "v3 cannot know the communicator size");
    }

    #[test]
    fn v4_step_trans_bytes_only_from_proxy_step_v4_args() {
        use crate::abi::{STATE_PROXY_STEP_RECV_GPU_WAIT, STATE_PROXY_STEP_SEND_WAIT};
        use crate::events::ProxyStepSlot;
        use crate::pool_config::PoolLimits;

        let counters = EventCounters::new();
        let limits = PoolLimits {
            coll: 4,
            proxy_op: 4,
            proxy_step: 4,
            kernel_ch: 4,
            net: 4,
        };
        let mut pools = Pools::new(limits);
        let mut op_h: *mut c_void = std::ptr::null_mut();
        pools.start_event(
            &mut op_h,
            ParsedEvent::ProxyOp {
                parent_obj: std::ptr::null_mut(),
                rank: 0,
                channel_id: 0,
                peer: 1,
                n_steps: 1,
                is_send: 1,
            },
            &counters,
            0,
        );
        let mut st_h: *mut c_void = std::ptr::null_mut();
        pools.start_event(
            &mut st_h,
            ParsedEvent::ProxyStep {
                parent_obj: op_h,
                step: 0,
            },
            &counters,
            0,
        );

        // Stale trans_size on a non-authoritative state must not be recorded
        // (plugin only forwards ProxyStepV4 on SendWait / RecvFlushWait).
        pools.record_state(st_h, STATE_PROXY_STEP_RECV_GPU_WAIT, StateArgs::None);
        let idx = unsafe { pools.step_pool.index_of(st_h as *mut ProxyStepSlot) }.unwrap();
        assert_eq!(
            pools.step_pool.get_mut(idx).unwrap().step.trans_bytes,
            0,
            "RecvGpuWait must not carry trans_size"
        );

        pools.record_state(
            st_h,
            STATE_PROXY_STEP_SEND_WAIT,
            StateArgs::ProxyStepV4 { trans_size: 4096 },
        );
        assert_eq!(pools.step_pool.get_mut(idx).unwrap().step.trans_bytes, 4096);
    }

    #[test]
    fn v3_proxy_op_zero_trans_size_does_not_clobber() {
        use crate::events::ProxyOpSlot;

        let counters = EventCounters::new();
        let limits = PoolLimits {
            coll: 4,
            proxy_op: 4,
            proxy_step: 4,
            kernel_ch: 4,
            net: 4,
        };
        let mut pools = Pools::new(limits);
        let mut op_h: *mut c_void = std::ptr::null_mut();
        pools.start_event(
            &mut op_h,
            ParsedEvent::ProxyOp {
                parent_obj: std::ptr::null_mut(),
                rank: 0,
                channel_id: 0,
                peer: 1,
                n_steps: 2,
                is_send: 1,
            },
            &counters,
            0,
        );
        pools.record_state(
            op_h,
            crate::abi::STATE_PROXY_OP_IN_PROGRESS_V4,
            StateArgs::ProxyOpV3 {
                trans_size: 8192,
                steps: 2,
            },
        );
        pools.record_state(
            op_h,
            crate::abi::STATE_PROXY_OP_IN_PROGRESS_V4,
            StateArgs::ProxyOpV3 {
                trans_size: 0,
                steps: 0,
            },
        );
        let idx = unsafe { pools.proxy_pool.index_of(op_h as *mut ProxyOpSlot) }.unwrap();
        assert_eq!(pools.proxy_pool.get_mut(idx).unwrap().op.trans_bytes, 8192);
    }

    #[test]
    fn collect_inflight_drains_pending_proxy_for_stale_coll() {
        use crate::pool_config::PoolLimits;

        let counters = EventCounters::new();
        let limits = PoolLimits {
            coll: 4,
            proxy_op: 4,
            proxy_step: 4,
            kernel_ch: 4,
            net: 4,
        };
        let mut pools = Pools::new(limits);
        let mut coll_h: *mut c_void = std::ptr::null_mut();
        pools.start_event(
            &mut coll_h,
            ParsedEvent::Coll {
                ctx: CollContext {
                    rank: 2,
                    comm_hash: 11,
                    ..Default::default()
                },
                meta: CollPerfMeta {
                    msg_bytes: 1024,
                    ..Default::default()
                },
            },
            &counters,
            0,
        );
        let coll_idx = unsafe { pools.coll_pool.index_of(coll_h as *mut CollSlot) }.unwrap();
        {
            let coll = pools.coll_pool.get_mut(coll_idx).unwrap();
            coll.start_ns = 1;
            let row = CompletedProxyOp {
                trans_bytes: 999,
                ..Default::default()
            };
            assert!(coll.push_pending(row));
        }

        let (_rows, batch) = pools.collect_inflight(1_000_000);
        assert_eq!(batch.proxy_ops.len(), 1);
        assert_eq!(batch.proxy_ops[0].trans_bytes, 999);
        assert_eq!(pools.coll_pool.get_mut(coll_idx).unwrap().pending_len, 0);
    }
}
