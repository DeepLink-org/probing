//! Event slot layouts and wait aggregation.

use std::sync::atomic::AtomicU64;
use std::time::Instant;

use once_cell::sync::Lazy;

use crate::abi::NcclProfilerEventState;
use crate::pool::INVALID_IDX;
use crate::role::RoleRanks;

pub const MAX_CHANNELS: usize = 32;
pub const MAX_STEPS_PER_OP: usize = 64;
pub const MAX_FUNC_NAME: usize = 32;
pub const MAX_PENDING_PER_COLL: usize = 128;

pub const EVT_COLL: u8 = 1;
pub const EVT_PROXY_OP: u8 = 2;
pub const EVT_PROXY_STEP: u8 = 3;
pub const EVT_NET_PLUGIN: u8 = 4;

pub const PROXY_STEP_STATE_SLOTS: usize = 5;

static ORIGIN: Lazy<Instant> = Lazy::new(Instant::now);

#[derive(Debug, Default)]
pub struct EventCounters {
    pub coll: AtomicU64,
    pub proxy_op: AtomicU64,
    pub proxy_step: AtomicU64,
    pub net_plugin: AtomicU64,
    pub rows_written: AtomicU64,
    pub pool_exhausted: AtomicU64,
    pub write_errors: AtomicU64,
}

impl EventCounters {
    pub const fn new() -> Self {
        Self {
            coll: AtomicU64::new(0),
            proxy_op: AtomicU64::new(0),
            proxy_step: AtomicU64::new(0),
            net_plugin: AtomicU64::new(0),
            rows_written: AtomicU64::new(0),
            pool_exhausted: AtomicU64::new(0),
            write_errors: AtomicU64::new(0),
        }
    }
}

#[inline]
pub fn now_ns() -> i64 {
    ORIGIN.elapsed().as_nanos() as i64
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CollContext {
    pub rank: i32,
    pub comm_hash: u64,
    pub seq: u64,
    pub func: [u8; MAX_FUNC_NAME],
    pub func_len: u8,
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
    pub state_ts: [i64; PROXY_STEP_STATE_SLOTS],
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
    pub steps: [ProxyStepData; MAX_STEPS_PER_OP],
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
            steps: std::array::from_fn(|_| ProxyStepData::default()),
            step_count: 0,
            parent_coll: INVALID_IDX,
        }
    }
}

#[repr(C)]
pub struct CollSlot {
    pub tag: u8,
    pub ctx: CollContext,
    pub start_ns: i64,
    pub stop_ns: i64,
    pub send_ref: [u32; MAX_CHANNELS],
    pub recv_ref: [u32; MAX_CHANNELS],
    pub pending: [CompletedProxyOp; MAX_PENDING_PER_COLL],
    pub pending_len: u16,
}

impl CollSlot {
    pub fn new(ctx: CollContext, start_ns: i64) -> Self {
        Self {
            tag: EVT_COLL,
            ctx,
            start_ns,
            stop_ns: 0,
            send_ref: [INVALID_IDX; MAX_CHANNELS],
            recv_ref: [INVALID_IDX; MAX_CHANNELS],
            pending: std::array::from_fn(|_| CompletedProxyOp::default()),
            pending_len: 0,
        }
    }

    pub fn register_proxy(&mut self, channel: usize, is_send: i32, idx: u32) {
        if channel >= MAX_CHANNELS {
            return;
        }
        if is_send != 0 {
            self.send_ref[channel] = idx;
        } else {
            self.recv_ref[channel] = idx;
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
    pub op: ProxyOpData,
}

#[repr(C)]
pub struct ProxyStepSlot {
    pub tag: u8,
    pub step: ProxyStepData,
    pub parent_proxy: u32,
}

#[repr(C)]
pub struct NetPluginSlot {
    pub tag: u8,
    pub start_ns: i64,
    pub stop_ns: i64,
    pub rank: i32,
    pub device: i32,
    pub qp_num: i32,
    pub wr_id: u64,
    pub opcode: i32,
    pub length: u64,
}

#[inline]
pub fn proxy_step_state_index(state: NcclProfilerEventState) -> Option<usize> {
    use NcclProfilerEventState::*;
    match state {
        ProxyStepSendGpuWait => Some(0),
        ProxyStepSendWait => Some(1),
        ProxyStepRecvWait => Some(2),
        ProxyStepRecvFlushWait => Some(3),
        ProxyStepRecvGpuWait => Some(4),
        _ => None,
    }
}

impl ProxyStepData {
    pub fn wait_deltas_ns(&self) -> (i64, i64, i64, i64) {
        let base = self.start_ns;
        let gpu = delta(base, self.state_ts[0]);
        let send = delta(self.state_ts[0], self.state_ts[1]);
        let recv = delta(self.state_ts[1], self.state_ts[2]);
        let flush = delta(self.state_ts[2], self.state_ts[3]);
        (gpu, send, recv, flush)
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
    pub fn push_step(&mut self, step: ProxyStepData) -> bool {
        let n = self.step_count as usize;
        if n >= MAX_STEPS_PER_OP {
            return false;
        }
        self.steps[n] = step;
        self.step_count += 1;
        true
    }

    pub fn aggregate_waits(&self) -> (i64, i64, i64, i64) {
        let mut gpu = 0i64;
        let mut send = 0i64;
        let mut recv = 0i64;
        let mut flush = 0i64;
        for s in &self.steps[..self.step_count as usize] {
            let (g, se, r, f) = s.wait_deltas_ns();
            gpu += g;
            send += se;
            recv += r;
            flush += f;
        }
        (gpu, send, recv, flush)
    }

    pub fn into_completed(self) -> CompletedProxyOp {
        let (send_gpu_wait_ns, send_wait_ns, recv_wait_ns, recv_flush_wait_ns) =
            self.aggregate_waits();
        CompletedProxyOp {
            ts_ns: if self.stop_ns != 0 {
                self.stop_ns
            } else {
                now_ns()
            },
            rank: self.coll.rank,
            roles: crate::role::snapshot(),
            comm_hash: self.coll.comm_hash,
            coll_func: self.coll.func,
            coll_func_len: self.coll.func_len,
            seq: self.coll.seq,
            channel_id: self.channel_id,
            peer: self.peer,
            is_send: self.is_send,
            n_steps: self.n_steps,
            trans_bytes: self.trans_bytes,
            send_gpu_wait_ns,
            send_wait_ns,
            recv_wait_ns,
            recv_flush_wait_ns,
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

#[inline]
pub unsafe fn event_type(handle: *mut std::ffi::c_void) -> u8 {
    if handle.is_null() {
        return 0;
    }
    *(handle as *const u8)
}
