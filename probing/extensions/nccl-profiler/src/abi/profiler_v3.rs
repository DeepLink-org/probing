//! `ncclProfiler_v3` and related descriptors (NCCL 2.26+).

use std::os::raw::{c_char, c_int, c_void};

// ── Event activation mask ─────────────────────────────────────────────

pub const NCCL_PROFILE_GROUP: i32 = 1 << 0;
pub const NCCL_PROFILE_COLL: i32 = 1 << 1;
pub const NCCL_PROFILE_P2P: i32 = 1 << 2;
pub const NCCL_PROFILE_PROXY_OP: i32 = 1 << 3;
pub const NCCL_PROFILE_PROXY_STEP: i32 = 1 << 4;
pub const NCCL_PROFILE_PROXY_CTRL: i32 = 1 << 5;
pub const NCCL_PROFILE_KERNEL_CH: i32 = 1 << 6;
pub const NCCL_PROFILE_NET_PLUGIN: i32 = 1 << 7;

/// Default events for Phase 2 (wait decomposition).
pub const DEFAULT_ACTIVATION_MASK: i32 =
    NCCL_PROFILE_COLL | NCCL_PROFILE_PROXY_OP | NCCL_PROFILE_PROXY_STEP;

// ── Event state (shared across profiler API versions) ─────────────────

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // mirrors NCCL C enum naming
pub enum NcclProfilerEventState {
    ProxyOpSendPosted = 0,
    ProxyOpSendRemFifoWait,
    ProxyOpSendTransmitted,
    ProxyOpSendDone,
    ProxyOpRecvPosted,
    ProxyOpRecvReceived,
    ProxyOpRecvTransmitted,
    ProxyOpRecvDone,

    ProxyStepSendGpuWait,
    ProxyStepSendWait,
    ProxyStepRecvWait,
    ProxyStepRecvFlushWait,
    ProxyStepRecvGpuWait,

    ProxyCtrlIdle,
    ProxyCtrlActive,
    ProxyCtrlSleep,
    ProxyCtrlWakeup,
    ProxyCtrlAppend,
    ProxyCtrlAppendEnd,
}

pub type NcclProfilerEventStateV3 = NcclProfilerEventState;

// ── Event descriptors ─────────────────────────────────────────────────

#[repr(C)]
pub struct NcclProfilerEventDescrV3 {
    pub type_: u8,
    pub parent_obj: *mut c_void,
    pub rank: c_int,
    pub body: NcclProfilerEventBodyV3,
}

#[repr(C)]
pub union NcclProfilerEventBodyV3 {
    pub coll: NcclProfilerCollDescr,
    pub p2p: NcclProfilerP2pDescr,
    pub proxy_op: NcclProfilerProxyOpDescr,
    pub proxy_step: NcclProfilerProxyStepDescr,
    pub kernel_ch: NcclProfilerKernelChDescr,
    pub net_plugin: NcclProfilerNetPluginDescr,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerCollDescr {
    pub name: *const c_char,
    pub comm_hash: u64,
    pub seq_number: u64,
    pub func: *const c_char,
    pub send_buff: *const c_void,
    pub recv_buff: *mut c_void,
    pub count: usize,
    pub root: c_int,
    pub datatype: *const c_char,
    pub n_max_channels: u8,
    pub n_warps: u8,
    pub algo: *const c_char,
    pub proto: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerP2pDescr {
    pub name: *const c_char,
    pub comm_hash: u64,
    pub func: *const c_char,
    pub buff: *mut c_void,
    pub datatype: *const c_char,
    pub count: usize,
    pub peer: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerProxyOpDescr {
    pub pid: i32,
    pub channel_id: u8,
    pub peer: c_int,
    pub n_steps: c_int,
    pub chunk_size: c_int,
    pub is_send: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerProxyStepDescr {
    pub step: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerKernelChDescr {
    pub channel_id: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerNetPluginDescr {
    pub id: i64,
    pub data: *mut c_void,
}

// ── State transition args ─────────────────────────────────────────────

#[repr(C)]
pub union NcclProfilerEventStateArgsV3 {
    pub proxy_op: NcclProfilerProxyOpStateArgs,
    pub proxy_ctrl: NcclProfilerProxyCtrlStateArgs,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NcclProfilerProxyOpStateArgs {
    pub trans_size: usize,
    pub steps: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NcclProfilerProxyCtrlStateArgs {
    pub appended_proxy_ops: c_int,
}

// ── Plugin vtable ─────────────────────────────────────────────────────

pub type ProfilerInitFn =
    unsafe extern "C" fn(context: *mut *mut c_void, mask: *mut c_int) -> super::NcclResult;
pub type ProfilerStartEventFn = unsafe extern "C" fn(
    context: *mut c_void,
    handle: *mut *mut c_void,
    descr: *mut NcclProfilerEventDescrV3,
) -> super::NcclResult;
pub type ProfilerStopEventFn = unsafe extern "C" fn(handle: *mut c_void) -> super::NcclResult;
pub type ProfilerRecordStateFn = unsafe extern "C" fn(
    handle: *mut c_void,
    state: NcclProfilerEventStateV3,
    args: *mut NcclProfilerEventStateArgsV3,
) -> super::NcclResult;
pub type ProfilerFinalizeFn = unsafe extern "C" fn(context: *mut c_void) -> super::NcclResult;

#[repr(C)]
pub struct NcclProfilerV3 {
    pub name: *const c_char,
    pub init: ProfilerInitFn,
    pub start_event: ProfilerStartEventFn,
    pub stop_event: ProfilerStopEventFn,
    pub record_event_state: ProfilerRecordStateFn,
    pub finalize: ProfilerFinalizeFn,
}

// SAFETY: exported as a read-only C vtable; `name` points to `PLUGIN_NAME` static bytes.
unsafe impl Sync for NcclProfilerV3 {}
