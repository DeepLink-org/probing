//! Memtable schemas for `nccl.proxy_ops`, `nccl.coll_perf`, `nccl.inflight_ops` and `nccl.net_qp`.

use probing_memtable::docs;
use probing_memtable::{DType, Schema};

pub const PROXY_OPS_FILE: &str = "nccl.proxy_ops";
pub const COLL_PERF_FILE: &str = "nccl.coll_perf";
pub const INFLIGHT_OPS_FILE: &str = "nccl.inflight_ops";
pub const NET_QP_FILE: &str = "nccl.net_qp";
pub const PROFILER_COUNTERS_FILE: &str = "nccl.profiler_counters";

/// Register all NCCL table docs (safe to call from writer or Engine startup).
pub fn register_docs() {
    docs::register_from_name(PROXY_OPS_FILE, &proxy_ops_schema());
    docs::register_from_name(COLL_PERF_FILE, &coll_perf_schema());
    docs::register_from_name(INFLIGHT_OPS_FILE, &inflight_ops_schema());
    docs::register_from_name(NET_QP_FILE, &net_qp_schema());
    docs::register_from_name(PROFILER_COUNTERS_FILE, &profiler_counters_schema());
}

pub fn proxy_ops_schema() -> Schema {
    Schema::new()
        .table_doc("NCCL profiler plugin proxy-op wait 分解（culprit / victim 归因）")
        .col_doc("ts", DType::I64, "事件时间戳（UNIX epoch 纳秒）")
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
        .col_doc("trans_bytes", DType::U64, "传输字节数（v4 按 step 累计）")
        .col_doc(
            "send_gpu_wait_ns",
            DType::I64,
            "Culprit 信号 — 本地 GPU 未就绪发送",
        )
        .col_doc(
            "send_peer_wait_ns",
            DType::I64,
            "等待接收端 clear-to-send credits（仅 v4 ABI；v3 为 0）",
        )
        .col_doc("send_wait_ns", DType::I64, "发送侧网络等待")
        .col_doc("recv_wait_ns", DType::I64, "Victim 信号 — 等待对端数据")
        .col_doc("recv_flush_wait_ns", DType::I64, "接收 flush 等待")
}

pub fn coll_perf_schema() -> Schema {
    Schema::new()
        .table_doc(
            "NCCL collective / P2P 级性能（timing_source 标注计时来源；busbw 需按通信组大小在 SQL 中换算）",
        )
        .col_doc("ts", DType::I64, "op 完成时间戳（UNIX epoch 纳秒）")
        .col_doc("rank", DType::I32, "torch.distributed rank")
        .col_doc("tp_rank", DType::I32, "张量并行 rank（未知 -1）")
        .col_doc("pp_rank", DType::I32, "流水线并行 rank（未知 -1）")
        .col_doc("dp_rank", DType::I32, "数据并行 rank（未知 -1）")
        .col_doc("comm_hash", DType::U64, "NCCL communicator hash")
        .col_doc(
            "n_ranks",
            DType::I32,
            "通信组大小（v4 init 提供；v3 为 -1）— busbw 换算用",
        )
        .col_doc(
            "coll_func",
            DType::Str,
            "操作名（AllReduce、AllGather、Send、Recv…）",
        )
        .col_doc("seq", DType::U64, "collective 序号（P2P 为 0）")
        .col_doc("is_p2p", DType::I32, "1=P2P（Send/Recv），0=collective")
        .col_doc("peer", DType::I32, "P2P 对端 rank（collective 为 -1）")
        .col_doc("count", DType::U64, "元素个数")
        .col_doc("msg_size_bytes", DType::U64, "消息大小 = count × dtype 字节数")
        .col_doc("dtype", DType::Str, "NCCL 数据类型名")
        .col_doc("algo", DType::Str, "NCCL 算法（Ring、Tree…；P2P 为空）")
        .col_doc("proto", DType::Str, "NCCL 协议（LL、LL128、Simple…）")
        .col_doc("n_channels", DType::I32, "最大 channel 数")
        .col_doc(
            "exec_time_ns",
            DType::I64,
            "实际执行耗时（纳秒）；由 timing_source 对应的事件窗口重建",
        )
        .col_doc(
            "enqueue_time_ns",
            DType::I64,
            "host 侧 enqueue 耗时（纳秒；NCCL coll stopEvent 语义）",
        )
        .col_doc(
            "timing_source",
            DType::Str,
            "exec_time 来源：kernel_gpu（GPU globaltimer，v4）> kernel_ch（host 观测内核窗口）> proxy > enqueue（回退）",
        )
        .col_doc(
            "algobw_gbps",
            DType::F64,
            "算法带宽 msg_size/exec_time（GB/s）；busbw 需乘集合通信系数",
        )
        .col_doc(
            "pool_events_dropped",
            DType::I32,
            "因子事件 slot pool 耗尽而丢失的子事件数（>0 表示 exec_time 可能不可信）",
        )
}

pub fn inflight_ops_schema() -> Schema {
    Schema::new()
        .table_doc("在途（已 start 未 stop）NCCL 操作周期快照 — hang 诊断信号")
        .col_doc("ts", DType::I64, "快照时间戳（UNIX epoch 纳秒）")
        .col_doc("rank", DType::I32, "torch.distributed rank")
        .col_doc("comm_hash", DType::U64, "NCCL communicator hash")
        .col_doc("coll_func", DType::Str, "操作名（未知为 unknown）")
        .col_doc("seq", DType::U64, "collective 序号")
        .col_doc("kind", DType::Str, "coll / p2p / proxy_op")
        .col_doc("channel_id", DType::I32, "NCCL channel（proxy_op 有效）")
        .col_doc("peer", DType::I32, "对端 rank（未知 -1）")
        .col_doc("is_send", DType::I32, "1=send，0=recv（proxy_op 有效）")
        .col_doc("start_ns", DType::I64, "op 开始时间戳（epoch 纳秒）")
        .col_doc("age_ns", DType::I64, "快照时刻已持续时长（纳秒）")
}

pub fn net_qp_schema() -> Schema {
    Schema::new()
        .table_doc("NCCL NetPlugin IB QP 完成耗时（可选 mask bit 128）")
        .col_doc("ts", DType::I64, "事件时间戳（UNIX epoch 纳秒）")
        .col_doc("rank", DType::I32, "torch.distributed rank")
        .col_doc("device", DType::I32, "IB 设备索引")
        .col_doc("qp_num", DType::I32, "Queue Pair 号")
        .col_doc("wr_id", DType::U64, "Work Request id")
        .col_doc("opcode", DType::I32, "IB opcode")
        .col_doc("length", DType::U64, "传输长度（字节）")
        .col_doc("duration_ns", DType::I64, "QP 完成耗时（纳秒）")
}

pub fn profiler_counters_schema() -> Schema {
    Schema::new()
        .table_doc("NCCL profiler 插件运行时计数与 pool 使用率（诊断数据完整性）")
        .col_doc("ts", DType::I64, "快照时间戳（UNIX epoch 纳秒）")
        .col_doc("rank", DType::I32, "torch.distributed rank（未知 -1）")
        .col_doc("coll_events", DType::U64, "累计 coll start 事件数")
        .col_doc("p2p_events", DType::U64, "累计 P2P start 事件数")
        .col_doc("proxy_op_events", DType::U64, "累计 proxy-op start 事件数")
        .col_doc(
            "proxy_step_events",
            DType::U64,
            "累计 proxy-step start 事件数",
        )
        .col_doc(
            "kernel_ch_events",
            DType::U64,
            "累计 kernel-ch start 事件数",
        )
        .col_doc("net_events", DType::U64, "累计 net-plugin start 事件数")
        .col_doc("rows_written", DType::U64, "累计写入 memtable 行数")
        .col_doc("pool_exhausted", DType::U64, "累计 slot pool 分配失败次数")
        .col_doc("write_errors", DType::U64, "累计 memtable 写入失败次数")
        .col_doc(
            "filtered",
            DType::U64,
            "累计被 PROBING_NCCL_MIN_MSG_BYTES 过滤的事件数",
        )
        .col_doc("coll_live", DType::I32, "当前 live coll slot 数")
        .col_doc("proxy_live", DType::I32, "当前 live proxy-op slot 数")
        .col_doc("step_live", DType::I32, "当前 live proxy-step slot 数")
        .col_doc("kch_live", DType::I32, "当前 live kernel-ch slot 数")
        .col_doc("net_live", DType::I32, "当前 live net slot 数")
        .col_doc("coll_cap", DType::I32, "coll slot pool 容量")
        .col_doc("proxy_cap", DType::I32, "proxy-op slot pool 容量")
        .col_doc("step_cap", DType::I32, "proxy-step slot pool 容量")
        .col_doc("kch_cap", DType::I32, "kernel-ch slot pool 容量")
        .col_doc("net_cap", DType::I32, "net slot pool 容量")
        .col_doc(
            "ring_chunks_recycled",
            DType::U64,
            "memtable 环缓冲累计回收 chunk 数（所有 nccl.* 表之和）",
        )
        .col_doc(
            "ring_rows_overwritten",
            DType::U64,
            "memtable 环缓冲累计覆写行数（所有 nccl.* 表之和）",
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_docs_populates_registry() {
        register_docs();
        let rows = docs::snapshot();
        for table in [
            "proxy_ops",
            "coll_perf",
            "inflight_ops",
            "net_qp",
            "profiler_counters",
        ] {
            assert!(
                rows.iter()
                    .any(|r| r.table_schema == "nccl" && r.table_name == table),
                "missing docs for nccl.{table}"
            );
        }
    }

    #[test]
    fn coll_perf_columns_documented() {
        let schema = coll_perf_schema();
        for name in [
            "msg_size_bytes",
            "exec_time_ns",
            "enqueue_time_ns",
            "timing_source",
            "algobw_gbps",
            "is_p2p",
            "n_ranks",
            "pool_events_dropped",
        ] {
            assert!(
                schema.cols.iter().any(|c| c.name == name),
                "missing column {name}"
            );
        }
    }

    #[test]
    fn inflight_ops_columns_documented() {
        let schema = inflight_ops_schema();
        for name in ["kind", "age_ns", "start_ns"] {
            assert!(
                schema.cols.iter().any(|c| c.name == name),
                "missing column {name}"
            );
        }
    }

    #[test]
    fn proxy_ops_culprit_columns_documented() {
        let schema = proxy_ops_schema();
        assert!(schema.table_doc.is_some());
        for name in ["send_gpu_wait_ns", "send_peer_wait_ns", "recv_wait_ns"] {
            let col = schema
                .cols
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("missing column {name}"));
            assert!(col.doc.is_some(), "{name} should have doc");
        }
    }
}
