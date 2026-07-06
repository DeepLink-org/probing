//! Event slot layouts and wait aggregation.

use std::sync::atomic::AtomicU64;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;

use crate::abi::{
    NcclProfilerEventState, STATE_PROXY_STEP_RECV_FLUSH_WAIT, STATE_PROXY_STEP_RECV_GPU_WAIT,
    STATE_PROXY_STEP_RECV_WAIT, STATE_PROXY_STEP_SEND_GPU_WAIT, STATE_PROXY_STEP_SEND_PEER_WAIT_V4,
    STATE_PROXY_STEP_SEND_WAIT,
};
use crate::pool::{Indexed, INVALID_IDX};
use crate::role::RoleRanks;

pub const MAX_FUNC_NAME: usize = 32;
pub const MAX_ALGO_NAME: usize = 16;
pub const MAX_PENDING_PER_COLL: usize = 128;

pub const EVT_COLL: u8 = 1;
pub const EVT_PROXY_OP: u8 = 2;
pub const EVT_PROXY_STEP: u8 = 3;
pub const EVT_NET_PLUGIN: u8 = 4;
pub const EVT_KERNEL_CH: u8 = 5;

/// How `exec_time_ns` in `nccl.coll_perf` was measured.
///
/// NCCL's own docs: coll `stopEvent` only marks **enqueue** completion; real
/// execution must be reconstructed from child proxy / kernel-channel events
/// (reference counting, as recommended by the official ext-profiler guide).
///
/// `kernel_gpu` (v4 only) is the GPU globaltimer window reported by the
/// kernel itself (`kernelCh.pTimer` start + `KernelChStop` state stop) —
/// device-clock, the most precise signal the ABI offers.
pub const TIMING_KERNEL_GPU: &str = "kernel_gpu";
pub const TIMING_KERNEL_CH: &str = "kernel_ch";
pub const TIMING_PROXY: &str = "proxy";
pub const TIMING_ENQUEUE: &str = "enqueue";

/// Per-step state-entry timestamps: SendGpuWait, SendPeerWait (v4),
/// SendWait, RecvWait, RecvFlushWait, RecvGpuWait.
pub const PROXY_STEP_STATE_SLOTS: usize = 6;

pub const IDX_SEND_GPU_WAIT: usize = 0;
pub const IDX_SEND_PEER_WAIT: usize = 1;
pub const IDX_SEND_WAIT: usize = 2;
pub const IDX_RECV_WAIT: usize = 3;
pub const IDX_RECV_FLUSH_WAIT: usize = 4;
pub const IDX_RECV_GPU_WAIT: usize = 5;

/// Monotonic origin plus the UNIX-epoch offset captured at plugin load.
///
/// `now_ns()` returns epoch-aligned nanoseconds so `ts` columns are
/// comparable across ranks/hosts (required by `global.nccl.*` federation),
/// while durations still come from the monotonic `Instant` domain.
static CLOCK: Lazy<(Instant, i64)> = Lazy::new(|| {
    let origin = Instant::now();
    let epoch_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or_default();
    (origin, epoch_ns)
});

#[derive(Debug, Default)]
pub struct EventCounters {
    pub coll: AtomicU64,
    pub p2p: AtomicU64,
    pub proxy_op: AtomicU64,
    pub proxy_step: AtomicU64,
    pub kernel_ch: AtomicU64,
    pub net_plugin: AtomicU64,
    pub rows_written: AtomicU64,
    pub pool_exhausted: AtomicU64,
    pub write_errors: AtomicU64,
    pub filtered: AtomicU64,
}

/// One row of `nccl.profiler_counters` — periodic / event-driven health snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProfilerCounterSnapshot {
    pub ts_ns: i64,
    pub rank: i32,
    pub coll_events: u64,
    pub p2p_events: u64,
    pub proxy_op_events: u64,
    pub proxy_step_events: u64,
    pub kernel_ch_events: u64,
    pub net_events: u64,
    pub rows_written: u64,
    pub pool_exhausted: u64,
    pub write_errors: u64,
    pub filtered: u64,
    pub coll_live: i32,
    pub proxy_live: i32,
    pub step_live: i32,
    pub kch_live: i32,
    pub net_live: i32,
    pub coll_cap: i32,
    pub proxy_cap: i32,
    pub step_cap: i32,
    pub kch_cap: i32,
    pub net_cap: i32,
    pub ring_chunks_recycled: u32,
    pub ring_rows_overwritten: u32,
}

impl EventCounters {
    pub const fn new() -> Self {
        Self {
            coll: AtomicU64::new(0),
            p2p: AtomicU64::new(0),
            proxy_op: AtomicU64::new(0),
            proxy_step: AtomicU64::new(0),
            kernel_ch: AtomicU64::new(0),
            net_plugin: AtomicU64::new(0),
            rows_written: AtomicU64::new(0),
            pool_exhausted: AtomicU64::new(0),
            write_errors: AtomicU64::new(0),
            filtered: AtomicU64::new(0),
        }
    }
}

#[inline]
pub fn now_ns() -> i64 {
    let (origin, epoch_ns) = *CLOCK;
    epoch_ns + origin.elapsed().as_nanos() as i64
}

/// NCCL datatype name → element size in bytes (NCCL `ncclDataType_t` names).
pub fn dtype_bytes(name: Option<&str>) -> u64 {
    match name.unwrap_or("") {
        "ncclInt8" | "ncclChar" | "ncclUint8" | "ncclFloat8e4m3" | "ncclFloat8e5m2" => 1,
        "ncclFloat16" | "ncclHalf" | "ncclBfloat16" => 2,
        "ncclInt32" | "ncclInt" | "ncclUint32" | "ncclFloat32" | "ncclFloat" => 4,
        "ncclInt64" | "ncclUint64" | "ncclFloat64" | "ncclDouble" => 8,
        _ => 1,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CollContext {
    pub rank: i32,
    pub comm_hash: u64,
    /// Communicator size (v4 init metadata; -1 when unknown, i.e. v3).
    pub n_ranks: i32,
    pub seq: u64,
    pub func: [u8; MAX_FUNC_NAME],
    pub func_len: u8,
}

impl Default for CollContext {
    fn default() -> Self {
        Self {
            rank: -1,
            comm_hash: 0,
            n_ranks: -1,
            seq: 0,
            func: [0; MAX_FUNC_NAME],
            func_len: 0,
        }
    }
}

/// Fixed-capacity name buffer for algo/proto/datatype strings.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShortName {
    pub buf: [u8; MAX_ALGO_NAME],
    pub len: u8,
}

impl ShortName {
    // Not `FromStr`: infallible, takes Option, truncates.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(src: Option<&str>) -> Self {
        let s = src.unwrap_or("");
        let bytes = s.as_bytes();
        let n = bytes.len().min(MAX_ALGO_NAME);
        let mut buf = [0u8; MAX_ALGO_NAME];
        buf[..n].copy_from_slice(&bytes[..n]);
        Self { buf, len: n as u8 }
    }

    pub fn as_str(&self) -> &str {
        let n = (self.len as usize).min(MAX_ALGO_NAME);
        std::str::from_utf8(&self.buf[..n]).unwrap_or("")
    }
}

impl CollContext {
    #[allow(dead_code)]
    pub fn func_str(&self) -> &str {
        let n = self.func_len as usize;
        std::str::from_utf8(&self.func[..n.min(MAX_FUNC_NAME)]).unwrap_or("unknown")
    }
}

pub fn copy_func_name(dst: &mut CollContext, src: Option<&str>) {
    let s = src.unwrap_or("unknown");
    let bytes = s.as_bytes();
    let n = bytes.len().min(MAX_FUNC_NAME);
    dst.func[..n].copy_from_slice(&bytes[..n]);
    dst.func_len = n as u8;
}

#[derive(Clone, Copy, Debug, Default)]
#[allow(dead_code)]
pub struct ProxyStepData {
    pub step: i32,
    pub is_send: i32,
    pub start_ns: i64,
    pub stop_ns: i64,
    /// Entry timestamp per state (see `IDX_*`); 0 = state never entered.
    pub state_ts: [i64; PROXY_STEP_STATE_SLOTS],
    /// v4: this step's transferred bytes (from SendWait / RecvFlushWait
    /// state args; 0 on v3).
    pub trans_bytes: u64,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ProxyOpData {
    pub channel_id: i32,
    pub peer: i32,
    pub is_send: i32,
    pub n_steps: i32,
    pub trans_bytes: u64,
    pub coll: CollContext,
    pub start_ns: i64,
    pub stop_ns: i64,
    /// Waits accumulated incrementally as each step stops (keeping the raw
    /// per-step data would cost ~5 KB per slot for nothing — only the sums
    /// are ever emitted).
    pub waits: WaitBreakdown,
    /// v4: sum of per-step transferred bytes (0 on v3).
    pub step_bytes: u64,
    pub step_count: u16,
    pub parent_coll: u32,
}

impl Default for ProxyOpData {
    fn default() -> Self {
        Self {
            channel_id: 0,
            peer: 0,
            is_send: 0,
            n_steps: 0,
            trans_bytes: 0,
            coll: CollContext::default(),
            start_ns: 0,
            stop_ns: 0,
            waits: WaitBreakdown::default(),
            step_bytes: 0,
            step_count: 0,
            parent_coll: INVALID_IDX,
        }
    }
}

/// Coll-level metadata carried from the NCCL descriptor (also used for P2P).
#[derive(Clone, Copy, Debug, Default)]
pub struct CollPerfMeta {
    pub is_p2p: bool,
    pub peer: i32,
    pub count: u64,
    pub msg_bytes: u64,
    pub dtype: ShortName,
    pub algo: ShortName,
    pub proto: ShortName,
    pub n_channels: i32,
}

/// Collective / P2P slot with **reference-counted lifetime**.
///
/// NCCL semantics: coll `stopEvent` only marks the end of the **enqueue** —
/// the kernel and proxy work run on after it. Following the official
/// ext-profiler guidance, the slot stays alive until `stopped` is set *and*
/// `live_children` (proxy ops + kernel-channel events) drops to zero; the
/// real execution window is reconstructed from child event timestamps.
#[repr(C)]
pub struct CollSlot {
    pub tag: u8,
    pub self_idx: u32,
    pub ctx: CollContext,
    pub meta: CollPerfMeta,
    pub start_ns: i64,
    /// Host-side enqueue completion (NCCL coll `stopEvent`).
    pub enqueue_stop_ns: i64,
    pub stopped: bool,
    pub live_children: u16,
    /// Child events dropped because a fixed slot pool was full (timing may be degraded).
    pub pool_events_dropped: u16,
    /// Kernel-channel activity window (proxy thread observation of the kernel).
    pub kch_min_start_ns: i64,
    pub kch_max_stop_ns: i64,
    pub kch_count: u16,
    /// v4: kernel window on the **GPU globaltimer** clock (`pTimer` from the
    /// kernelCh descriptor + `KernelChStop` state). Only the duration is
    /// meaningful — the GPU clock has no epoch alignment with host time.
    pub kch_gpu_min_start: u64,
    pub kch_gpu_max_stop: u64,
    pub kch_gpu_count: u16,
    /// Proxy-op activity window.
    pub px_min_start_ns: i64,
    pub px_max_stop_ns: i64,
    pub pending: [CompletedProxyOp; MAX_PENDING_PER_COLL],
    pub pending_len: u16,
}

impl CollSlot {
    pub fn new(ctx: CollContext, meta: CollPerfMeta, start_ns: i64) -> Self {
        Self {
            tag: EVT_COLL,
            self_idx: INVALID_IDX,
            ctx,
            meta,
            start_ns,
            enqueue_stop_ns: 0,
            stopped: false,
            live_children: 0,
            pool_events_dropped: 0,
            kch_min_start_ns: 0,
            kch_max_stop_ns: 0,
            kch_count: 0,
            kch_gpu_min_start: 0,
            kch_gpu_max_stop: 0,
            kch_gpu_count: 0,
            px_min_start_ns: 0,
            px_max_stop_ns: 0,
            pending: std::array::from_fn(|_| CompletedProxyOp::default()),
            pending_len: 0,
        }
    }

    /// Ready for completion: enqueue finished and no child event still live.
    pub fn is_complete(&self) -> bool {
        self.stopped && self.live_children == 0
    }

    pub fn observe_kch(&mut self, start_ns: i64, stop_ns: i64) {
        if self.kch_count == 0 || start_ns < self.kch_min_start_ns {
            self.kch_min_start_ns = start_ns;
        }
        if stop_ns > self.kch_max_stop_ns {
            self.kch_max_stop_ns = stop_ns;
        }
        self.kch_count = self.kch_count.saturating_add(1);
    }

    /// Fold a v4 GPU-globaltimer kernel window (`pTimer` pair) into the coll.
    /// All channels of one GPU share the globaltimer, so min/max across
    /// channels is a valid aggregated kernel window.
    pub fn observe_kch_gpu(&mut self, start_ptimer: u64, stop_ptimer: u64) {
        if start_ptimer == 0 || stop_ptimer <= start_ptimer {
            return;
        }
        if self.kch_gpu_count == 0 || start_ptimer < self.kch_gpu_min_start {
            self.kch_gpu_min_start = start_ptimer;
        }
        if stop_ptimer > self.kch_gpu_max_stop {
            self.kch_gpu_max_stop = stop_ptimer;
        }
        self.kch_gpu_count = self.kch_gpu_count.saturating_add(1);
    }

    pub fn observe_proxy(&mut self, start_ns: i64, stop_ns: i64) {
        if self.px_min_start_ns == 0 || start_ns < self.px_min_start_ns {
            self.px_min_start_ns = start_ns;
        }
        if stop_ns > self.px_max_stop_ns {
            self.px_max_stop_ns = stop_ns;
        }
    }

    /// Build the completed coll-perf row, choosing the best available timing:
    /// GPU-globaltimer kernel window (v4) > host-observed kernel-channel
    /// window > proxy window > enqueue window. Bandwidth is algorithm
    /// bandwidth (`msg_bytes / exec_time`); bus bandwidth additionally needs
    /// the communicator size (`n_ranks`, v4 init; -1 on v3).
    pub fn coll_perf_row(&self) -> CompletedCollPerf {
        let (exec_time_ns, ts_ns, timing_source) =
            if self.kch_gpu_count > 0 && self.kch_gpu_max_stop > self.kch_gpu_min_start {
                (
                    (self.kch_gpu_max_stop - self.kch_gpu_min_start) as i64,
                    // GPU clock has no epoch alignment; stamp with the host
                    // observation (fall back to now).
                    if self.kch_max_stop_ns > 0 {
                        self.kch_max_stop_ns
                    } else {
                        now_ns()
                    },
                    TIMING_KERNEL_GPU,
                )
            } else if self.kch_count > 0 && self.kch_max_stop_ns > self.kch_min_start_ns {
                (
                    self.kch_max_stop_ns - self.kch_min_start_ns,
                    self.kch_max_stop_ns,
                    TIMING_KERNEL_CH,
                )
            } else if self.px_max_stop_ns > self.px_min_start_ns && self.px_min_start_ns > 0 {
                (
                    self.px_max_stop_ns - self.px_min_start_ns,
                    self.px_max_stop_ns,
                    TIMING_PROXY,
                )
            } else if self.enqueue_stop_ns > self.start_ns {
                (
                    self.enqueue_stop_ns - self.start_ns,
                    self.enqueue_stop_ns,
                    TIMING_ENQUEUE,
                )
            } else {
                (0, now_ns(), TIMING_ENQUEUE)
            };
        let enqueue_time_ns = if self.enqueue_stop_ns > self.start_ns {
            self.enqueue_stop_ns - self.start_ns
        } else {
            0
        };
        // bytes per nanosecond == decimal GB/s
        let algobw_gbps = if exec_time_ns > 0 {
            self.meta.msg_bytes as f64 / exec_time_ns as f64
        } else {
            0.0
        };
        CompletedCollPerf {
            ts_ns,
            rank: self.ctx.rank,
            roles: crate::role::cached(),
            comm_hash: self.ctx.comm_hash,
            n_ranks: self.ctx.n_ranks,
            coll_func: self.ctx.func,
            coll_func_len: self.ctx.func_len,
            seq: self.ctx.seq,
            is_p2p: self.meta.is_p2p,
            peer: self.meta.peer,
            count: self.meta.count,
            msg_size_bytes: self.meta.msg_bytes,
            dtype: self.meta.dtype,
            algo: self.meta.algo,
            proto: self.meta.proto,
            n_channels: self.meta.n_channels,
            exec_time_ns,
            enqueue_time_ns,
            timing_source,
            algobw_gbps,
            pool_events_dropped: self.pool_events_dropped as i32,
        }
    }

    pub fn push_pending(&mut self, row: CompletedProxyOp) -> bool {
        let n = self.pending_len as usize;
        if n >= MAX_PENDING_PER_COLL {
            return false;
        }
        self.pending[n] = row;
        self.pending_len += 1;
        true
    }
}

#[repr(C)]
pub struct ProxyOpSlot {
    pub tag: u8,
    pub self_idx: u32,
    pub op: ProxyOpData,
}

#[repr(C)]
pub struct ProxyStepSlot {
    pub tag: u8,
    pub self_idx: u32,
    pub step: ProxyStepData,
    pub parent_proxy: u32,
}

/// Kernel-channel event (`ncclProfileKernelCh`): the proxy progress thread's
/// observation of the GPU kernel working on a channel. This is NCCL's own
/// signal for actual kernel activity — the closest thing to device timing the
/// profiler ABI offers without CUDA instrumentation.
#[repr(C)]
pub struct KernelChSlot {
    pub tag: u8,
    pub self_idx: u32,
    pub parent_coll: u32,
    pub channel_id: i32,
    pub start_ns: i64,
    /// v4: GPU globaltimer at kernel start (descriptor `pTimer`; 0 on v3).
    pub gpu_start_ptimer: u64,
    /// v4: GPU globaltimer at `KernelChStop` (0 until reported).
    pub gpu_stop_ptimer: u64,
}

#[repr(C)]
pub struct NetPluginSlot {
    pub tag: u8,
    pub self_idx: u32,
    pub start_ns: i64,
    pub stop_ns: i64,
    pub rank: i32,
    pub device: i32,
    pub qp_num: i32,
    pub wr_id: u64,
    pub opcode: i32,
    pub length: u64,
}

macro_rules! impl_indexed {
    ($($ty:ty),+) => {$(
        impl Indexed for $ty {
            fn set_self_idx(&mut self, idx: u32) {
                self.self_idx = idx;
            }
            fn self_idx(&self) -> u32 {
                self.self_idx
            }
        }
    )+};
}

impl_indexed!(
    CollSlot,
    ProxyOpSlot,
    ProxyStepSlot,
    KernelChSlot,
    NetPluginSlot
);

#[inline]
pub fn proxy_step_state_index(state: NcclProfilerEventState) -> Option<usize> {
    match state {
        STATE_PROXY_STEP_SEND_GPU_WAIT => Some(IDX_SEND_GPU_WAIT),
        STATE_PROXY_STEP_SEND_PEER_WAIT_V4 => Some(IDX_SEND_PEER_WAIT),
        STATE_PROXY_STEP_SEND_WAIT => Some(IDX_SEND_WAIT),
        STATE_PROXY_STEP_RECV_WAIT => Some(IDX_RECV_WAIT),
        STATE_PROXY_STEP_RECV_FLUSH_WAIT => Some(IDX_RECV_FLUSH_WAIT),
        STATE_PROXY_STEP_RECV_GPU_WAIT => Some(IDX_RECV_GPU_WAIT),
        _ => None,
    }
}

/// Per-op wait decomposition (nanoseconds spent *in* each state).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaitBreakdown {
    pub send_gpu_wait_ns: i64,
    /// v4 only: waiting for the receiver's clear-to-send credits.
    pub send_peer_wait_ns: i64,
    pub send_wait_ns: i64,
    pub recv_wait_ns: i64,
    pub recv_flush_wait_ns: i64,
}

impl WaitBreakdown {
    fn add(&mut self, other: WaitBreakdown) {
        self.send_gpu_wait_ns += other.send_gpu_wait_ns;
        self.send_peer_wait_ns += other.send_peer_wait_ns;
        self.send_wait_ns += other.send_wait_ns;
        self.recv_wait_ns += other.recv_wait_ns;
        self.recv_flush_wait_ns += other.recv_flush_wait_ns;
    }
}

impl ProxyStepData {
    /// Dwell time per state: entry timestamp of a state to the entry of the
    /// next observed state in its chain (send: GpuWait → PeerWait → SendWait;
    /// recv: RecvWait → FlushWait → RecvGpuWait), the last one closing at
    /// `stop_ns`. Unentered states contribute zero.
    pub fn wait_deltas_ns(&self) -> WaitBreakdown {
        let send_chain = [IDX_SEND_GPU_WAIT, IDX_SEND_PEER_WAIT, IDX_SEND_WAIT];
        let recv_chain = [IDX_RECV_WAIT, IDX_RECV_FLUSH_WAIT, IDX_RECV_GPU_WAIT];
        let mut dwell = [0i64; PROXY_STEP_STATE_SLOTS];
        for chain in [&send_chain, &recv_chain] {
            for (pos, &idx) in chain.iter().enumerate() {
                let entered = self.state_ts[idx];
                if entered == 0 {
                    continue;
                }
                let next_entry = chain[pos + 1..]
                    .iter()
                    .map(|&n| self.state_ts[n])
                    .find(|&t| t != 0)
                    .unwrap_or(self.stop_ns);
                dwell[idx] = delta(entered, next_entry);
            }
        }
        WaitBreakdown {
            send_gpu_wait_ns: dwell[IDX_SEND_GPU_WAIT],
            send_peer_wait_ns: dwell[IDX_SEND_PEER_WAIT],
            send_wait_ns: dwell[IDX_SEND_WAIT],
            recv_wait_ns: dwell[IDX_RECV_WAIT],
            recv_flush_wait_ns: dwell[IDX_RECV_FLUSH_WAIT],
        }
    }
}

#[inline]
fn delta(from: i64, to: i64) -> i64 {
    if from == 0 || to == 0 || to < from {
        0
    } else {
        to - from
    }
}

impl ProxyOpData {
    /// Fold a finished step into the running totals (no per-step storage).
    pub fn push_step(&mut self, step: ProxyStepData) {
        self.waits.add(step.wait_deltas_ns());
        self.step_bytes += step.trans_bytes;
        self.step_count = self.step_count.saturating_add(1);
    }

    pub fn into_completed(self) -> CompletedProxyOp {
        let waits = self.waits;
        // v4 reports transferred bytes per step; prefer the summed step sizes
        // when present (v3 keeps reporting at the proxy-op level).
        let step_bytes = self.step_bytes;
        CompletedProxyOp {
            ts_ns: if self.stop_ns != 0 {
                self.stop_ns
            } else {
                now_ns()
            },
            rank: self.coll.rank,
            roles: crate::role::cached(),
            comm_hash: self.coll.comm_hash,
            coll_func: self.coll.func,
            coll_func_len: self.coll.func_len,
            seq: self.coll.seq,
            channel_id: self.channel_id,
            peer: self.peer,
            is_send: self.is_send,
            n_steps: self.n_steps,
            trans_bytes: if step_bytes > 0 {
                step_bytes.max(self.trans_bytes)
            } else {
                self.trans_bytes
            },
            send_gpu_wait_ns: waits.send_gpu_wait_ns,
            send_peer_wait_ns: waits.send_peer_wait_ns,
            send_wait_ns: waits.send_wait_ns,
            recv_wait_ns: waits.recv_wait_ns,
            recv_flush_wait_ns: waits.recv_flush_wait_ns,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CompletedProxyOp {
    pub ts_ns: i64,
    pub rank: i32,
    pub roles: RoleRanks,
    pub comm_hash: u64,
    pub coll_func: [u8; MAX_FUNC_NAME],
    pub coll_func_len: u8,
    pub seq: u64,
    pub channel_id: i32,
    pub peer: i32,
    pub is_send: i32,
    pub n_steps: i32,
    pub trans_bytes: u64,
    pub send_gpu_wait_ns: i64,
    /// v4 only: waiting for receiver clear-to-send credits (0 on v3).
    pub send_peer_wait_ns: i64,
    pub send_wait_ns: i64,
    pub recv_wait_ns: i64,
    pub recv_flush_wait_ns: i64,
}

impl CompletedProxyOp {
    pub fn func_str(&self) -> &str {
        let n = self.coll_func_len as usize;
        std::str::from_utf8(&self.coll_func[..n.min(MAX_FUNC_NAME)]).unwrap_or("unknown")
    }
}

/// One row of `nccl.coll_perf` (collective or P2P op level).
///
/// `exec_time_ns` comes from the best available NCCL-native signal
/// (`timing_source`: kernel_ch > proxy > enqueue); `enqueue_time_ns` is
/// always the host-side enqueue duration for comparison.
#[derive(Clone, Copy, Debug)]
pub struct CompletedCollPerf {
    pub ts_ns: i64,
    pub rank: i32,
    pub roles: RoleRanks,
    pub comm_hash: u64,
    /// Communicator size (v4; -1 on v3 — busbw conversion needs it).
    pub n_ranks: i32,
    pub coll_func: [u8; MAX_FUNC_NAME],
    pub coll_func_len: u8,
    pub seq: u64,
    pub is_p2p: bool,
    pub peer: i32,
    pub count: u64,
    pub msg_size_bytes: u64,
    pub dtype: ShortName,
    pub algo: ShortName,
    pub proto: ShortName,
    pub n_channels: i32,
    pub exec_time_ns: i64,
    pub enqueue_time_ns: i64,
    pub timing_source: &'static str,
    pub algobw_gbps: f64,
    /// Non-zero when child proxy/kernel slots were dropped (pool exhausted).
    pub pool_events_dropped: i32,
}

impl Default for CompletedCollPerf {
    fn default() -> Self {
        Self {
            ts_ns: 0,
            rank: 0,
            roles: RoleRanks::default(),
            comm_hash: 0,
            n_ranks: -1,
            coll_func: [0; MAX_FUNC_NAME],
            coll_func_len: 0,
            seq: 0,
            is_p2p: false,
            peer: -1,
            count: 0,
            msg_size_bytes: 0,
            dtype: ShortName::default(),
            algo: ShortName::default(),
            proto: ShortName::default(),
            n_channels: 0,
            exec_time_ns: 0,
            enqueue_time_ns: 0,
            timing_source: TIMING_ENQUEUE,
            algobw_gbps: 0.0,
            pool_events_dropped: 0,
        }
    }
}

impl CompletedCollPerf {
    pub fn func_str(&self) -> &str {
        let n = self.coll_func_len as usize;
        std::str::from_utf8(&self.coll_func[..n.min(MAX_FUNC_NAME)]).unwrap_or("unknown")
    }
}

/// One row of `nccl.inflight_ops` — a periodic snapshot of an operation that
/// started but has not stopped yet (hang visibility; see CoMMA's
/// `NCCL_PROFILER_NCCLOP_TIMEOUT` for the same idea).
#[derive(Clone, Copy, Debug)]
pub struct InflightOp {
    pub ts_ns: i64,
    pub rank: i32,
    pub comm_hash: u64,
    pub coll_func: [u8; MAX_FUNC_NAME],
    pub coll_func_len: u8,
    pub seq: u64,
    /// "coll" / "p2p" / "proxy_op"
    pub kind: &'static str,
    pub channel_id: i32,
    pub peer: i32,
    pub is_send: i32,
    pub start_ns: i64,
    pub age_ns: i64,
}

impl InflightOp {
    pub fn func_str(&self) -> &str {
        let n = self.coll_func_len as usize;
        std::str::from_utf8(&self.coll_func[..n.min(MAX_FUNC_NAME)]).unwrap_or("unknown")
    }
}

#[inline]
pub unsafe fn event_type(handle: *mut std::ffi::c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }
    *(handle as *const u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ns_is_epoch_aligned() {
        // Epoch nanoseconds are ~1.7e18 in the 2020s; a process-relative
        // clock would be many orders of magnitude smaller.
        let ts = now_ns();
        assert!(ts > 1_500_000_000_000_000_000, "ts={ts} not epoch-scale");
        let ts2 = now_ns();
        assert!(ts2 >= ts, "clock must be monotonic non-decreasing");
    }

    #[test]
    fn dtype_bytes_mapping() {
        assert_eq!(dtype_bytes(Some("ncclFloat16")), 2);
        assert_eq!(dtype_bytes(Some("ncclBfloat16")), 2);
        assert_eq!(dtype_bytes(Some("ncclFloat32")), 4);
        assert_eq!(dtype_bytes(Some("ncclInt64")), 8);
        assert_eq!(dtype_bytes(Some("ncclFloat8e4m3")), 1);
        assert_eq!(dtype_bytes(None), 1);
        assert_eq!(dtype_bytes(Some("bogus")), 1);
    }

    #[test]
    fn short_name_roundtrip_and_truncation() {
        assert_eq!(ShortName::from_str(Some("Ring")).as_str(), "Ring");
        assert_eq!(ShortName::from_str(None).as_str(), "");
        let long = "a".repeat(MAX_ALGO_NAME + 10);
        assert_eq!(
            ShortName::from_str(Some(&long)).as_str().len(),
            MAX_ALGO_NAME
        );
    }

    fn sample_slot() -> CollSlot {
        let mut ctx = CollContext {
            rank: 3,
            comm_hash: 42,
            seq: 7,
            ..Default::default()
        };
        copy_func_name(&mut ctx, Some("AllReduce"));
        let meta = CollPerfMeta {
            count: 1 << 20,
            msg_bytes: 2 << 20, // 2 MiB of fp16
            dtype: ShortName::from_str(Some("ncclFloat16")),
            algo: ShortName::from_str(Some("Ring")),
            proto: ShortName::from_str(Some("Simple")),
            n_channels: 4,
            ..Default::default()
        };
        CollSlot::new(ctx, meta, 1_000)
    }

    #[test]
    fn coll_perf_row_enqueue_fallback_computes_algobw() {
        let mut slot = sample_slot();
        slot.stopped = true;
        slot.enqueue_stop_ns = 1_000 + 1_000_000; // 1ms
        let row = slot.coll_perf_row();
        assert_eq!(row.timing_source, TIMING_ENQUEUE);
        assert_eq!(row.exec_time_ns, 1_000_000);
        assert_eq!(row.enqueue_time_ns, 1_000_000);
        assert_eq!(row.msg_size_bytes, 2 << 20);
        // 2 MiB / 1 ms ≈ 2.097 GB/s (bytes per ns == GB/s)
        assert!(
            (row.algobw_gbps - 2.097).abs() < 0.01,
            "{}",
            row.algobw_gbps
        );
        assert_eq!(row.func_str(), "AllReduce");
        assert!(!row.is_p2p);
    }

    #[test]
    fn coll_perf_row_prefers_gpu_timer_window() {
        let mut slot = sample_slot();
        slot.stopped = true;
        slot.enqueue_stop_ns = 1_000 + 50_000;
        slot.observe_proxy(2_000, 3_000_000);
        slot.observe_kch(1_500, 2_501_500); // host-observed window: 2.5ms
                                            // v4 GPU globaltimer window: 2.0ms (more precise, wins)
        slot.observe_kch_gpu(10_000_000, 12_000_000);
        let row = slot.coll_perf_row();
        assert_eq!(row.timing_source, TIMING_KERNEL_GPU);
        assert_eq!(row.exec_time_ns, 2_000_000);
        // ts stays on the host clock (GPU clock has no epoch alignment)
        assert_eq!(row.ts_ns, 2_501_500);
    }

    #[test]
    fn observe_kch_gpu_ignores_invalid_windows() {
        let mut slot = sample_slot();
        slot.observe_kch_gpu(0, 5_000); // missing start
        slot.observe_kch_gpu(5_000, 5_000); // zero-length
        slot.observe_kch_gpu(6_000, 4_000); // negative
        assert_eq!(slot.kch_gpu_count, 0);
    }

    #[test]
    fn wait_deltas_send_chain_with_peer_wait() {
        // v4 send chain: GpuWait@100 → PeerWait@400 → SendWait@600 → stop@1000
        let step = ProxyStepData {
            is_send: 1,
            start_ns: 50,
            stop_ns: 1_000,
            state_ts: {
                let mut ts = [0i64; PROXY_STEP_STATE_SLOTS];
                ts[IDX_SEND_GPU_WAIT] = 100;
                ts[IDX_SEND_PEER_WAIT] = 400;
                ts[IDX_SEND_WAIT] = 600;
                ts
            },
            ..Default::default()
        };
        let w = step.wait_deltas_ns();
        assert_eq!(w.send_gpu_wait_ns, 300); // 100 → 400
        assert_eq!(w.send_peer_wait_ns, 200); // 400 → 600
        assert_eq!(w.send_wait_ns, 400); // 600 → stop
        assert_eq!(w.recv_wait_ns, 0);
    }

    #[test]
    fn wait_deltas_v3_send_chain_without_peer_wait() {
        // v3 never enters PeerWait; SendGpuWait closes at SendWait directly.
        let step = ProxyStepData {
            is_send: 1,
            start_ns: 50,
            stop_ns: 1_000,
            state_ts: {
                let mut ts = [0i64; PROXY_STEP_STATE_SLOTS];
                ts[IDX_SEND_GPU_WAIT] = 100;
                ts[IDX_SEND_WAIT] = 700;
                ts
            },
            ..Default::default()
        };
        let w = step.wait_deltas_ns();
        assert_eq!(w.send_gpu_wait_ns, 600); // 100 → 700
        assert_eq!(w.send_peer_wait_ns, 0);
        assert_eq!(w.send_wait_ns, 300); // 700 → stop
    }

    #[test]
    fn wait_deltas_recv_chain() {
        let step = ProxyStepData {
            is_send: 0,
            start_ns: 50,
            stop_ns: 2_000,
            state_ts: {
                let mut ts = [0i64; PROXY_STEP_STATE_SLOTS];
                ts[IDX_RECV_WAIT] = 200;
                ts[IDX_RECV_FLUSH_WAIT] = 1_500;
                ts
            },
            ..Default::default()
        };
        let w = step.wait_deltas_ns();
        assert_eq!(w.recv_wait_ns, 1_300); // 200 → 1500
        assert_eq!(w.recv_flush_wait_ns, 500); // 1500 → stop
        assert_eq!(w.send_gpu_wait_ns, 0);
    }

    #[test]
    fn proxy_op_prefers_v4_step_bytes() {
        let mut op = ProxyOpData::default();
        let s1 = ProxyStepData {
            trans_bytes: 1_024,
            ..Default::default()
        };
        let s2 = ProxyStepData {
            trans_bytes: 2_048,
            ..Default::default()
        };
        op.push_step(s1);
        op.push_step(s2);
        op.trans_bytes = 100; // stale v3-style op-level value
        let row = op.into_completed();
        assert_eq!(row.trans_bytes, 3_072);
    }

    #[test]
    fn proxy_step_state_index_covers_v4_states() {
        use crate::abi::{STATE_PROXY_STEP_SEND_GPU_WAIT, STATE_PROXY_STEP_SEND_PEER_WAIT_V4};
        assert_eq!(
            proxy_step_state_index(STATE_PROXY_STEP_SEND_GPU_WAIT),
            Some(IDX_SEND_GPU_WAIT)
        );
        assert_eq!(
            proxy_step_state_index(STATE_PROXY_STEP_SEND_PEER_WAIT_V4),
            Some(IDX_SEND_PEER_WAIT)
        );
        assert_eq!(proxy_step_state_index(999), None);
    }

    #[test]
    fn coll_perf_row_prefers_kernel_ch_window() {
        let mut slot = sample_slot();
        slot.stopped = true;
        slot.enqueue_stop_ns = 1_000 + 50_000; // enqueue: 50µs
        slot.observe_proxy(2_000, 3_000_000); // proxy window ~3ms
        slot.observe_kch(1_500, 2_001_500); // kernel window: 2ms
        slot.observe_kch(1_800, 1_900_000); // second channel inside window
        let row = slot.coll_perf_row();
        assert_eq!(row.timing_source, TIMING_KERNEL_CH);
        assert_eq!(row.exec_time_ns, 2_000_000);
        assert_eq!(row.enqueue_time_ns, 50_000);
    }

    #[test]
    fn coll_perf_row_uses_proxy_window_without_kernel_ch() {
        let mut slot = sample_slot();
        slot.stopped = true;
        slot.enqueue_stop_ns = 1_000 + 50_000;
        slot.observe_proxy(10_000, 4_010_000); // 4ms of proxy activity
        let row = slot.coll_perf_row();
        assert_eq!(row.timing_source, TIMING_PROXY);
        assert_eq!(row.exec_time_ns, 4_000_000);
    }

    #[test]
    fn coll_perf_row_zero_duration_is_zero_bw() {
        let slot = CollSlot::new(CollContext::default(), CollPerfMeta::default(), 500);
        let row = slot.coll_perf_row();
        assert_eq!(row.algobw_gbps, 0.0);
    }

    #[test]
    fn refcount_lifecycle() {
        let mut slot = sample_slot();
        assert!(!slot.is_complete());
        slot.live_children += 2; // one proxy op + one kernel ch
        slot.stopped = true;
        slot.enqueue_stop_ns = 2_000;
        assert!(!slot.is_complete(), "children still live");
        slot.live_children -= 1;
        assert!(!slot.is_complete());
        slot.live_children -= 1;
        assert!(slot.is_complete());
    }
}
