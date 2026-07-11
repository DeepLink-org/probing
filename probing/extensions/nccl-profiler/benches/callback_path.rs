//! Criterion benchmarks for the NCCL callback hot path.
//!
//! ```text
//! cargo bench -p probing-nccl-profiler --bench callback_path
//! ```
//!
//! Full callback-cycle benches run on Linux only; clock and pool benches run
//! everywhere.

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

use probing_nccl_profiler::events::{now_ns, CollContext, CollPerfMeta, CollSlot};
use probing_nccl_profiler::pool::SlotPool;

fn bench_clock(c: &mut Criterion) {
    c.bench_function("now_ns", |b| b.iter(|| black_box(now_ns())));
}

fn bench_pool(c: &mut Criterion) {
    let mut pool: SlotPool<CollSlot> = SlotPool::with_capacity(256);
    c.bench_function("pool_alloc_index_free", |b| {
        b.iter(|| {
            let (ptr, idx) = pool
                .alloc(|| CollSlot::new(CollContext::default(), CollPerfMeta::default(), 0))
                .unwrap();
            // SAFETY: ptr came from `alloc` above and is still live.
            black_box(unsafe { pool.index_of(black_box(ptr)) });
            pool.free_idx(idx);
        })
    });
}

#[cfg(target_os = "linux")]
mod linux {
    use criterion::{BenchmarkId, Criterion, Throughput};
    use probing_nccl_profiler::abi::profiler_v4::{
        NcclProfilerCollDescrV4, NcclProfilerEventBodyV4, NcclProfilerKernelChDescrV4,
    };
    use probing_nccl_profiler::abi::{
        NcclProfilerCollDescr, NcclProfilerEventBodyV3, NcclProfilerEventDescrV3,
        NcclProfilerEventDescrV4, NcclProfilerProxyOpDescr, NCCL_PROFILE_COLL,
        NCCL_PROFILE_KERNEL_CH, NCCL_PROFILE_PROXY_OP, NCCL_PROFILE_PROXY_STEP,
        STATE_KERNEL_CH_STOP, STATE_PROXY_STEP_SEND_GPU_WAIT, STATE_PROXY_STEP_SEND_WAIT,
    };
    use probing_nccl_profiler::state::{CommInfo, PluginState, StateArgs};
    use std::ffi::c_void;

    const COMM: CommInfo = CommInfo {
        comm_hash: 0xBEEF,
        n_ranks: 8,
    };

    fn v3_coll_descr() -> NcclProfilerEventDescrV3 {
        NcclProfilerEventDescrV3 {
            type_: NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 0,
            body: NcclProfilerEventBodyV3 {
                coll: {
                    let mut d: NcclProfilerCollDescr = unsafe { std::mem::zeroed() };
                    d.comm_hash = 1;
                    d.seq_number = 1;
                    d.func = c"AllReduce".as_ptr();
                    d.count = 1 << 20;
                    d.datatype = c"ncclFloat16".as_ptr();
                    d.n_max_channels = 4;
                    d
                },
            },
        }
    }

    fn v3_proxy_descr(parent: *mut c_void) -> NcclProfilerEventDescrV3 {
        NcclProfilerEventDescrV3 {
            type_: NCCL_PROFILE_PROXY_OP as u8,
            parent_obj: parent,
            rank: 0,
            body: NcclProfilerEventBodyV3 {
                proxy_op: NcclProfilerProxyOpDescr {
                    pid: 0,
                    channel_id: 0,
                    peer: 1,
                    n_steps: 4,
                    chunk_size: 1024,
                    is_send: 1,
                },
            },
        }
    }

    fn v4_coll_descr() -> NcclProfilerEventDescrV4 {
        NcclProfilerEventDescrV4 {
            type_: NCCL_PROFILE_COLL as u8,
            parent_obj: std::ptr::null_mut(),
            rank: 0,
            body: NcclProfilerEventBodyV4 {
                coll: {
                    let mut d: NcclProfilerCollDescrV4 = unsafe { std::mem::zeroed() };
                    d.seq_number = 1;
                    d.func = c"AllReduce".as_ptr();
                    d.count = 1 << 20;
                    d.datatype = c"ncclFloat16".as_ptr();
                    d.n_channels = 4;
                    d
                },
            },
        }
    }

    fn run_v3_coll_proxy_cycle(state: &PluginState) {
        let coll_descr = v3_coll_descr();
        let mut coll_h: *mut c_void = std::ptr::null_mut();
        state.start_event(&mut coll_h, &coll_descr);

        let proxy_descr = v3_proxy_descr(coll_h);
        let mut proxy_h: *mut c_void = std::ptr::null_mut();
        state.start_event(&mut proxy_h, &proxy_descr);
        state.stop_event(proxy_h);
        state.stop_event(coll_h);
    }

    fn run_v4_full_cycle(state: &PluginState) {
        let coll_descr = v4_coll_descr();
        let mut coll_h: *mut c_void = std::ptr::null_mut();
        state.start_event_v4(&mut coll_h, &coll_descr, COMM);

        let proxy_descr = NcclProfilerEventDescrV4 {
            type_: NCCL_PROFILE_PROXY_OP as u8,
            parent_obj: coll_h,
            rank: 0,
            body: NcclProfilerEventBodyV4 {
                proxy_op: NcclProfilerProxyOpDescr {
                    pid: 0,
                    channel_id: 0,
                    peer: 1,
                    n_steps: 2,
                    chunk_size: 1024,
                    is_send: 1,
                },
            },
        };
        let mut proxy_h: *mut c_void = std::ptr::null_mut();
        state.start_event_v4(&mut proxy_h, &proxy_descr, COMM);

        // One proxy step with two state transitions + trans_size (v4).
        let step_descr = NcclProfilerEventDescrV4 {
            type_: NCCL_PROFILE_PROXY_STEP as u8,
            parent_obj: proxy_h,
            rank: 0,
            body: NcclProfilerEventBodyV4 {
                proxy_step: probing_nccl_profiler::abi::NcclProfilerProxyStepDescr { step: 0 },
            },
        };
        let mut step_h: *mut c_void = std::ptr::null_mut();
        state.start_event_v4(&mut step_h, &step_descr, COMM);
        state.record_state(step_h, STATE_PROXY_STEP_SEND_GPU_WAIT, StateArgs::None);
        state.record_state(
            step_h,
            STATE_PROXY_STEP_SEND_WAIT,
            StateArgs::ProxyStepV4 { trans_size: 65536 },
        );
        state.stop_event(step_h);

        state.stop_event(proxy_h);

        // KernelCh with GPU timer (v4).
        let kch_descr = NcclProfilerEventDescrV4 {
            type_: NCCL_PROFILE_KERNEL_CH as u8,
            parent_obj: coll_h,
            rank: 0,
            body: NcclProfilerEventBodyV4 {
                kernel_ch: NcclProfilerKernelChDescrV4 {
                    channel_id: 0,
                    p_timer: 1_000_000,
                },
            },
        };
        let mut kch_h: *mut c_void = std::ptr::null_mut();
        state.start_event_v4(&mut kch_h, &kch_descr, COMM);
        state.record_state(
            kch_h,
            STATE_KERNEL_CH_STOP,
            StateArgs::KernelChStop { p_timer: 2_500_000 },
        );
        state.stop_event(kch_h);

        state.stop_event(coll_h);
    }

    pub fn bench_callback_cycles(c: &mut Criterion) {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PROBING_DATA_DIR", dir.path());
        std::env::set_var("PROBING_NCCL_INFLIGHT_THRESHOLD_SECS", "0");
        let state = PluginState::new();

        let mut group = c.benchmark_group("callback_cycle");
        group.throughput(Throughput::Elements(1));

        group.bench_function(BenchmarkId::new("v3", "coll_proxy"), |b| {
            b.iter(|| run_v3_coll_proxy_cycle(&state));
        });

        group.bench_function(BenchmarkId::new("v4", "coll_proxy_step_kch"), |b| {
            b.iter(|| run_v4_full_cycle(&state));
        });

        group.finish();
    }

    pub fn bench_record_state(c: &mut Criterion) {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PROBING_DATA_DIR", dir.path());
        let state = PluginState::new();

        let coll_descr = v3_coll_descr();
        let mut coll_h: *mut c_void = std::ptr::null_mut();
        state.start_event(&mut coll_h, &coll_descr);
        let proxy_descr = v3_proxy_descr(coll_h);
        let mut proxy_h: *mut c_void = std::ptr::null_mut();
        state.start_event(&mut proxy_h, &proxy_descr);
        let step_descr = NcclProfilerEventDescrV3 {
            type_: NCCL_PROFILE_PROXY_STEP as u8,
            parent_obj: proxy_h,
            rank: 0,
            body: NcclProfilerEventBodyV3 {
                proxy_step: probing_nccl_profiler::abi::NcclProfilerProxyStepDescr { step: 0 },
            },
        };
        let mut step_h: *mut c_void = std::ptr::null_mut();
        state.start_event(&mut step_h, &step_descr);

        c.bench_function("record_state_proxy_step", |b| {
            b.iter(|| {
                state.record_state(step_h, STATE_PROXY_STEP_SEND_GPU_WAIT, StateArgs::None);
            });
        });

        state.stop_event(step_h);
        state.stop_event(proxy_h);
        state.stop_event(coll_h);
    }
}

#[cfg(target_os = "linux")]
fn bench_callback_cycle(c: &mut Criterion) {
    linux::bench_callback_cycles(c);
    linux::bench_record_state(c);
}

#[cfg(not(target_os = "linux"))]
fn bench_callback_cycle(_c: &mut Criterion) {}

criterion_group!(benches, bench_clock, bench_pool, bench_callback_cycle);
criterion_main!(benches);
