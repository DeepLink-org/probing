//! Plugin runtime: slot pools, event hierarchy, batch flush.

use std::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;

use parking_lot::Mutex;

use crate::abi::net_ib_v1::{net_plugin_type, read_ib_qp, NCCL_PROFILER_NET_TYPE_IB};
use crate::abi::{
    NcclProfilerEventDescrV3, NcclProfilerEventStateArgsV3, NcclProfilerEventStateV3,
    NCCL_PROFILE_COLL, NCCL_PROFILE_NET_PLUGIN, NCCL_PROFILE_PROXY_OP, NCCL_PROFILE_PROXY_STEP,
};
use crate::events::{
    copy_func_name, event_type, now_ns, proxy_step_state_index, CollContext, CollSlot,
    CompletedProxyOp, EventCounters, NetPluginSlot, ProxyOpData, ProxyOpSlot, ProxyStepData,
    ProxyStepSlot, EVT_COLL, EVT_NET_PLUGIN, EVT_PROXY_OP, EVT_PROXY_STEP, MAX_CHANNELS,
};
use crate::pool::{SlotPool, INVALID_IDX};
use crate::writer::{CompletedNetQp, NcclWriter};

const MAX_COLL_SLOTS: usize = 256;
const MAX_PROXY_OP_SLOTS: usize = 4096;
const MAX_PROXY_STEP_SLOTS: usize = 16384;
const MAX_NET_SLOTS: usize = 4096;

pub struct PluginState {
    inner: Mutex<PluginStateInner>,
}

pub struct PluginStateInner {
    pub counters: EventCounters,
    pub coll_pool: SlotPool<CollSlot>,
    pub proxy_pool: SlotPool<ProxyOpSlot>,
    pub step_pool: SlotPool<ProxyStepSlot>,
    pub net_pool: SlotPool<NetPluginSlot>,
    pub writer: NcclWriter,
}

impl PluginState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PluginStateInner {
                counters: EventCounters::new(),
                coll_pool: SlotPool::with_capacity(MAX_COLL_SLOTS),
                proxy_pool: SlotPool::with_capacity(MAX_PROXY_OP_SLOTS),
                step_pool: SlotPool::with_capacity(MAX_PROXY_STEP_SLOTS),
                net_pool: SlotPool::with_capacity(MAX_NET_SLOTS),
                writer: NcclWriter::new(),
            }),
        }
    }

    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut PluginStateInner) -> R,
    {
        let mut guard = self.inner.lock();
        f(&mut guard)
    }
}

fn cstr_opt(p: *const c_char) -> Option<&'static str> {
    if p.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(p) }.to_str().ok()
}

fn read_coll_context(descr: &NcclProfilerEventDescrV3) -> CollContext {
    let c = unsafe { descr.body.coll };
    let mut ctx = CollContext {
        rank: descr.rank,
        comm_hash: c.comm_hash,
        seq: c.seq_number,
        func: [0; crate::events::MAX_FUNC_NAME],
        func_len: 0,
    };
    copy_func_name(&mut ctx, cstr_opt(c.func));
    ctx
}

fn coll_context_fallback(descr: &NcclProfilerEventDescrV3) -> CollContext {
    let mut ctx = CollContext {
        rank: descr.rank,
        ..Default::default()
    };
    copy_func_name(&mut ctx, None);
    ctx
}

impl PluginStateInner {
    pub fn start_event(&mut self, handle: *mut *mut c_void, descr: &NcclProfilerEventDescrV3) {
        let t = descr.type_ as i32;
        match t {
            x if x == NCCL_PROFILE_COLL => {
                self.counters.coll.fetch_add(1, Ordering::Relaxed);
                let ctx = read_coll_context(descr);
                let ts = now_ns();
                if let Some((ptr, _idx)) = self.coll_pool.alloc(|| CollSlot::new(ctx, ts)) {
                    unsafe { *handle = ptr as *mut c_void };
                } else {
                    self.counters.pool_exhausted.fetch_add(1, Ordering::Relaxed);
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            x if x == NCCL_PROFILE_PROXY_OP => {
                self.counters.proxy_op.fetch_add(1, Ordering::Relaxed);
                let p = unsafe { descr.body.proxy_op };
                let channel = p.channel_id as usize;
                let (coll, parent_coll) = if descr.parent_obj.is_null() {
                    (coll_context_fallback(descr), INVALID_IDX)
                } else if let Some(coll_idx) =
                    self.coll_pool.index_of(descr.parent_obj as *mut CollSlot)
                {
                    if let Some(coll_slot) = self.coll_pool.get_mut(coll_idx) {
                        (coll_slot.ctx, coll_idx)
                    } else {
                        (coll_context_fallback(descr), INVALID_IDX)
                    }
                } else {
                    (coll_context_fallback(descr), INVALID_IDX)
                };

                if let Some((ptr, proxy_idx)) = self.proxy_pool.alloc(|| ProxyOpSlot {
                    tag: EVT_PROXY_OP,
                    op: ProxyOpData {
                        channel_id: p.channel_id as i32,
                        peer: p.peer,
                        is_send: p.is_send,
                        n_steps: p.n_steps,
                        coll,
                        start_ns: now_ns(),
                        parent_coll,
                        ..Default::default()
                    },
                }) {
                    if parent_coll != INVALID_IDX {
                        if let Some(coll_slot) = self.coll_pool.get_mut(parent_coll) {
                            coll_slot.register_proxy(channel, p.is_send, proxy_idx);
                        }
                    }
                    unsafe { *handle = ptr as *mut c_void };
                } else {
                    self.counters.pool_exhausted.fetch_add(1, Ordering::Relaxed);
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            x if x == NCCL_PROFILE_PROXY_STEP => {
                self.counters.proxy_step.fetch_add(1, Ordering::Relaxed);
                let parent_proxy = if descr.parent_obj.is_null() {
                    INVALID_IDX
                } else {
                    self.proxy_pool
                        .index_of(descr.parent_obj as *mut ProxyOpSlot)
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
                let step_id = unsafe { descr.body.proxy_step.step };
                if let Some((ptr, _)) = self.step_pool.alloc(|| ProxyStepSlot {
                    tag: EVT_PROXY_STEP,
                    parent_proxy,
                    step: ProxyStepData {
                        step: step_id,
                        is_send,
                        start_ns: now_ns(),
                        ..Default::default()
                    },
                }) {
                    unsafe { *handle = ptr as *mut c_void };
                } else {
                    self.counters.pool_exhausted.fetch_add(1, Ordering::Relaxed);
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            x if x == NCCL_PROFILE_NET_PLUGIN => {
                self.counters.net_plugin.fetch_add(1, Ordering::Relaxed);
                let id = unsafe { descr.body.net_plugin.id };
                let mut device = 0i32;
                let mut qp_num = 0i32;
                let mut wr_id = 0u64;
                let mut opcode = 0i32;
                let mut length = 0u64;
                if net_plugin_type(id) == NCCL_PROFILER_NET_TYPE_IB {
                    if let Some(qp) = unsafe { read_ib_qp(descr.body.net_plugin.data) } {
                        device = qp.device;
                        qp_num = qp.qp_num;
                        wr_id = qp.wr_id;
                        opcode = qp.opcode;
                        length = qp.length as u64;
                    }
                }
                if let Some((ptr, _)) = self.net_pool.alloc(|| NetPluginSlot {
                    tag: EVT_NET_PLUGIN,
                    start_ns: now_ns(),
                    rank: descr.rank,
                    device,
                    qp_num,
                    wr_id,
                    opcode,
                    length,
                    stop_ns: 0,
                }) {
                    unsafe { *handle = ptr as *mut c_void };
                } else {
                    self.counters.pool_exhausted.fetch_add(1, Ordering::Relaxed);
                    unsafe { *handle = std::ptr::null_mut() };
                }
            }
            _ => unsafe { *handle = std::ptr::null_mut() },
        }
    }

    pub fn stop_event(&mut self, handle: *mut c_void) {
        if handle.is_null() {
            return;
        }
        match unsafe { event_type(handle) } {
            EVT_PROXY_STEP => {
                let step_idx = self
                    .step_pool
                    .index_of(handle as *mut ProxyStepSlot)
                    .unwrap_or(INVALID_IDX);
                if step_idx == INVALID_IDX {
                    return;
                }
                let (parent_proxy, step) = {
                    let Some(slot) = self.step_pool.get_mut(step_idx) else {
                        return;
                    };
                    slot.step.stop_ns = now_ns();
                    (slot.parent_proxy, slot.step)
                };
                if parent_proxy != INVALID_IDX {
                    if let Some(proxy) = self.proxy_pool.get_mut(parent_proxy) {
                        if !proxy.op.push_step(step) {
                            self.counters.pool_exhausted.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                self.step_pool.free_idx(step_idx);
            }
            EVT_PROXY_OP => {
                let proxy_idx = self
                    .proxy_pool
                    .index_of(handle as *mut ProxyOpSlot)
                    .unwrap_or(INVALID_IDX);
                if proxy_idx == INVALID_IDX {
                    return;
                }
                let (parent_coll, row) = {
                    let Some(slot) = self.proxy_pool.get_mut(proxy_idx) else {
                        return;
                    };
                    slot.op.stop_ns = now_ns();
                    let parent = slot.op.parent_coll;
                    let row = std::mem::take(&mut slot.op).into_completed();
                    (parent, row)
                };
                self.flush_proxy_row(parent_coll, row);
                if parent_coll != INVALID_IDX {
                    if let Some(coll) = self.coll_pool.get_mut(parent_coll) {
                        let ch = row.channel_id as usize;
                        if row.is_send != 0 {
                            if ch < MAX_CHANNELS {
                                coll.send_ref[ch] = INVALID_IDX;
                            }
                        } else if ch < MAX_CHANNELS {
                            coll.recv_ref[ch] = INVALID_IDX;
                        }
                    }
                }
                self.proxy_pool.free_idx(proxy_idx);
            }
            EVT_COLL => {
                let coll_idx = self
                    .coll_pool
                    .index_of(handle as *mut CollSlot)
                    .unwrap_or(INVALID_IDX);
                if coll_idx == INVALID_IDX {
                    return;
                }
                let pending: Vec<CompletedProxyOp> = {
                    let Some(slot) = self.coll_pool.get_mut(coll_idx) else {
                        return;
                    };
                    slot.stop_ns = now_ns();
                    slot.pending[..slot.pending_len as usize].to_vec()
                };
                self.writer.flush_proxy_ops(&pending, &self.counters);
                self.coll_pool.free_idx(coll_idx);
            }
            EVT_NET_PLUGIN => {
                let net_idx = self
                    .net_pool
                    .index_of(handle as *mut NetPluginSlot)
                    .unwrap_or(INVALID_IDX);
                if net_idx == INVALID_IDX {
                    return;
                }
                let row = {
                    let Some(slot) = self.net_pool.get_mut(net_idx) else {
                        return;
                    };
                    slot.stop_ns = now_ns();
                    CompletedNetQp {
                        ts_ns: slot.stop_ns,
                        rank: slot.rank,
                        device: slot.device,
                        qp_num: slot.qp_num,
                        wr_id: slot.wr_id,
                        opcode: slot.opcode,
                        length: slot.length,
                        duration_ns: slot.stop_ns.saturating_sub(slot.start_ns),
                    }
                };
                self.writer.append_net_qp(&row, &self.counters);
                self.net_pool.free_idx(net_idx);
            }
            _ => {}
        }
    }

    fn flush_proxy_row(&mut self, parent_coll: u32, row: CompletedProxyOp) {
        if parent_coll != INVALID_IDX {
            if let Some(coll) = self.coll_pool.get_mut(parent_coll) {
                if coll.push_pending(row) {
                    return;
                }
                // pending full — flush coll batch early
                let pending: Vec<CompletedProxyOp> =
                    coll.pending[..coll.pending_len as usize].to_vec();
                coll.pending_len = 0;
                self.writer.flush_proxy_ops(&pending, &self.counters);
                if !coll.push_pending(row) {
                    self.writer.append_proxy_op(&row, &self.counters);
                }
                return;
            }
        }
        self.writer.append_proxy_op(&row, &self.counters);
    }

    pub fn record_state(
        &mut self,
        handle: *mut c_void,
        state: NcclProfilerEventStateV3,
        args: *const NcclProfilerEventStateArgsV3,
    ) {
        if handle.is_null() {
            return;
        }
        let ts = now_ns();
        match unsafe { event_type(handle) } {
            EVT_PROXY_STEP => {
                let step_idx = self
                    .step_pool
                    .index_of(handle as *mut ProxyStepSlot)
                    .unwrap_or(INVALID_IDX);
                if step_idx == INVALID_IDX {
                    return;
                }
                let Some(slot) = self.step_pool.get_mut(step_idx) else {
                    return;
                };
                if let Some(idx) = proxy_step_state_index(state) {
                    slot.step.state_ts[idx] = ts;
                }
            }
            EVT_PROXY_OP => {
                let proxy_idx = self
                    .proxy_pool
                    .index_of(handle as *mut ProxyOpSlot)
                    .unwrap_or(INVALID_IDX);
                if proxy_idx == INVALID_IDX {
                    return;
                }
                let Some(slot) = self.proxy_pool.get_mut(proxy_idx) else {
                    return;
                };
                if !args.is_null() {
                    unsafe {
                        let a = &*args;
                        slot.op.trans_bytes = a.proxy_op.trans_size as u64;
                        if a.proxy_op.steps > 0 {
                            slot.op.n_steps = a.proxy_op.steps;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn finalize_flush(&mut self) {
        for idx in 0..MAX_COLL_SLOTS as u32 {
            if let Some(coll) = self.coll_pool.get_mut(idx) {
                if coll.pending_len == 0 {
                    continue;
                }
                let pending: Vec<CompletedProxyOp> =
                    coll.pending[..coll.pending_len as usize].to_vec();
                self.writer.flush_proxy_ops(&pending, &self.counters);
                coll.pending_len = 0;
            }
        }
    }
}
