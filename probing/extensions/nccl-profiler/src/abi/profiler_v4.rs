//! `ncclProfiler_v4` descriptors (NCCL 2.27+).
//!
//! Differences vs v3 that matter to us:
//!
//! * `init` receives per-communicator metadata (`commName`, `commHash`,
//!   `nNodes`, `nRanks`, `rank`) — coll/p2p descriptors no longer carry
//!   `name`/`commHash`, and we finally learn the communicator size (busbw).
//! * `kernelCh` descriptors carry `pTimer` (GPU globaltimer, nanoseconds) and
//!   the new `KernelChStop` state reports the stop `pTimer` — a device-clock
//!   kernel window instead of a host-side observation.
//! * proxy step gains the `SendPeerWait` state (waiting for the receiver's
//!   clear-to-send credits) and `transSize` moves from proxy-op state args to
//!   proxy-step state args.

use std::os::raw::{c_char, c_int, c_void};

use super::{
    NcclProfilerNetPluginDescr, NcclProfilerProxyCtrlStateArgs, NcclProfilerProxyOpDescr,
    NcclProfilerProxyStepDescr,
};

pub type NcclProfilerEventStateV4 = c_int;

// ── Event descriptors ─────────────────────────────────────────────────

#[repr(C)]
pub struct NcclProfilerEventDescrV4 {
    pub type_: u8,
    pub parent_obj: *mut c_void,
    pub rank: c_int,
    pub body: NcclProfilerEventBodyV4,
}

#[repr(C)]
pub union NcclProfilerEventBodyV4 {
    pub coll: NcclProfilerCollDescrV4,
    pub p2p: NcclProfilerP2pDescrV4,
    pub proxy_op: NcclProfilerProxyOpDescr,
    pub proxy_step: NcclProfilerProxyStepDescr,
    pub kernel_ch: NcclProfilerKernelChDescrV4,
    pub net_plugin: NcclProfilerNetPluginDescr,
}

/// v4 coll: no `name`/`commHash` (provided by `init`), `nChannels` replaces
/// `nMaxChannels`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerCollDescrV4 {
    pub seq_number: u64,
    pub func: *const c_char,
    pub send_buff: *const c_void,
    pub recv_buff: *mut c_void,
    pub count: usize,
    pub root: c_int,
    pub datatype: *const c_char,
    pub n_channels: u8,
    pub n_warps: u8,
    pub algo: *const c_char,
    pub proto: *const c_char,
}

/// v4 p2p: no `name`/`commHash`, adds `nChannels`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerP2pDescrV4 {
    pub func: *const c_char,
    pub buff: *mut c_void,
    pub datatype: *const c_char,
    pub count: usize,
    pub peer: c_int,
    pub n_channels: u8,
}

/// v4 kernelCh: adds `pTimer` — GPU globaltimer start timestamp (ns domain).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerKernelChDescrV4 {
    pub channel_id: u8,
    pub p_timer: u64,
}

// ── State transition args ─────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NcclProfilerProxyStepStateArgsV4 {
    pub trans_size: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerNetPluginStateArgsV4 {
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NcclProfilerKernelChStateArgsV4 {
    /// GPU globaltimer stop timestamp (`KernelChStop`).
    pub p_timer: u64,
}

#[repr(C)]
pub union NcclProfilerEventStateArgsV4 {
    pub proxy_step: NcclProfilerProxyStepStateArgsV4,
    pub proxy_ctrl: NcclProfilerProxyCtrlStateArgs,
    pub net_plugin: NcclProfilerNetPluginStateArgsV4,
    pub kernel_ch: NcclProfilerKernelChStateArgsV4,
}

// ── Plugin vtable ─────────────────────────────────────────────────────

/// `ncclDebugLogger_t` — invoked with `fmt = "%s"` and one C string argument.
pub type NcclDebugLoggerFn = crate::log::NcclDebugLoggerFn;

pub type ProfilerInitV4Fn = unsafe extern "C" fn(
    context: *mut *mut c_void,
    mask: *mut c_int,
    comm_name: *const c_char,
    comm_hash: u64,
    n_nodes: c_int,
    n_ranks: c_int,
    rank: c_int,
    logfn: NcclDebugLoggerFn,
) -> super::NcclResult;
pub type ProfilerStartEventV4Fn = unsafe extern "C" fn(
    context: *mut c_void,
    handle: *mut *mut c_void,
    descr: *mut NcclProfilerEventDescrV4,
) -> super::NcclResult;
pub type ProfilerRecordStateV4Fn = unsafe extern "C" fn(
    handle: *mut c_void,
    state: NcclProfilerEventStateV4,
    args: *mut NcclProfilerEventStateArgsV4,
) -> super::NcclResult;

#[repr(C)]
pub struct NcclProfilerV4 {
    pub name: *const c_char,
    pub init: ProfilerInitV4Fn,
    pub start_event: ProfilerStartEventV4Fn,
    pub stop_event: super::ProfilerStopEventFn,
    pub record_event_state: ProfilerRecordStateV4Fn,
    pub finalize: super::ProfilerFinalizeFn,
}

// SAFETY: exported as a read-only C vtable; `name` points to static bytes.
unsafe impl Sync for NcclProfilerV4 {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    // Layout must match the C structs in NCCL's profiler_v4.h exactly.
    #[test]
    fn kernel_ch_descr_layout_matches_c() {
        assert_eq!(offset_of!(NcclProfilerKernelChDescrV4, channel_id), 0);
        assert_eq!(offset_of!(NcclProfilerKernelChDescrV4, p_timer), 8);
        assert_eq!(std::mem::size_of::<NcclProfilerKernelChDescrV4>(), 16);
    }

    #[test]
    fn coll_descr_layout_matches_c() {
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, seq_number), 0);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, func), 8);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, count), 32);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, root), 40);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, datatype), 48);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, n_channels), 56);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, n_warps), 57);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, algo), 64);
        assert_eq!(offset_of!(NcclProfilerCollDescrV4, proto), 72);
    }

    #[test]
    fn p2p_descr_layout_matches_c() {
        assert_eq!(offset_of!(NcclProfilerP2pDescrV4, func), 0);
        assert_eq!(offset_of!(NcclProfilerP2pDescrV4, count), 24);
        assert_eq!(offset_of!(NcclProfilerP2pDescrV4, peer), 32);
        assert_eq!(offset_of!(NcclProfilerP2pDescrV4, n_channels), 36);
    }
}
