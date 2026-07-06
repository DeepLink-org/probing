//! Probing NCCL profiler plugin — exports `ncclProfiler_v4` (NCCL ≥ 2.27)
//! and `ncclProfiler_v3` (NCCL 2.26) on Linux; NCCL picks the highest one.

#![allow(clippy::missing_safety_doc)]
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

// Internal modules are `pub` + `doc(hidden)` so benches (`benches/`) can
// exercise the hot path; they are not a stable API.
#[doc(hidden)]
pub mod abi;
#[doc(hidden)]
pub mod events;
mod log;
#[doc(hidden)]
pub mod pool;
mod pool_config;
mod pool_pressure;
mod ring_config;
mod role;
mod shard;
mod tables;
mod writer;

#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod state;

#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod plugin;

pub use tables::{
    coll_perf_schema, inflight_ops_schema, net_qp_schema, profiler_counters_schema,
    proxy_ops_schema, register_docs, COLL_PERF_FILE, INFLIGHT_OPS_FILE, NET_QP_FILE,
    PROFILER_COUNTERS_FILE, PROXY_OPS_FILE,
};

#[cfg(target_os = "linux")]
mod export {
    use std::os::raw::c_char;

    use crate::abi::{NcclProfilerV3, NcclProfilerV4};
    use crate::plugin::{
        probing_profiler_finalize, probing_profiler_finalize_v4, probing_profiler_init,
        probing_profiler_init_v4, probing_profiler_record_state, probing_profiler_record_state_v4,
        probing_profiler_start_event, probing_profiler_start_event_v4, probing_profiler_stop_event,
    };

    static PLUGIN_NAME: &[u8] = b"probing-nccl-profiler\0";

    #[no_mangle]
    pub static ncclProfiler_v3: NcclProfilerV3 = NcclProfilerV3 {
        name: PLUGIN_NAME.as_ptr() as *const c_char,
        init: probing_profiler_init,
        start_event: probing_profiler_start_event,
        stop_event: probing_profiler_stop_event,
        record_event_state: probing_profiler_record_state,
        finalize: probing_profiler_finalize,
    };

    #[no_mangle]
    pub static ncclProfiler_v4: NcclProfilerV4 = NcclProfilerV4 {
        name: PLUGIN_NAME.as_ptr() as *const c_char,
        init: probing_profiler_init_v4,
        start_event: probing_profiler_start_event_v4,
        stop_event: probing_profiler_stop_event,
        record_event_state: probing_profiler_record_state_v4,
        finalize: probing_profiler_finalize_v4,
    };
}

#[cfg(not(target_os = "linux"))]
mod stub {
    //! Non-Linux builds omit the NCCL plugin symbol (dev machines / CI without NCCL).
    pub const BUILD_NOTE: &str = "probing-nccl-profiler: plugin symbol exported on Linux only";
}
