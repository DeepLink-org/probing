//! Memtable schemas for HCCL MSProf intercept.

use probing_memtable::docs;
use probing_memtable::{DType, Schema};

pub const HOST_OPS_FILE: &str = "hccl.host_ops";
pub const TASKS_FILE: &str = "hccl.tasks";
pub const COLLECTIVES_FILE: &str = "hccl.collectives";
pub const MC2_STREAMS_FILE: &str = "hccl.mc2_streams";
pub const CONTEXT_IDS_FILE: &str = "hccl.context_ids";

/// Register all HCCL table docs (safe to call from writer or Engine startup).
pub fn register_docs() {
    docs::register_from_name(HOST_OPS_FILE, &host_ops_schema());
    docs::register_from_name(TASKS_FILE, &tasks_schema());
    docs::register_from_name(COLLECTIVES_FILE, &collectives_schema());
    docs::register_from_name(MC2_STREAMS_FILE, &mc2_streams_schema());
    docs::register_from_name(CONTEXT_IDS_FILE, &context_ids_schema());
}

pub fn host_ops_schema() -> Schema {
    Schema::new()
        .table_doc("HCCL MSProf Host API 时间线（集合通信 op、ACL、task master/slave）")
        .col_doc("ts", DType::I64, "结束时间（CANN sys cycle）")
        .col_doc("begin_ns", DType::I64, "开始时间")
        .col_doc("end_ns", DType::I64, "结束时间")
        .col_doc("duration_ns", DType::I64, "耗时 end - begin")
        .col_doc("thread_id", DType::I32, "上报线程 id")
        .col_doc("level", DType::I32, "MSProf level")
        .col_doc("type_id", DType::I32, "MSProf type id")
        .col_doc("item_id", DType::U64, "名称 hash")
        .col_doc(
            "item_name",
            DType::Str,
            "解码名称（hcom_allReduce_、Memcpy 等）",
        )
        .col_doc(
            "event_class",
            DType::Str,
            "host_hccl_op | task_master | task_slave | host_acl | node_launch",
        )
        .col_doc("aging", DType::I32, "MSProf aging flag")
}

pub fn tasks_schema() -> Schema {
    Schema::new()
        .table_doc("HCCL 设备侧 task 明细（MsprofHcclInfo L1）")
        .col_doc("ts", DType::I64, "上报时间戳")
        .col_doc("thread_id", DType::I32, "上报线程")
        .col_doc("info_type", DType::I32, "AdditionalInfo type id")
        .col_doc("info_level", DType::I32, "AdditionalInfo level")
        .col_doc(
            "info_type_name",
            DType::Str,
            "RegTypeInfo 注册名（如 hccl_info）",
        )
        .col_doc("item_id", DType::U64, "task 类型 hash")
        .col_doc("task_name", DType::Str, "task 类型名（Memcpy、RDMASend…）")
        .col_doc("ccl_tag", DType::U64, "CCL tag hash")
        .col_doc("group_name", DType::U64, "comm group hash")
        .col_doc("local_rank", DType::I32, "本端 rank")
        .col_doc("remote_rank", DType::I32, "对端 rank（-1 表示 N/A）")
        .col_doc("rank_size", DType::I32, "通信组大小")
        .col_doc("workflow_mode", DType::I32, "HCCL workflow mode enum")
        .col_doc("plane_id", DType::I32, "原始 plane 编码")
        .col_doc(
            "plane_index",
            DType::I32,
            "plane 索引（plane_id bits 28-31）",
        )
        .col_doc("rank_in_plane", DType::I32, "plane 内 rank（bits 0-15）")
        .col_doc("rank_size_plane", DType::I32, "plane 宽度（bits 16-27）")
        .col_doc("ctx_id", DType::I32, "FFTS context id")
        .col_doc("notify_id", DType::U64, "notify 对象 id")
        .col_doc("stage", DType::I32, "流水线 stage")
        .col_doc("role", DType::I32, "task 角色 enum")
        .col_doc("data_size", DType::U64, "传输字节数")
        .col_doc("op_type", DType::I32, "操作类型 enum")
        .col_doc("data_type", DType::I32, "数据类型 enum")
        .col_doc("link_type", DType::I32, "链路类型")
        .col_doc("transport_type", DType::I32, "传输类型")
        .col_doc("rdma_type", DType::I32, "RDMA 类型")
        .col_doc("duration_est_us", DType::F64, "估算耗时（微秒）")
        .col_doc("payload_len", DType::I32, "AdditionalInfo payload 长度")
}

pub fn collectives_schema() -> Schema {
    Schema::new()
        .table_doc("HCCL 集合通信元数据与 Host 耗时（row_source 区分 api/compact 行）")
        .col_doc("ts", DType::I64, "事件时间")
        .col_doc("thread_id", DType::I32, "上报线程")
        .col_doc(
            "row_source",
            DType::Str,
            "api=耗时行；compact=count/group/alg 参数行",
        )
        .col_doc("begin_ns", DType::I64, "开始时间（api 行）")
        .col_doc("end_ns", DType::I64, "结束时间（api 行）")
        .col_doc("duration_ns", DType::I64, "耗时（api 行）")
        .col_doc("op_hash", DType::U64, "算子名 hash（api 行）")
        .col_doc("op_name", DType::Str, "算子名（api 行）")
        .col_doc("group_hash", DType::U64, "comm group 名 hash（compact 行）")
        .col_doc("alg_hash", DType::U64, "算法名 hash（compact 行）")
        .col_doc("count", DType::U64, "元素个数（compact 行）")
        .col_doc("data_type", DType::I32, "HcclDataType enum")
        .col_doc("relay", DType::I32, "HCCL relay 标志")
        .col_doc("retry", DType::I32, "HCCL retry 计数")
        .col_doc(
            "compact_type",
            DType::I32,
            "MsprofReportCompactInfo type id",
        )
}

pub fn mc2_streams_schema() -> Schema {
    Schema::new()
        .table_doc("HCCL MC2 communicator stream 拓扑快照")
        .col_doc("ts", DType::I64, "上报时间")
        .col_doc("thread_id", DType::I32, "上报线程")
        .col_doc("info_type", DType::I32, "MSProf AdditionalInfo type")
        .col_doc("group_hash", DType::U64, "comm group 名 hash")
        .col_doc("rank_size", DType::I32, "组内 rank 数")
        .col_doc("rank_id", DType::I32, "rank id")
        .col_doc("usr_rank_id", DType::I32, "用户可见 rank id")
        .col_doc("aicpu_kfc_stream_id", DType::I32, "KFC stream id")
        .col_doc("comm_stream_size", DType::I32, "comm stream 数量")
        .col_doc("comm_stream_ids", DType::Str, "逗号分隔的 stream id 列表")
}

pub fn context_ids_schema() -> Schema {
    Schema::new()
        .table_doc("HCCL FFTS context id 范围（dispatch 时上报）")
        .col_doc("ts", DType::I64, "上报时间")
        .col_doc("thread_id", DType::I32, "上报线程")
        .col_doc("info_type", DType::I32, "MSProf AdditionalInfo type")
        .col_doc("ctx_id_num", DType::I32, "context 数量（HCCL 固定报 2）")
        .col_doc("ctx_id_min", DType::I32, "范围起点（通常 0）")
        .col_doc("ctx_id_max", DType::I32, "范围终点（ctxIdMax）")
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
            .any(|r| r.table_schema == "hccl" && r.table_name == "tasks"));
        assert!(rows
            .iter()
            .any(|r| r.table_schema == "hccl" && r.table_name == "host_ops"));
    }

    #[test]
    fn tasks_schema_every_column_documented() {
        let schema = tasks_schema();
        assert!(schema.table_doc.is_some());
        assert_eq!(schema.cols.len(), 29);
        assert!(
            schema.cols.iter().all(|c| c.doc.is_some()),
            "missing docs: {:?}",
            schema
                .cols
                .iter()
                .filter(|c| c.doc.is_none())
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn host_ops_event_class_doc() {
        let schema = host_ops_schema();
        let event_class = schema
            .cols
            .iter()
            .find(|c| c.name == "event_class")
            .expect("event_class column");
        assert!(event_class.doc.as_ref().unwrap().contains("host_hccl_op"));
    }
}
