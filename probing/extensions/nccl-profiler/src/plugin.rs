//! NCCL profiler plugin callbacks (`ncclProfiler_v3` + `ncclProfiler_v4`).
//!
//! Both versions funnel into the same version-independent [`PluginState`]
//! (see `state::ParsedEvent` / `state::StateArgs`); only descriptor parsing
//! and the `init` signature differ.

use std::ffi::{c_char, c_void};
use std::os::raw::c_int;
use std::sync::atomic::Ordering;
use std::sync::{Once, OnceLock};
use std::time::Duration;

use crate::abi::profiler_v4::{
    NcclDebugLoggerFn, NcclProfilerEventDescrV4, NcclProfilerEventStateArgsV4,
    NcclProfilerEventStateV4,
};
use crate::abi::{
    nccl_success, NcclProfilerEventDescrV3, NcclProfilerEventStateArgsV3, NcclProfilerEventStateV3,
    NcclResult, DEFAULT_ACTIVATION_MASK, STATE_KERNEL_CH_STOP, STATE_PROXY_STEP_RECV_FLUSH_WAIT,
    STATE_PROXY_STEP_SEND_WAIT,
};
use crate::events::{event_type, EVT_PROXY_OP};
use crate::log;
use crate::state::{CommInfo, PluginState, StateArgs};

fn instance() -> &'static PluginState {
    static INSTANCE: OnceLock<PluginState> = OnceLock::new();
    INSTANCE.get_or_init(PluginState::new)
}

fn resolve_mask() -> i32 {
    std::env::var("NCCL_PROFILE_EVENT_MASK")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(DEFAULT_ACTIVATION_MASK)
}

/// Spawn the in-flight watchdog once per process.
///
/// Ops still live after `PROBING_NCCL_INFLIGHT_THRESHOLD_SECS` (default 10,
/// same default as CoMMA's `NCCL_PROFILER_NCCLOP_TIMEOUT`) are snapshotted
/// into `nccl.inflight_ops`; `0` disables the watchdog. NCCL calls `init`
/// once per communicator, hence the `Once` guard.
fn spawn_inflight_watchdog() {
    static SPAWNED: Once = Once::new();
    SPAWNED.call_once(|| {
        let threshold_secs = std::env::var("PROBING_NCCL_INFLIGHT_THRESHOLD_SECS")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(10);
        if threshold_secs == 0 {
            return;
        }
        let threshold_ns = (threshold_secs * 1_000_000_000) as i64;
        let interval = Duration::from_secs((threshold_secs / 2).max(1));
        let _ = std::thread::Builder::new()
            .name("probing-nccl-watchdog".into())
            .spawn(move || loop {
                std::thread::sleep(interval);
                instance().snapshot_inflight(threshold_ns);
            });
    });
}

fn log_finalize_counters(state: &PluginState) {
    let c = &state.counters;
    let pool_exhausted = c.pool_exhausted.load(Ordering::Relaxed);
    let write_errors = c.write_errors.load(Ordering::Relaxed);
    let msg = format!(
        "finalize: coll={} p2p={} proxy_op={} proxy_step={} kernel_ch={} net={} rows={} pool_exhausted={pool_exhausted} write_errors={write_errors} filtered={}",
        c.coll.load(Ordering::Relaxed),
        c.p2p.load(Ordering::Relaxed),
        c.proxy_op.load(Ordering::Relaxed),
        c.proxy_step.load(Ordering::Relaxed),
        c.kernel_ch.load(Ordering::Relaxed),
        c.net_plugin.load(Ordering::Relaxed),
        c.rows_written.load(Ordering::Relaxed),
        c.filtered.load(Ordering::Relaxed),
    );
    if pool_exhausted > 0 || write_errors > 0 {
        log::warn(msg);
    } else {
        log::info(msg);
    }
}

// ── v3 callbacks (NCCL 2.26) ──────────────────────────────────────────

pub unsafe extern "C" fn probing_profiler_init(
    context: *mut *mut c_void,
    mask: *mut i32,
) -> NcclResult {
    if !mask.is_null() {
        *mask = resolve_mask();
    }

    let state = instance();
    if !context.is_null() {
        *context = state as *const PluginState as *mut c_void;
    }
    spawn_inflight_watchdog();
    log::ensure_rust_logger();

    static FIRST_INIT_V3: Once = Once::new();
    let verbose = std::env::var("PROBING_NCCL_VERBOSE").is_ok_and(|v| v == "1");
    let mut log_line = verbose;
    FIRST_INIT_V3.call_once(|| log_line = true);
    if log_line {
        let mask_val = if mask.is_null() { 0 } else { *mask };
        log::info(format!("init ok (abi=v3, mask={mask_val})"));
    }
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
    state.start_event(handle, &*descr);
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_stop_event(handle: *mut c_void) -> NcclResult {
    instance().stop_event(handle);
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_record_state(
    handle: *mut c_void,
    state: NcclProfilerEventStateV3,
    args: *mut NcclProfilerEventStateArgsV3,
) -> NcclResult {
    if handle.is_null() {
        return nccl_success();
    }
    // The v3 args union is only meaningful on proxy-op events.
    let parsed = if !args.is_null() && event_type(handle) == EVT_PROXY_OP {
        let a = &*args;
        StateArgs::ProxyOpV3 {
            trans_size: a.proxy_op.trans_size as u64,
            steps: a.proxy_op.steps,
        }
    } else {
        StateArgs::None
    };
    instance().record_state(handle, state, parsed);
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_finalize(context: *mut c_void) -> NcclResult {
    let _ = context;
    let state = instance();
    state.finalize_flush();
    log_finalize_counters(state);
    nccl_success()
}

// ── v4 callbacks (NCCL 2.27+) ─────────────────────────────────────────

/// Per-communicator context handed back to NCCL as `void* context`.
struct CommCtxV4 {
    info: CommInfo,
}

/// Live v4 communicator contexts (finalize logs counters only when the last
/// one goes away, instead of once per communicator).
static LIVE_COMMS_V4: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn probing_profiler_init_v4(
    context: *mut *mut c_void,
    mask: *mut c_int,
    comm_name: *const c_char,
    comm_hash: u64,
    n_nodes: c_int,
    n_ranks: c_int,
    rank: c_int,
    logfn: NcclDebugLoggerFn,
) -> NcclResult {
    log::set_nccl_logger(logfn);
    if !mask.is_null() {
        *mask = resolve_mask();
    }

    let info = CommInfo { comm_hash, n_ranks };
    if !context.is_null() {
        *context = Box::into_raw(Box::new(CommCtxV4 { info })) as *mut c_void;
        LIVE_COMMS_V4.fetch_add(1, Ordering::Relaxed);
    }
    // Make sure the global state (pools/writer) is alive before callbacks.
    let _ = instance();
    spawn_inflight_watchdog();

    // v4 init runs once per communicator — a large job creates dozens per
    // rank. Log the first at full detail; the rest only when asked.
    static FIRST_INIT: Once = Once::new();
    let verbose = std::env::var("PROBING_NCCL_VERBOSE").is_ok_and(|v| v == "1");
    let mut log_line = verbose;
    FIRST_INIT.call_once(|| log_line = true);
    if log_line {
        let name = if comm_name.is_null() {
            ""
        } else {
            std::ffi::CStr::from_ptr(comm_name).to_str().unwrap_or("")
        };
        log::info(format!(
            "init ok (abi=v4, mask={}, comm={name} hash={comm_hash:#x} nodes={n_nodes} ranks={n_ranks} rank={rank})",
            if mask.is_null() { 0 } else { *mask },
        ));
    }
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_start_event_v4(
    context: *mut c_void,
    handle: *mut *mut c_void,
    descr: *mut NcclProfilerEventDescrV4,
) -> NcclResult {
    if descr.is_null() || handle.is_null() {
        return nccl_success();
    }
    let comm = if context.is_null() {
        CommInfo::default()
    } else {
        (*(context as *const CommCtxV4)).info
    };
    instance().start_event_v4(handle, &*descr, comm);
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_record_state_v4(
    handle: *mut c_void,
    state: NcclProfilerEventStateV4,
    args: *mut NcclProfilerEventStateArgsV4,
) -> NcclResult {
    if handle.is_null() {
        return nccl_success();
    }
    let parsed = if args.is_null() {
        StateArgs::None
    } else {
        match state {
            // NCCL stamps `proxyStep.transSize` on *every* step-state report,
            // but the value is only freshly assigned for the current step
            // right before SendWait / RecvFlushWait (net.cc); at other states
            // it may hold a different in-flight step's size. Only capture it
            // where it is authoritative (same rule as NCCL's example plugin).
            STATE_PROXY_STEP_SEND_WAIT | STATE_PROXY_STEP_RECV_FLUSH_WAIT => {
                StateArgs::ProxyStepV4 {
                    trans_size: (*args).proxy_step.trans_size as u64,
                }
            }
            STATE_KERNEL_CH_STOP => StateArgs::KernelChStop {
                p_timer: (*args).kernel_ch.p_timer,
            },
            _ => StateArgs::None,
        }
    };
    instance().record_state(handle, state, parsed);
    nccl_success()
}

pub unsafe extern "C" fn probing_profiler_finalize_v4(context: *mut c_void) -> NcclResult {
    let mut last_comm = false;
    if !context.is_null() {
        // Reclaim the per-comm context allocated in `init_v4`.
        drop(Box::from_raw(context as *mut CommCtxV4));
        last_comm = LIVE_COMMS_V4.fetch_sub(1, Ordering::Relaxed) == 1;
    }
    let state = instance();
    state.finalize_flush();
    if last_comm {
        log_finalize_counters(state);
    }
    nccl_success()
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;
    use crate::abi::NcclProfilerEventBodyV3;
    use crate::abi::{
        NcclProfilerProxyOpDescr, NCCL_PROFILE_COLL, NCCL_PROFILE_KERNEL_CH, NCCL_PROFILE_PROXY_OP,
    };
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;

    /// Tests share the process-global plugin instance; serialize them so
    /// row-count assertions don't race.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn init_sets_default_mask() {
        let _guard = serial();
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mask = 0i32;
        unsafe {
            assert_eq!(probing_profiler_init(&mut ctx, &mut mask), 0);
            assert_eq!(mask, DEFAULT_ACTIVATION_MASK);
            probing_profiler_finalize(ctx);
        }
    }

    fn make_coll_descr(rank: i32, seq: u64) -> NcclProfilerEventDescrV3 {
        NcclProfilerEventDescrV3 {
            type_: NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank,
            body: NcclProfilerEventBodyV3 {
                coll: {
                    let mut c: crate::abi::NcclProfilerCollDescr = unsafe { std::mem::zeroed() };
                    c.comm_hash = 99;
                    c.seq_number = seq;
                    c.func = c"AllReduce".as_ptr();
                    c.count = 4096;
                    c.datatype = c"ncclFloat32".as_ptr();
                    c.n_max_channels = 4;
                    c
                },
            },
        }
    }

    #[test]
    fn proxy_op_with_parent_coll_batches_at_coll_stop() {
        let _guard = serial();
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

        let mut coll_descr = make_coll_descr(3, 1);

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

        let rows = instance().counters.rows_written.load(Ordering::Relaxed);
        assert!(rows >= 1, "expected batch flush on coll stop");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn kernel_ch_extends_coll_lifetime_and_sets_timing() {
        let _guard = serial();
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mask = 0i32;
        unsafe {
            probing_profiler_init(&mut ctx, &mut mask).unwrap();
        }
        assert_ne!(mask & NCCL_PROFILE_KERNEL_CH, 0, "KernelCh in default mask");

        let base = std::env::temp_dir().join(format!("probing_nccl_kch_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let before = instance().counters.rows_written.load(Ordering::Relaxed);

        let mut coll_descr = make_coll_descr(2, 5);
        let mut coll_h: *mut c_void = std::ptr::null_mut();
        let mut kch_h: *mut c_void = std::ptr::null_mut();
        unsafe {
            probing_profiler_start_event(ctx, &mut coll_h, &mut coll_descr);
            assert!(!coll_h.is_null());

            let mut kch_descr = NcclProfilerEventDescrV3 {
                type_: NCCL_PROFILE_KERNEL_CH as u8,
                parent_obj: coll_h,
                rank: 2,
                body: NcclProfilerEventBodyV3 {
                    kernel_ch: crate::abi::NcclProfilerKernelChDescr { channel_id: 1 },
                },
            };
            probing_profiler_start_event(ctx, &mut kch_h, &mut kch_descr);
            assert!(!kch_h.is_null(), "kernelCh with live parent must track");

            // NCCL semantics: coll stopEvent = enqueue done; kernel still runs.
            probing_profiler_stop_event(coll_h);
            let mid = instance().counters.rows_written.load(Ordering::Relaxed);
            assert_eq!(mid, before, "coll must not complete while kernelCh live");

            probing_profiler_stop_event(kch_h);
        }

        let after = instance().counters.rows_written.load(Ordering::Relaxed);
        assert!(after > before, "kernelCh stop must complete the coll");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn p2p_event_writes_coll_perf_row() {
        let _guard = serial();
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mask = 0i32;
        unsafe {
            probing_profiler_init(&mut ctx, &mut mask).unwrap();
        }
        assert_ne!(
            mask & crate::abi::NCCL_PROFILE_P2P,
            0,
            "P2P in default mask"
        );

        let base = std::env::temp_dir().join(format!("probing_nccl_p2p_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let before = instance().counters.rows_written.load(Ordering::Relaxed);

        let mut p2p_descr = NcclProfilerEventDescrV3 {
            type_: crate::abi::NCCL_PROFILE_P2P as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 4,
            body: NcclProfilerEventBodyV3 {
                p2p: {
                    let mut p: crate::abi::NcclProfilerP2pDescr = unsafe { std::mem::zeroed() };
                    p.comm_hash = 7;
                    p.func = c"Send".as_ptr();
                    p.count = 65536;
                    p.datatype = c"ncclFloat16".as_ptr();
                    p.peer = 5;
                    p
                },
            },
        };
        let mut h: *mut c_void = std::ptr::null_mut();
        unsafe {
            probing_profiler_start_event(ctx, &mut h, &mut p2p_descr);
            assert!(!h.is_null(), "P2P event must allocate a slot");
            probing_profiler_stop_event(h);
        }

        let after = instance().counters.rows_written.load(Ordering::Relaxed);
        assert!(after > before, "expected a coll_perf row for P2P op");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn inflight_snapshot_sees_unstopped_coll() {
        let _guard = serial();
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mask = 0i32;
        unsafe {
            probing_profiler_init(&mut ctx, &mut mask).unwrap();
        }

        let base =
            std::env::temp_dir().join(format!("probing_nccl_inflight_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let mut coll_descr = NcclProfilerEventDescrV3 {
            type_: NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 1,
            body: NcclProfilerEventBodyV3 {
                coll: {
                    let mut c: crate::abi::NcclProfilerCollDescr = unsafe { std::mem::zeroed() };
                    c.comm_hash = 11;
                    c.seq_number = 3;
                    c.func = c"AllGather".as_ptr();
                    c
                },
            },
        };
        let mut h: *mut c_void = std::ptr::null_mut();
        unsafe {
            probing_profiler_start_event(ctx, &mut h, &mut coll_descr);
            assert!(!h.is_null());
        }

        // age threshold 0 → the just-started coll qualifies immediately
        let snapshotted = instance().snapshot_inflight(0);
        assert!(snapshotted >= 1, "unstopped coll must appear in snapshot");

        unsafe { probing_profiler_stop_event(h) };
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn v4_lifecycle_with_gpu_timer_and_comm_info() {
        use crate::abi::profiler_v4::{
            NcclProfilerCollDescrV4, NcclProfilerEventBodyV4, NcclProfilerEventStateArgsV4,
            NcclProfilerKernelChDescrV4, NcclProfilerKernelChStateArgsV4,
        };

        let _guard = serial();
        let mut ctx: *mut c_void = std::ptr::null_mut();
        let mut mask = 0i32;
        unsafe {
            probing_profiler_init_v4(
                &mut ctx,
                &mut mask,
                c"train_comm".as_ptr(),
                0xABCD,
                2,  // n_nodes
                16, // n_ranks
                3,  // rank
                None,
            )
            .unwrap();
        }
        assert!(!ctx.is_null(), "v4 init must allocate a comm context");
        assert_eq!(mask, DEFAULT_ACTIVATION_MASK);

        let base = std::env::temp_dir().join(format!("probing_nccl_v4_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let before = instance().counters.rows_written.load(Ordering::Relaxed);

        let mut coll_descr = NcclProfilerEventDescrV4 {
            type_: crate::abi::NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 3,
            body: NcclProfilerEventBodyV4 {
                coll: {
                    let mut c: NcclProfilerCollDescrV4 = unsafe { std::mem::zeroed() };
                    c.seq_number = 9;
                    c.func = c"AllReduce".as_ptr();
                    c.count = 4096;
                    c.datatype = c"ncclFloat32".as_ptr();
                    c.n_channels = 4;
                    c
                },
            },
        };
        let mut coll_h: *mut c_void = std::ptr::null_mut();
        let mut kch_h: *mut c_void = std::ptr::null_mut();
        unsafe {
            probing_profiler_start_event_v4(ctx, &mut coll_h, &mut coll_descr);
            assert!(!coll_h.is_null());

            let mut kch_descr = NcclProfilerEventDescrV4 {
                type_: crate::abi::NCCL_PROFILE_KERNEL_CH as u8,
                parent_obj: coll_h,
                rank: 3,
                body: NcclProfilerEventBodyV4 {
                    kernel_ch: NcclProfilerKernelChDescrV4 {
                        channel_id: 0,
                        p_timer: 5_000_000, // GPU globaltimer start
                    },
                },
            };
            probing_profiler_start_event_v4(ctx, &mut kch_h, &mut kch_descr);
            assert!(!kch_h.is_null());

            // v4: KernelChStop carries the GPU globaltimer stop timestamp.
            let mut args = NcclProfilerEventStateArgsV4 {
                kernel_ch: NcclProfilerKernelChStateArgsV4 { p_timer: 7_500_000 },
            };
            probing_profiler_record_state_v4(kch_h, crate::abi::STATE_KERNEL_CH_STOP, &mut args);

            probing_profiler_stop_event(coll_h);
            probing_profiler_stop_event(kch_h);
            probing_profiler_finalize_v4(ctx);
        }

        let after = instance().counters.rows_written.load(Ordering::Relaxed);
        assert!(after > before, "v4 coll must emit a coll_perf row");
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
