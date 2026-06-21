//! Probing NCCL profiler plugin — exports `ncclProfiler_v3` on Linux (NCCL ≥ 2.26).

#![allow(clippy::missing_safety_doc)]
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod abi;
mod events;
mod pool;
mod role;
mod tables;
mod writer;

#[cfg(target_os = "linux")]
mod state;

#[cfg(target_os = "linux")]
mod plugin;

pub use tables::{net_qp_schema, proxy_ops_schema, register_docs, NET_QP_FILE, PROXY_OPS_FILE};

#[cfg(target_os = "linux")]
mod export {
    use std::os::raw::c_char;

    use crate::abi::NcclProfilerV3;
    use crate::plugin::{
        probing_profiler_finalize, probing_profiler_init, probing_profiler_record_state,
        probing_profiler_start_event, probing_profiler_stop_event,
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
}

#[cfg(not(target_os = "linux"))]
mod stub {
    //! Non-Linux builds omit the NCCL plugin symbol (dev machines / CI without NCCL).
    pub const BUILD_NOTE: &str = "probing-nccl-profiler: plugin symbol exported on Linux only";
}
