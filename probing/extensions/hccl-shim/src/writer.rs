//! Mmap writer for intercepted MSProf events.

use std::sync::atomic::{AtomicBool, Ordering};

use probing_memtable::discover::ExposedTable;
use probing_memtable::Value;

use crate::msprof::{
    Mc2CommInfo, MsprofAdditionalInfoHeader, MsprofApi, MsprofCompactInfoHeader,
    MsprofContextIdInfo, MsprofHCCLOPInfo, MsprofHcclInfo,
};
use crate::names::{classify_api_event, is_hccl_op_name, lookup_hash, lookup_type_id};
use crate::tables::{
    collectives_schema, context_ids_schema, host_ops_schema, mc2_streams_schema, tasks_schema,
    COLLECTIVES_FILE, CONTEXT_IDS_FILE, HOST_OPS_FILE, MC2_STREAMS_FILE, TASKS_FILE,
};

const CHUNK_SIZE: u32 = 16 * 1024;
const NUM_CHUNKS: u32 = 32;

macro_rules! open_table {
    ($self:ident, $field:ident, $failed:ident, $logged:ident, $file:expr, $schema:expr) => {{
        if $self.$field.is_none() && !$self.$failed.load(Ordering::Relaxed) {
            match ExposedTable::create($file, &$schema(), CHUNK_SIZE, NUM_CHUNKS) {
                Ok(t) => $self.$field = Some(t),
                Err(e) => {
                    $self.$failed.store(true, Ordering::Relaxed);
                    if $self
                        .$logged
                        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok()
                    {
                        eprintln!("[probing-hccl-shim] failed to open {}: {e}", $file);
                    }
                }
            }
        }
        $self.$field.as_mut().ok_or(())
    }};
}

pub struct HcclWriter {
    host_table: Option<ExposedTable>,
    tasks_table: Option<ExposedTable>,
    collectives_table: Option<ExposedTable>,
    mc2_table: Option<ExposedTable>,
    context_table: Option<ExposedTable>,
    host_failed: AtomicBool,
    tasks_failed: AtomicBool,
    collectives_failed: AtomicBool,
    mc2_failed: AtomicBool,
    context_failed: AtomicBool,
    logged_host: AtomicBool,
    logged_tasks: AtomicBool,
    logged_collectives: AtomicBool,
    logged_mc2: AtomicBool,
    logged_context: AtomicBool,
}

impl HcclWriter {
    pub fn new() -> Self {
        Self {
            host_table: None,
            tasks_table: None,
            collectives_table: None,
            mc2_table: None,
            context_table: None,
            host_failed: AtomicBool::new(false),
            tasks_failed: AtomicBool::new(false),
            collectives_failed: AtomicBool::new(false),
            mc2_failed: AtomicBool::new(false),
            context_failed: AtomicBool::new(false),
            logged_host: AtomicBool::new(false),
            logged_tasks: AtomicBool::new(false),
            logged_collectives: AtomicBool::new(false),
            logged_mc2: AtomicBool::new(false),
            logged_context: AtomicBool::new(false),
        }
    }

    fn open_host(&mut self) -> Result<&mut ExposedTable, ()> {
        open_table!(
            self,
            host_table,
            host_failed,
            logged_host,
            HOST_OPS_FILE,
            host_ops_schema
        )
    }

    fn open_tasks(&mut self) -> Result<&mut ExposedTable, ()> {
        open_table!(
            self,
            tasks_table,
            tasks_failed,
            logged_tasks,
            TASKS_FILE,
            tasks_schema
        )
    }

    fn open_collectives(&mut self) -> Result<&mut ExposedTable, ()> {
        open_table!(
            self,
            collectives_table,
            collectives_failed,
            logged_collectives,
            COLLECTIVES_FILE,
            collectives_schema
        )
    }

    fn open_mc2(&mut self) -> Result<&mut ExposedTable, ()> {
        open_table!(
            self,
            mc2_table,
            mc2_failed,
            logged_mc2,
            MC2_STREAMS_FILE,
            mc2_streams_schema
        )
    }

    fn open_context(&mut self) -> Result<&mut ExposedTable, ()> {
        open_table!(
            self,
            context_table,
            context_failed,
            logged_context,
            CONTEXT_IDS_FILE,
            context_ids_schema
        )
    }

    pub fn record_api(&mut self, aging: u32, api: &MsprofApi) {
        let item_name = lookup_hash(api.item_id);
        let event_class = classify_api_event(api.level, api.type_id, api.item_id);
        let duration = api.end_time.saturating_sub(api.begin_time) as i64;

        if let Ok(table) = self.open_host() {
            table.push_row(&[
                Value::I64(api.end_time as i64),
                Value::I64(api.begin_time as i64),
                Value::I64(api.end_time as i64),
                Value::I64(duration),
                Value::I32(api.thread_id as i32),
                Value::I32(api.level as i32),
                Value::I32(api.type_id as i32),
                Value::U64(api.item_id),
                Value::Str(&item_name),
                Value::Str(event_class),
                Value::I32(aging as i32),
            ]);
        }

        if event_class == "host_hccl_op" || is_hccl_op_name(&item_name) {
            if let Ok(table) = self.open_collectives() {
                table.push_row(&[
                    Value::I64(api.end_time as i64),
                    Value::I32(api.thread_id as i32),
                    Value::Str("api"),
                    Value::I64(api.begin_time as i64),
                    Value::I64(api.end_time as i64),
                    Value::I64(duration),
                    Value::U64(api.item_id),
                    Value::Str(&item_name),
                    Value::U64(0),
                    Value::U64(0),
                    Value::U64(0),
                    Value::I32(-1),
                    Value::I32(0),
                    Value::I32(0),
                    Value::I32(0),
                ]);
            }
        }
    }

    pub fn record_compact_hccl_op(
        &mut self,
        header: &MsprofCompactInfoHeader,
        op: &MsprofHCCLOPInfo,
    ) {
        let Ok(table) = self.open_collectives() else {
            return;
        };
        table.push_row(&[
            Value::I64(header.time_stamp as i64),
            Value::I32(header.thread_id as i32),
            Value::Str("compact"),
            Value::I64(0),
            Value::I64(0),
            Value::I64(0),
            Value::U64(0),
            Value::Str(""),
            Value::U64(op.group_name),
            Value::U64(op.alg_type),
            Value::U64(op.count),
            Value::I32(op.data_type as i32),
            Value::I32(op.relay as i32),
            Value::I32(op.retry as i32),
            Value::I32(header.type_id as i32),
        ]);
    }

    pub fn record_task(
        &mut self,
        header: &MsprofAdditionalInfoHeader,
        hccl: &MsprofHcclInfo,
        payload_len: i32,
    ) {
        let Ok(table) = self.open_tasks() else {
            return;
        };
        let type_name = lookup_type_id(header.type_id);
        let task_name = lookup_hash(hccl.item_id);
        let (plane_index, rank_in_plane, rank_size_plane) =
            crate::msprof::decode_plane(hccl.plane_id);
        table.push_row(&[
            Value::I64(header.time_stamp as i64),
            Value::I32(header.thread_id as i32),
            Value::I32(header.type_id as i32),
            Value::I32(header.level as i32),
            Value::Str(&type_name),
            Value::U64(hccl.item_id),
            Value::Str(&task_name),
            Value::U64(hccl.ccl_tag),
            Value::U64(hccl.group_name),
            Value::I32(hccl.local_rank as i32),
            Value::I32(hccl.remote_rank as i32),
            Value::I32(hccl.rank_size as i32),
            Value::I32(hccl.workflow_mode as i32),
            Value::I32(hccl.plane_id as i32),
            Value::I32(plane_index),
            Value::I32(rank_in_plane),
            Value::I32(rank_size_plane),
            Value::I32(hccl.ctx_id as i32),
            Value::U64(hccl.notify_id),
            Value::I32(hccl.stage as i32),
            Value::I32(hccl.role as i32),
            Value::U64(hccl.data_size),
            Value::I32(hccl.op_type as i32),
            Value::I32(hccl.data_type as i32),
            Value::I32(hccl.link_type as i32),
            Value::I32(hccl.transport_type as i32),
            Value::I32(hccl.rdma_type as i32),
            Value::F64(hccl.duration_estimated),
            Value::I32(payload_len),
        ]);
    }

    pub fn record_mc2(&mut self, header: &MsprofAdditionalInfoHeader, mc2: &Mc2CommInfo) {
        let Ok(table) = self.open_mc2() else {
            return;
        };
        let ids = mc2
            .comm_stream_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        table.push_row(&[
            Value::I64(header.time_stamp as i64),
            Value::I32(header.thread_id as i32),
            Value::I32(header.type_id as i32),
            Value::U64(mc2.header.group_name),
            Value::I32(mc2.header.rank_size as i32),
            Value::I32(mc2.header.rank_id as i32),
            Value::I32(mc2.header.usr_rank_id as i32),
            Value::I32(mc2.header.aicpu_kfc_stream_id as i32),
            Value::I32(mc2.comm_stream_size as i32),
            Value::Str(&ids),
        ]);
    }

    pub fn record_context(
        &mut self,
        header: &MsprofAdditionalInfoHeader,
        ctx: &MsprofContextIdInfo,
    ) {
        let Ok(table) = self.open_context() else {
            return;
        };
        let ctx_min = ctx.ctx_ids.first().copied().unwrap_or(0);
        let ctx_max = if ctx.ctx_id_num >= 2 {
            ctx.ctx_ids[1]
        } else {
            ctx_min
        };
        table.push_row(&[
            Value::I64(header.time_stamp as i64),
            Value::I32(header.thread_id as i32),
            Value::I32(header.type_id as i32),
            Value::I32(ctx.ctx_id_num as i32),
            Value::I32(ctx_min as i32),
            Value::I32(ctx_max as i32),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msprof::{MSPROF_HCCL_INFO_MIN, MSPROF_HCCL_OP_INFO_MIN};
    use probing_memtable::discover::discover_in;
    use std::fs;

    fn test_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("probing_hccl_shim_test_{}", std::process::id()))
    }

    #[test]
    fn tasks_mmap_roundtrip() {
        let base = test_dir();
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let mut w = HcclWriter::new();
        let header = MsprofAdditionalInfoHeader {
            level: 1,
            type_id: 0,
            thread_id: 42,
            data_len: MSPROF_HCCL_INFO_MIN as u32,
            time_stamp: 999,
        };
        let info = MsprofHcclInfo {
            item_id: 1,
            plane_id: (2u64 << 28 | 4u64 << 16 | 7) as u32,
            data_size: 4096,
            remote_rank: 3,
            ..Default::default()
        };
        w.record_task(&header, &info, header.data_len as i32);

        let found = discover_in(&base).unwrap();
        assert!(found.iter().any(|t| t.name() == TASKS_FILE));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn collectives_compact_roundtrip() {
        let base = test_dir();
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let mut w = HcclWriter::new();
        let header = MsprofCompactInfoHeader {
            level: 2,
            type_id: 100,
            thread_id: 7,
            data_len: MSPROF_HCCL_OP_INFO_MIN as u32,
            time_stamp: 1234,
        };
        let op = MsprofHCCLOPInfo {
            count: 1024,
            data_type: 1,
            group_name: 99,
            alg_type: 88,
            ..Default::default()
        };
        w.record_compact_hccl_op(&header, &op);

        let found = discover_in(&base).unwrap();
        assert!(found.iter().any(|t| t.name() == COLLECTIVES_FILE));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn host_ops_schema_columns() {
        let base = test_dir();
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let mut w = HcclWriter::new();
        w.record_api(
            1,
            &MsprofApi {
                level: 3,
                type_id: 1,
                thread_id: 1,
                begin_time: 100,
                end_time: 250,
                item_id: 42,
                ..Default::default()
            },
        );

        let found = discover_in(&base).unwrap();
        assert!(found.iter().any(|t| t.name() == HOST_OPS_FILE));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn context_ids_roundtrip() {
        let base = test_dir();
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("PROBING_DATA_DIR", &base);

        let mut w = HcclWriter::new();
        let header = MsprofAdditionalInfoHeader {
            level: 2,
            type_id: 200,
            thread_id: 1,
            data_len: 12,
            time_stamp: 500,
        };
        w.record_context(
            &header,
            &MsprofContextIdInfo {
                ctx_id_num: 2,
                ctx_ids: [0, 15],
            },
        );

        let found = discover_in(&base).unwrap();
        assert!(found.iter().any(|t| t.name() == CONTEXT_IDS_FILE));
        let _ = fs::remove_dir_all(&base);
    }
}
