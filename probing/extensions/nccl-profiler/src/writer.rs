//! Mmap writer: batch flush, surfaced errors (no silent drop).

use std::sync::atomic::{AtomicBool, Ordering};

use probing_memtable::discover::ExposedTable;
use probing_memtable::Value;

use crate::events::{CompletedProxyOp, EventCounters};
use crate::tables::{net_qp_schema, proxy_ops_schema, NET_QP_FILE, PROXY_OPS_FILE};

const CHUNK_SIZE: u32 = 16 * 1024;
const NUM_CHUNKS: u32 = 32;

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

pub struct NcclWriter {
    proxy_table: Option<ExposedTable>,
    net_table: Option<ExposedTable>,
    proxy_init_failed: AtomicBool,
    net_init_failed: AtomicBool,
    logged_proxy_init: AtomicBool,
    logged_net_init: AtomicBool,
}

impl NcclWriter {
    pub fn new() -> Self {
        Self {
            proxy_table: None,
            net_table: None,
            proxy_init_failed: AtomicBool::new(false),
            net_init_failed: AtomicBool::new(false),
            logged_proxy_init: AtomicBool::new(false),
            logged_net_init: AtomicBool::new(false),
        }
    }

    fn open_proxy(&mut self) -> Result<&mut ExposedTable, ()> {
        if self.proxy_table.is_none() && !self.proxy_init_failed.load(Ordering::Relaxed) {
            match ExposedTable::create(PROXY_OPS_FILE, &proxy_ops_schema(), CHUNK_SIZE, NUM_CHUNKS)
            {
                Ok(t) => self.proxy_table = Some(t),
                Err(e) => {
                    self.proxy_init_failed.store(true, Ordering::Relaxed);
                    if self
                        .logged_proxy_init
                        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                    {
                        eprintln!("[probing-nccl-profiler] failed to open {PROXY_OPS_FILE}: {e}");
                    }
                }
            }
        }
        self.proxy_table.as_mut().ok_or(())
    }

    fn open_net(&mut self) -> Result<&mut ExposedTable, ()> {
        if self.net_table.is_none() && !self.net_init_failed.load(Ordering::Relaxed) {
            match ExposedTable::create(NET_QP_FILE, &net_qp_schema(), CHUNK_SIZE, NUM_CHUNKS) {
                Ok(t) => self.net_table = Some(t),
                Err(e) => {
                    self.net_init_failed.store(true, Ordering::Relaxed);
                    if self
                        .logged_net_init
                        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                    {
                        eprintln!("[probing-nccl-profiler] failed to open {NET_QP_FILE}: {e}");
                    }
                }
            }
        }
        self.net_table.as_mut().ok_or(())
    }

    pub fn append_proxy_op(&mut self, row: &CompletedProxyOp, counters: &EventCounters) -> bool {
        let Ok(table) = self.open_proxy() else {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        table.push_row(&[
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
            Value::I64(row.send_wait_ns),
            Value::I64(row.recv_wait_ns),
            Value::I64(row.recv_flush_wait_ns),
        ]);
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

    pub fn append_net_qp(&mut self, row: &CompletedNetQp, counters: &EventCounters) -> bool {
        let Ok(table) = self.open_net() else {
            counters.write_errors.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        table.push_row(&[
            Value::I64(row.ts_ns),
            Value::I32(row.rank),
            Value::I32(row.device),
            Value::I32(row.qp_num),
            Value::U64(row.wr_id),
            Value::I32(row.opcode),
            Value::U64(row.length),
            Value::I64(row.duration_ns),
        ]);
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
