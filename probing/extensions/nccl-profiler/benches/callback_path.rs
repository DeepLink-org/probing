//! Criterion benchmarks for the NCCL callback hot path.
//!
//! Run with `cargo bench -p probing-nccl-profiler`. The full callback-cycle
//! bench only runs on Linux (the plugin state machine is Linux-gated); the
//! clock and pool benches run everywhere.

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
fn bench_callback_cycle(c: &mut Criterion) {
    use probing_nccl_profiler::abi::{
        NcclProfilerCollDescr, NcclProfilerEventBodyV3, NcclProfilerEventDescrV3,
        NcclProfilerProxyOpDescr, NCCL_PROFILE_COLL, NCCL_PROFILE_PROXY_OP,
    };
    use probing_nccl_profiler::state::PluginState;
    use std::ffi::c_void;

    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("PROBING_DATA_DIR", dir.path());
    let state = PluginState::new();

    let coll_descr = NcclProfilerEventDescrV3 {
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
    };

    // coll start → proxy start → proxy stop → coll stop: one full op cycle,
    // including the coll_perf row emit and the mmap append.
    c.bench_function("coll_proxy_full_cycle", |b| {
        b.iter(|| {
            let mut coll_h: *mut c_void = std::ptr::null_mut();
            state.start_event(&mut coll_h, &coll_descr);

            let proxy_descr = NcclProfilerEventDescrV3 {
                type_: NCCL_PROFILE_PROXY_OP as u8,
                parent_obj: coll_h,
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
            };
            let mut proxy_h: *mut c_void = std::ptr::null_mut();
            state.start_event(&mut proxy_h, &proxy_descr);
            state.stop_event(proxy_h);
            state.stop_event(coll_h);
        })
    });
}

#[cfg(not(target_os = "linux"))]
fn bench_callback_cycle(_c: &mut Criterion) {}

criterion_group!(benches, bench_clock, bench_pool, bench_callback_cycle);
criterion_main!(benches);
