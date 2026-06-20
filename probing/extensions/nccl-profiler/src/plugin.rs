//! NCCL profiler plugin callbacks (`ncclProfiler_v3`).

use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use crate::abi::{
    nccl_success, NcclProfilerEventDescrV3, NcclProfilerEventStateArgsV3, NcclProfilerEventStateV3,
    NcclResult, DEFAULT_ACTIVATION_MASK,
};
use crate::state::PluginState;

fn instance() -> &'static PluginState {
    static INSTANCE: OnceLock<PluginState> = OnceLock::new();
    INSTANCE.get_or_init(PluginState::new)
}

pub unsafe extern "C" fn probing_profiler_init(
    context: *mut *mut c_void,
    mask: *mut i32,
) -> NcclResult {
    if !mask.is_null() {
        let env = std::env::var("NCCL_PROFILE_EVENT_MASK")
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(DEFAULT_ACTIVATION_MASK);
        *mask = env;
    }

    let state = instance();
    if !context.is_null() {
        *context = state as *const PluginState as *mut c_void;
    }

    eprintln!(
        "[probing-nccl-profiler] init ok (mask={})",
        if mask.is_null() { 0 } else { *mask }
    );
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_start_event(
    context: *mut c_void,
    handle: *mut *mut c_void,
    descr: *mut NcclProfilerEventDescrV3,
) -> NcclResult {
    if descr.is_null() || handle.is_null() {
        return nccl_success();
    }
    let state = if context.is_null() {
        instance()
    } else {
        &*(context as *const PluginState)
    };
    state.with_mut(|inner| inner.start_event(handle, &*descr));
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_stop_event(handle: *mut c_void) -> NcclResult {
    instance().with_mut(|inner| inner.stop_event(handle));
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_record_state(
    handle: *mut c_void,
    state: NcclProfilerEventStateV3,
    args: *mut NcclProfilerEventStateArgsV3,
) -> NcclResult {
    instance().with_mut(|inner| inner.record_state(handle, state, args));
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_finalize(context: *mut c_void) -> NcclResult {
    let _ = context;
    let state = instance();
    let (coll, proxy_op, proxy_step, net, rows, pool_ex, write_err) = state.with_mut(|inner| {
        inner.finalize_flush();
        let c = &inner.counters;
        (
            c.coll.load(Ordering::Relaxed),
            c.proxy_op.load(Ordering::Relaxed),
            c.proxy_step.load(Ordering::Relaxed),
            c.net_plugin.load(Ordering::Relaxed),
            c.rows_written.load(Ordering::Relaxed),
            c.pool_exhausted.load(Ordering::Relaxed),
            c.write_errors.load(Ordering::Relaxed),
        )
    });
    eprintln!(
        "[probing-nccl-profiler] finalize: coll={coll} proxy_op={proxy_op} proxy_step={proxy_step} net={net} rows={rows} pool_exhausted={pool_ex} write_errors={write_err}"
    );
    nccl_success()
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;
    use crate::abi::NcclProfilerEventBodyV3;
    use crate::abi::NCCL_PROFILE_PROXY_OP;
    use std::sync::atomic::Ordering;

    #[test]
    fn init_sets_default_mask() {
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mask = 0i32;
        unsafe {
            assert_eq!(probing_profiler_init(&mut ctx, &mut mask), 0);
            assert_eq!(mask, DEFAULT_ACTIVATION_MASK);
            probing_profiler_finalize(ctx);
        }
    }

    #[test]
    fn proxy_op_with_parent_coll_batches_at_coll_stop() {
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mask = 0i32;
        unsafe {
            probing_profiler_init(&mut ctx, &mut mask).unwrap();
        }

        let base = std::env::temp_dir().join(format!("probing_nccl_p2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let mut coll_h: *mut c_void = std::ptr::null_mut();
        let mut proxy_h: *mut c_void = std::ptr::null_mut();

        let mut coll_descr = NcclProfilerEventDescrV3 {
            type_: NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 3,
            body: NcclProfilerEventBodyV3 {
                coll: {
                    let mut c: crate::abi::NcclProfilerCollDescr = unsafe { std::mem::zeroed() };
                    c.comm_hash = 99;
                    c.seq_number = 1;
                    c.func = b"AllReduce\0".as_ptr() as *const _;
                    c.n_max_channels = 4;
                    c
                },
            },
        };

        unsafe {
            probing_profiler_start_event(ctx, &mut coll_h, &mut coll_descr);

            let mut proxy_descr = NcclProfilerEventDescrV3 {
                type_: NCCL_PROFILE_PROXY_OP as u8,
                parent_obj: coll_h,
                rank: 3,
                body: NcclProfilerEventBodyV3 {
                    proxy_op: NcclProfilerProxyOpDescr {
                        pid: std::process::id() as i32,
                        channel_id: 0,
                        peer: 1,
                        n_steps: 2,
                        chunk_size: 1024,
                        is_send: 1,
                    },
                },
            };
            probing_profiler_start_event(ctx, &mut proxy_h, &mut proxy_descr);
            probing_profiler_stop_event(proxy_h);
            probing_profiler_stop_event(coll_h);
            probing_profiler_finalize(ctx);
        }

        let state = instance();
        let rows = state.with_mut(|inner| inner.counters.rows_written.load(Ordering::Relaxed));
        assert!(rows >= 1, "expected batch flush on coll stop");
        let _ = std::fs::remove_dir_all(&base);
    }

    trait NcclResultExt {
        fn unwrap(self) -> NcclResult;
    }

    impl NcclResultExt for NcclResult {
        fn unwrap(self) -> NcclResult {
            assert_eq!(self, 0);
            self
        }
    }
}
