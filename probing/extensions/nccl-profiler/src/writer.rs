//! Mmap writer: batch flush, surfaced errors (no silent drop).

use std::sync::atomic::{AtomicBool, Ordering};

use probing_memtable::discover::ExposedTable;
use probing_memtable::Value;

use crate::events::{
    CompletedCollPerf, CompletedProxyOp, EventCounters, InflightOp, ProfilerCounterSnapshot,
};
use crate::ring_config::nccl_mmap_ring_config;
use crate::tables::{
    coll_perf_schema, inflight_ops_schema, net_qp_schema, profiler_counters_schema,
    proxy_ops_schema, COLL_PERF_FILE, INFLIGHT_OPS_FILE, NET_QP_FILE, PROFILER_COUNTERS_FILE,
    PROXY_OPS_FILE,
};

pub struct CompletedNetQp {
    pub ts_ns: i64,
    pub rank: i32,
    pub device: i32,
    pub qp_num: i32,
    pub wr_id: u64,
    pub opcode: i32,
    pub length: u64,
    pub duration_ns: i64,
}

/// Lazily-created mmap table; failure is remembered and logged once.
struct LazyTable {
    name: &'static str,
    schema: fn() -> probing_memtable::Schema,
    table: Option<ExposedTable>,
    init_failed: AtomicBool,
    logged_init: AtomicBool,
}

impl LazyTable {
    fn new(name: &'static str, schema: fn() -> probing_memtable::Schema) -> Self {
        Self {
            name,
            schema,
            table: None,
            init_failed: AtomicBool::new(false),
            logged_init: AtomicBool::new(false),
        }
    }

    fn open(&mut self) -> Result<&mut ExposedTable, ()> {
        if self.table.is_none() && !self.init_failed.load(Ordering::Relaxed) {
            let (chunk_size, num_chunks) = nccl_mmap_ring_config();
            match ExposedTable::create(self.name, &(self.schema)(), chunk_size, num_chunks) {
                Ok(t) => self.table = Some(t),
                Err(e) => {
                    self.init_failed.store(true, Ordering::Relaxed);
                    if self
                        .logged_init
                        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                    {
                        crate::log::warn(format!("failed to open {}: {e}", self.name));
                    }
                }
            }
        }
        self.table.as_mut().ok_or(())
    }

    fn ring_overwrite_stats(&self) -> (u32, u32) {
        self.table
            .as_ref()
            .map(ExposedTable::ring_overwrite_stats)
            .unwrap_or((0, 0))
    }
}

pub struct NcclWriter {
    proxy: LazyTable,
    coll_perf: LazyTable,
    inflight: LazyTable,
    net: LazyTable,
    counters: LazyTable,
}

impl NcclWriter {
    pub fn new() -> Self {
        Self {
            proxy: LazyTable::new(PROXY_OPS_FILE, proxy_ops_schema),
            coll_perf: LazyTable::new(COLL_PERF_FILE, coll_perf_schema),
            inflight: LazyTable::new(INFLIGHT_OPS_FILE, inflight_ops_schema),
            net: LazyTable::new(NET_QP_FILE, net_qp_schema),
            counters: LazyTable::new(PROFILER_COUNTERS_FILE, profiler_counters_schema),
        }
    }

    fn ring_overwrite_totals(&self) -> (u32, u32) {
        let mut chunks = 0u32;
        let mut rows = 0u32;
        for stats in [
            self.proxy.ring_overwrite_stats(),
            self.coll_perf.ring_overwrite_stats(),
            self.inflight.ring_overwrite_stats(),
            self.net.ring_overwrite_stats(),
            self.counters.ring_overwrite_stats(),
        ] {
            chunks = chunks.saturating_add(stats.0);
            rows = rows.saturating_add(stats.1);
        }
        (chunks, rows)
    }

    pub fn append_proxy_op(&mut self, row: &CompletedProxyOp, counters: &EventCounters) -> bool {
        let Ok(table) = self.proxy.open() else {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if !table.push_row(&[
            Value::I64(row.ts_ns),
            Value::I32(row.rank),
            Value::I32(row.roles.tp_rank),
            Value::I32(row.roles.pp_rank),
            Value::I32(row.roles.dp_rank),
            Value::U64(row.comm_hash),
            Value::Str(row.func_str()),
            Value::U64(row.seq),
            Value::I32(row.channel_id),
            Value::I32(row.peer),
            Value::I32(row.is_send),
            Value::I32(row.n_steps),
            Value::U64(row.trans_bytes),
            Value::I64(row.send_gpu_wait_ns),
            Value::I64(row.send_peer_wait_ns),
            Value::I64(row.send_wait_ns),
            Value::I64(row.recv_wait_ns),
            Value::I64(row.recv_flush_wait_ns),
        ]) {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        counters.rows_written.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub fn flush_proxy_ops(
        &mut self,
        rows: &[CompletedProxyOp],
        counters: &EventCounters,
    ) -> usize {
        let mut ok = 0usize;
        for row in rows {
            if self.append_proxy_op(row, counters) {
                ok += 1;
            }
        }
        ok
    }

    pub fn append_coll_perf(&mut self, row: &CompletedCollPerf, counters: &EventCounters) -> bool {
        let Ok(table) = self.coll_perf.open() else {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if !table.push_row(&[
            Value::I64(row.ts_ns),
            Value::I32(row.rank),
            Value::I32(row.roles.tp_rank),
            Value::I32(row.roles.pp_rank),
            Value::I32(row.roles.dp_rank),
            Value::U64(row.comm_hash),
            Value::I32(row.n_ranks),
            Value::Str(row.func_str()),
            Value::U64(row.seq),
            Value::I32(row.is_p2p as i32),
            Value::I32(row.peer),
            Value::U64(row.count),
            Value::U64(row.msg_size_bytes),
            Value::Str(row.dtype.as_str()),
            Value::Str(row.algo.as_str()),
            Value::Str(row.proto.as_str()),
            Value::I32(row.n_channels),
            Value::I64(row.exec_time_ns),
            Value::I64(row.enqueue_time_ns),
            Value::Str(row.timing_source),
            Value::F64(row.algobw_gbps),
            Value::I32(row.pool_events_dropped),
        ]) {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        counters.rows_written.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub fn append_inflight(&mut self, row: &InflightOp, counters: &EventCounters) -> bool {
        let Ok(table) = self.inflight.open() else {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if !table.push_row(&[
            Value::I64(row.ts_ns),
            Value::I32(row.rank),
            Value::U64(row.comm_hash),
            Value::Str(row.func_str()),
            Value::U64(row.seq),
            Value::Str(row.kind),
            Value::I32(row.channel_id),
            Value::I32(row.peer),
            Value::I32(row.is_send),
            Value::I64(row.start_ns),
            Value::I64(row.age_ns),
        ]) {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        counters.rows_written.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub fn append_net_qp(&mut self, row: &CompletedNetQp, counters: &EventCounters) -> bool {
        let Ok(table) = self.net.open() else {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if !table.push_row(&[
            Value::I64(row.ts_ns),
            Value::I32(row.rank),
            Value::I32(row.device),
            Value::I32(row.qp_num),
            Value::U64(row.wr_id),
            Value::I32(row.opcode),
            Value::U64(row.length),
            Value::I64(row.duration_ns),
        ]) {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        counters.rows_written.fetch_add(1, Ordering::Relaxed);
        true
    }

    pub fn append_profiler_counters(
        &mut self,
        row: &ProfilerCounterSnapshot,
        counters: &EventCounters,
    ) -> bool {
        let (ring_chunks, ring_rows) = self.ring_overwrite_totals();
        let row = ProfilerCounterSnapshot {
            ring_chunks_recycled: ring_chunks,
            ring_rows_overwritten: ring_rows,
            ..*row
        };
        let Ok(table) = self.counters.open() else {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if !table.push_row(&[
            Value::I64(row.ts_ns),
            Value::I32(row.rank),
            Value::U64(row.coll_events),
            Value::U64(row.p2p_events),
            Value::U64(row.proxy_op_events),
            Value::U64(row.proxy_step_events),
            Value::U64(row.kernel_ch_events),
            Value::U64(row.net_events),
            Value::U64(row.rows_written),
            Value::U64(row.pool_exhausted),
            Value::U64(row.write_errors),
            Value::U64(row.filtered),
            Value::I32(row.coll_live),
            Value::I32(row.proxy_live),
            Value::I32(row.step_live),
            Value::I32(row.kch_live),
            Value::I32(row.net_live),
            Value::I32(row.coll_cap),
            Value::I32(row.proxy_cap),
            Value::I32(row.step_cap),
            Value::I32(row.kch_cap),
            Value::I32(row.net_cap),
            Value::U64(row.ring_chunks_recycled as u64),
            Value::U64(row.ring_rows_overwritten as u64),
        ]) {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        counters.rows_written.fetch_add(1, Ordering::Relaxed);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_memtable::discover::discover_in;
    use std::fs;

    #[test]
    fn proxy_ops_mmap_roundtrip() {
        let base =
            std::env::temp_dir().join(format!("probing_nccl_profiler_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let counters = EventCounters::new();
        let mut w = NcclWriter::new();
        let mut func = [0u8; 32];
        func[..9].copy_from_slice(b"AllReduce");
        assert!(w.append_proxy_op(
            &CompletedProxyOp {
                ts_ns: 1,
                rank: 0,
                roles: crate::role::RoleRanks::default(),
                comm_hash: 42,
                coll_func: func,
                coll_func_len: 9,
                seq: 7,
                channel_id: 1,
                peer: 2,
                is_send: 1,
                n_steps: 4,
                trans_bytes: 1024,
                send_gpu_wait_ns: 10,
                send_peer_wait_ns: 2,
                send_wait_ns: 20,
                recv_wait_ns: 30,
                recv_flush_wait_ns: 5,
            },
            &counters
        ));

        let found = discover_in(&base).unwrap();
        assert!(found.iter().any(|t| t.name() == PROXY_OPS_FILE));

        let _ = fs::remove_dir_all(&base);
    }
}
