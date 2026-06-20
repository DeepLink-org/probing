//! Network IB plugin event descriptor (NCCL profiler net plugin v1).

use std::os::raw::c_void;

pub const NCCL_PROFILER_NET_IB_VER: i64 = 1;
pub const NCCL_PROFILER_NET_VER_BITS: i32 = 16;
pub const NCCL_PROFILER_NET_VER_MASK: i64 = (!0i64) >> NCCL_PROFILER_NET_VER_BITS;
pub const NCCL_PROFILER_NET_TYPE_MASK: i64 = (!0i64) << NCCL_PROFILER_NET_VER_BITS;

pub const NCCL_PROFILER_NET_TYPE_IB: i64 = 1i64 << NCCL_PROFILER_NET_VER_BITS;

pub const NCCL_PROFILE_QP: u8 = 1 << 0;

#[repr(C)]
pub struct NcclProfilerNetIbDescrV1 {
    pub type_: u8,
    pub body: NcclProfilerNetIbBodyV1,
}

#[repr(C)]
pub union NcclProfilerNetIbBodyV1 {
    pub qp: NcclProfilerNetIbQpDescr,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NcclProfilerNetIbQpDescr {
    pub device: i32,
    pub wr_id: u64,
    pub opcode: i32,
    pub qp_num: i32,
    pub length: usize,
}

/// Decode plugin id from `netPlugin.id`.
#[inline]
pub fn net_plugin_type(id: i64) -> i64 {
    id & NCCL_PROFILER_NET_TYPE_MASK
}

#[inline]
pub fn net_plugin_ver(id: i64) -> i64 {
    id & NCCL_PROFILER_NET_VER_MASK
}

/// Safety: `data` must point to a valid `NcclProfilerNetIbDescrV1` when type is IB v1.
#[inline]
pub unsafe fn read_ib_qp(data: *mut c_void) -> Option<NcclProfilerNetIbQpDescr> {
    if data.is_null() {
        return None;
    }
    let d = &*(data as *const NcclProfilerNetIbDescrV1);
    if d.type_ != NCCL_PROFILE_QP {
        return None;
    }
    Some(d.body.qp)
}
