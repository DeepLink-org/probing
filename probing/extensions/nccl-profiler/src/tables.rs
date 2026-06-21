//! Memtable schemas for `nccl.proxy_ops` and `nccl.net_qp`.

use probing_memtable::docs;
use probing_memtable::{DType, Schema};

pub const PROXY_OPS_FILE: &str = "nccl.proxy_ops";
pub const NET_QP_FILE: &str = "nccl.net_qp";

/// Register all NCCL table docs (safe to call from writer or Engine startup).
pub fn register_docs() {
    docs::register_from_name(PROXY_OPS_FILE, &proxy_ops_schema());
    docs::register_from_name(NET_QP_FILE, &net_qp_schema());
}

pub fn proxy_ops_schema() -> Schema {
    Schema::new()
        .table_doc("NCCL profiler plugin proxy-op wait 分解（culprit / victim 归因）")
        .col_doc("ts", DType::I64, "事件时间戳（纳秒）")
        .col_doc("rank", DType::I32, "torch.distributed rank")
        .col_doc("tp_rank", DType::I32, "张量并行 rank（未知 -1）")
        .col_doc("pp_rank", DType::I32, "流水线并行 rank（未知 -1）")
        .col_doc("dp_rank", DType::I32, "数据并行 rank（未知 -1）")
        .col_doc("comm_hash", DType::U64, "NCCL communicator hash")
        .col_doc(
            "coll_func",
            DType::Str,
            "集合通信名（AllReduce、AllGather…）",
        )
        .col_doc("seq", DType::U64, "collective 序号")
        .col_doc("channel_id", DType::I32, "NCCL channel id")
        .col_doc("peer", DType::I32, "对端 rank")
        .col_doc("is_send", DType::I32, "1=send proxy，0=recv proxy")
        .col_doc("n_steps", DType::I32, "聚合的 ProxyStep 数")
        .col_doc("trans_bytes", DType::U64, "传输字节数")
        .col_doc(
            "send_gpu_wait_ns",
            DType::I64,
            "Culprit 信号 — 本地 GPU 未就绪发送",
        )
        .col_doc("send_wait_ns", DType::I64, "发送侧网络等待")
        .col_doc("recv_wait_ns", DType::I64, "Victim 信号 — 等待对端数据")
        .col_doc("recv_flush_wait_ns", DType::I64, "接收 flush 等待")
}

pub fn net_qp_schema() -> Schema {
    Schema::new()
        .table_doc("NCCL NetPlugin IB QP 完成耗时（可选 mask bit 128）")
        .col_doc("ts", DType::I64, "事件时间戳（纳秒）")
        .col_doc("rank", DType::I32, "torch.distributed rank")
        .col_doc("device", DType::I32, "IB 设备索引")
        .col_doc("qp_num", DType::I32, "Queue Pair 号")
        .col_doc("wr_id", DType::U64, "Work Request id")
        .col_doc("opcode", DType::I32, "IB opcode")
        .col_doc("length", DType::U64, "传输长度（字节）")
        .col_doc("duration_ns", DType::I64, "QP 完成耗时（纳秒）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_docs_populates_registry() {
        register_docs();
        let rows = docs::snapshot();
        assert!(rows
            .iter()
            .any(|r| r.table_schema == "nccl" && r.table_name == "proxy_ops"));
        assert!(rows
            .iter()
            .any(|r| r.table_schema == "nccl" && r.table_name == "net_qp"));
    }

    #[test]
    fn proxy_ops_culprit_columns_documented() {
        let schema = proxy_ops_schema();
        assert!(schema.table_doc.is_some());
        for name in ["send_gpu_wait_ns", "recv_wait_ns"] {
            let col = schema
                .cols
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("missing column {name}"));
            assert!(col.doc.is_some(), "{name} should have doc");
        }
    }
}
