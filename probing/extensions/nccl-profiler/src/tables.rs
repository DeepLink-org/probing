//! Memtable schemas for `nccl.proxy_ops` and `nccl.net_qp`.

use probing_memtable::{DType, Schema};

pub const PROXY_OPS_FILE: &str = "nccl.proxy_ops";
pub const NET_QP_FILE: &str = "nccl.net_qp";

pub fn proxy_ops_schema() -> Schema {
    Schema::new()
        .col("ts", DType::I64)
        .col("rank", DType::I32)
        .col("tp_rank", DType::I32)
        .col("pp_rank", DType::I32)
        .col("dp_rank", DType::I32)
        .col("comm_hash", DType::U64)
        .col("coll_func", DType::Str)
        .col("seq", DType::U64)
        .col("channel_id", DType::I32)
        .col("peer", DType::I32)
        .col("is_send", DType::I32)
        .col("n_steps", DType::I32)
        .col("trans_bytes", DType::U64)
        .col("send_gpu_wait_ns", DType::I64)
        .col("send_wait_ns", DType::I64)
        .col("recv_wait_ns", DType::I64)
        .col("recv_flush_wait_ns", DType::I64)
}

pub fn net_qp_schema() -> Schema {
    Schema::new()
        .col("ts", DType::I64)
        .col("rank", DType::I32)
        .col("device", DType::I32)
        .col("qp_num", DType::I32)
        .col("wr_id", DType::U64)
        .col("opcode", DType::I32)
        .col("length", DType::U64)
        .col("duration_ns", DType::I64)
}
