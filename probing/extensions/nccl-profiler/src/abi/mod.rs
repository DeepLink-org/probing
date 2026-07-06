//! NCCL profiler plugin C ABI (v3 for NCCL ≥ 2.26, v4 for NCCL ≥ 2.27).
//!
//! Types mirror `src/include/plugin/profiler/` from the NCCL repository.

#![allow(dead_code)]

pub mod net_ib_v1;
pub mod profiler_v3;
pub mod profiler_v4;

pub use profiler_v3::*;
pub use profiler_v4::{
    NcclProfilerCollDescrV4, NcclProfilerEventBodyV4, NcclProfilerEventDescrV4,
    NcclProfilerEventStateArgsV4, NcclProfilerEventStateV4, NcclProfilerKernelChDescrV4,
    NcclProfilerP2pDescrV4, NcclProfilerV4,
};

/// `ncclResult_t` — success is zero.
pub type NcclResult = i32;

pub const NCCL_SUCCESS: NcclResult = 0;

#[inline]
pub const fn nccl_success() -> NcclResult {
    NCCL_SUCCESS
}
