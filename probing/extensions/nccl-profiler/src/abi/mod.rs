//! NCCL profiler plugin C ABI (v3, NCCL ≥ 2.26).
//!
//! Types mirror `ext-profiler/example/nccl/` from the NCCL repository.

#![allow(dead_code)]

pub mod net_ib_v1;
pub mod profiler_v3;

pub use profiler_v3::*;

/// `ncclResult_t` — success is zero.
pub type NcclResult = i32;

pub const NCCL_SUCCESS: NcclResult = 0;

#[inline]
pub const fn nccl_success() -> NcclResult {
    NCCL_SUCCESS
}
