use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array, RecordBatch,
    StringArray, TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datafusion::error::{DataFusionError, Result};
use probing_proto::prelude::{DataFrame, Seq};

pub const PROBE_HOST_COL: &str = "_host";
pub const PROBE_ADDR_COL: &str = "_addr";
/// Cluster `rank` from `cluster.nodes` for the row's source probing endpoint.
pub const PROBE_RANK_COL: &str = "_rank";
/// Node/worker group rank (`GROUP_RANK` / `group_rank` on the endpoint).
pub const PROBE_NODE_RANK_COL: &str = "_node_rank";
/// Intra-node GPU index (`LOCAL_RANK` / `local_rank` on the endpoint).
pub const PROBE_LOCAL_RANK_COL: &str = "_local_rank";
/// Parallel-role key (e.g. "dp=2,pp=1,tp=0") for the row's source endpoint.
pub const PROBE_ROLE_COL: &str = "_role";

/// Fixed federation tag columns appended to `global.*` results (stable order).
pub const FEDERATION_TAG_COLUMNS: &[&str] = &[
    PROBE_HOST_COL,
    PROBE_ADDR_COL,
    PROBE_RANK_COL,
    PROBE_NODE_RANK_COL,
    PROBE_LOCAL_RANK_COL,
    PROBE_ROLE_COL,
];

fn record_batch(schema: Schema, columns: Vec<ArrayRef>, ctx: &'static str) -> Result<RecordBatch> {
    RecordBatch::try_new(Arc::new(schema), columns)
        .map_err(|e| DataFusionError::ArrowError(Box::new(e), Some(ctx.to_string())))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationEndpointTags {
    pub host: String,
    pub addr: String,
    pub rank: i32,
    pub node_rank: i32,
    pub local_rank: i32,
    pub role: String,
}

#[cfg(test)]
fn node_label(host: &str, addr: &str) -> String {
    if host.is_empty() {
        addr.to_string()
    } else {
        host.to_string()
    }
}

/// Resolve cluster rank for a probing endpoint (`host` + `addr` key in CLUSTER).
pub fn cluster_rank_for_endpoint(host: &str, addr: &str) -> Option<i32> {
    cluster_node_field(host, addr, |n| n.rank)
}

/// Resolve node/worker group rank for a probing endpoint.
pub fn cluster_node_rank_for_endpoint(host: &str, addr: &str) -> Option<i32> {
    cluster_node_field(host, addr, |n| n.group_rank)
}

/// Resolve intra-node local rank for a probing endpoint.
pub fn cluster_local_rank_for_endpoint(host: &str, addr: &str) -> Option<i32> {
    cluster_node_field(host, addr, |n| n.local_rank)
}

fn cluster_node_field<F>(host: &str, addr: &str, field: F) -> Option<i32>
where
    F: Fn(&probing_proto::prelude::Node) -> Option<i32>,
{
    use crate::core::cluster::CLUSTER;

    CLUSTER
        .read()
        .ok()
        .and_then(|c| c.get_by_addr(host, addr).and_then(&field))
        .or_else(|| {
            crate::core::cluster::get_nodes()
                .into_iter()
                .find(|n| n.addr == addr)
                .and_then(|n| field(&n))
        })
}

/// Resolve parallel-role key for a probing endpoint (`host` + `addr` key in CLUSTER).
pub fn cluster_role_for_endpoint(host: &str, addr: &str) -> Option<String> {
    use crate::core::cluster::CLUSTER;

    CLUSTER
        .read()
        .ok()
        .and_then(|c| c.get_by_addr(host, addr).and_then(|n| n.role.clone()))
        .or_else(|| {
            crate::core::cluster::get_nodes()
                .into_iter()
                .find(|n| n.addr == addr)
                .and_then(|n| n.role)
        })
        .filter(|r| !r.is_empty())
}

pub fn federation_tags_for_endpoint(host: &str, addr: &str) -> FederationEndpointTags {
    FederationEndpointTags {
        host: host.to_string(),
        addr: addr.to_string(),
        rank: cluster_rank_for_endpoint(host, addr).unwrap_or(-1),
        node_rank: cluster_node_rank_for_endpoint(host, addr).unwrap_or(-1),
        local_rank: cluster_local_rank_for_endpoint(host, addr).unwrap_or(-1),
        role: cluster_role_for_endpoint(host, addr).unwrap_or_default(),
    }
}

pub fn federated_output_schema(local: SchemaRef) -> SchemaRef {
    let mut fields = local.fields().to_vec();
    for (name, dtype, nullable) in [
        (PROBE_HOST_COL, DataType::Utf8, false),
        (PROBE_ADDR_COL, DataType::Utf8, false),
        (PROBE_RANK_COL, DataType::Int32, true),
        (PROBE_NODE_RANK_COL, DataType::Int32, true),
        (PROBE_LOCAL_RANK_COL, DataType::Int32, true),
        (PROBE_ROLE_COL, DataType::Utf8, true),
    ] {
        if !fields.iter().any(|f| f.name() == name) {
            fields.push(Arc::new(Field::new(name, dtype, nullable)));
        }
    }
    Arc::new(Schema::new(fields))
}

pub fn is_federation_tag_column(name: &str) -> bool {
    matches!(
        name,
        PROBE_HOST_COL
            | PROBE_ADDR_COL
            | PROBE_RANK_COL
            | PROBE_NODE_RANK_COL
            | PROBE_LOCAL_RANK_COL
            | PROBE_ROLE_COL
    )
}

/// Attach federation node columns to an in-memory query result.
pub fn tag_proto_dataframe(df: &mut DataFrame, host: &str, addr: &str, rank: Option<i32>) {
    if df.is_empty() {
        return;
    }
    let mut tags = federation_tags_for_endpoint(host, addr);
    if let Some(rank) = rank {
        tags.rank = rank;
    }
    tag_proto_dataframe_with_tags(df, &tags);
}

pub(crate) fn tag_proto_dataframe_with_tags(df: &mut DataFrame, tags: &FederationEndpointTags) {
    if df.is_empty() {
        return;
    }
    append_proto_tags(df, tags);
}

fn append_proto_tags(df: &mut DataFrame, tags: &FederationEndpointTags) {
    let rows = df.len();
    df.names.push(PROBE_HOST_COL.to_string());
    df.names.push(PROBE_ADDR_COL.to_string());
    df.names.push(PROBE_RANK_COL.to_string());
    df.names.push(PROBE_NODE_RANK_COL.to_string());
    df.names.push(PROBE_LOCAL_RANK_COL.to_string());
    df.names.push(PROBE_ROLE_COL.to_string());
    df.cols.push(Seq::SeqText(vec![tags.host.clone(); rows]));
    df.cols.push(Seq::SeqText(vec![tags.addr.clone(); rows]));
    df.cols.push(Seq::SeqI32(vec![tags.rank; rows]));
    df.cols.push(Seq::SeqI32(vec![tags.node_rank; rows]));
    df.cols.push(Seq::SeqI32(vec![tags.local_rank; rows]));
    df.cols.push(Seq::SeqText(vec![tags.role.clone(); rows]));
    df.size = df.len() as u64;
}

/// Convert a protocol dataframe to a record batch without adding federation tags.
pub fn proto_dataframe_to_record_batch(df: &DataFrame) -> Result<RecordBatch> {
    if df.is_empty() {
        return Ok(RecordBatch::new_empty(Arc::new(Schema::empty())));
    }
    let mut columns = Vec::with_capacity(df.cols.len());
    let mut fields = Vec::with_capacity(df.names.len());
    for (name, col) in df.names.iter().zip(df.cols.iter()) {
        fields.push(Field::new(name, array_data_type(col), true));
        columns.push(seq_to_array(col)?);
    }
    record_batch(
        Schema::new(fields),
        columns,
        "proto dataframe conversion failed",
    )
}

/// Honor the caller's column projection for `global.*` scans.
pub fn extend_projection_with_probe_tags(
    projection: Option<&Vec<usize>>,
    _schema: &SchemaRef,
) -> Option<Vec<usize>> {
    projection.cloned()
}

fn seq_to_array(seq: &Seq) -> Result<ArrayRef> {
    match seq {
        Seq::SeqI32(values) => Ok(Arc::new(Int32Array::from(values.clone()))),
        Seq::SeqI64(values) => Ok(Arc::new(Int64Array::from(values.clone()))),
        Seq::SeqF32(values) => Ok(Arc::new(Float32Array::from(values.clone()))),
        Seq::SeqF64(values) => Ok(Arc::new(Float64Array::from(values.clone()))),
        Seq::SeqText(values) => Ok(Arc::new(StringArray::from(values.clone()))),
        Seq::SeqBOOL(values) => Ok(Arc::new(BooleanArray::from(values.clone()))),
        Seq::SeqDateTime(values) => Ok(Arc::new(Int64Array::from(
            values.iter().map(|v| *v as i64).collect::<Vec<_>>(),
        ))),
        Seq::Nil => Ok(Arc::new(StringArray::from(Vec::<String>::new()))),
    }
}

pub fn dataframe_to_record_batch(
    df: &DataFrame,
    host: &str,
    addr: &str,
    rank: Option<i32>,
) -> Result<RecordBatch> {
    if df.is_empty() {
        return Ok(RecordBatch::new_empty(Arc::new(Schema::empty())));
    }

    let mut tags = federation_tags_for_endpoint(host, addr);
    if let Some(rank) = rank {
        tags.rank = rank;
    }
    let mut columns = Vec::with_capacity(df.cols.len() + FEDERATION_TAG_COLUMNS.len());
    let mut fields = Vec::with_capacity(df.names.len() + FEDERATION_TAG_COLUMNS.len());

    for (name, col) in df.names.iter().zip(df.cols.iter()) {
        fields.push(Field::new(name, array_data_type(col), true));
        columns.push(seq_to_array(col)?);
    }

    let rows = df.len();
    fields.push(Field::new(PROBE_HOST_COL, DataType::Utf8, false));
    fields.push(Field::new(PROBE_ADDR_COL, DataType::Utf8, false));
    fields.push(Field::new(PROBE_RANK_COL, DataType::Int32, true));
    fields.push(Field::new(PROBE_NODE_RANK_COL, DataType::Int32, true));
    fields.push(Field::new(PROBE_LOCAL_RANK_COL, DataType::Int32, true));
    fields.push(Field::new(PROBE_ROLE_COL, DataType::Utf8, true));
    columns.push(Arc::new(StringArray::from(vec![tags.host; rows])));
    columns.push(Arc::new(StringArray::from(vec![tags.addr; rows])));
    columns.push(Arc::new(Int32Array::from(vec![tags.rank; rows])));
    columns.push(Arc::new(Int32Array::from(vec![tags.node_rank; rows])));
    columns.push(Arc::new(Int32Array::from(vec![tags.local_rank; rows])));
    columns.push(Arc::new(StringArray::from(vec![tags.role; rows])));

    record_batch(Schema::new(fields), columns, "dataframe conversion failed")
}

pub fn tag_record_batch(
    batch: RecordBatch,
    host: &str,
    addr: &str,
    rank: Option<i32>,
) -> Result<RecordBatch> {
    if batch.num_rows() == 0 {
        return Ok(batch);
    }

    let mut tags = federation_tags_for_endpoint(host, addr);
    if let Some(rank) = rank {
        tags.rank = rank;
    }
    let rows = batch.num_rows();
    let mut fields = batch.schema().fields().to_vec();
    let mut columns = batch.columns().to_vec();

    append_batch_tags(&mut fields, &mut columns, rows, &tags)?;

    record_batch(Schema::new(fields), columns, "tagging batch failed")
}

fn append_batch_tags(
    fields: &mut Vec<Arc<Field>>,
    columns: &mut Vec<ArrayRef>,
    rows: usize,
    tags: &FederationEndpointTags,
) -> Result<()> {
    if !fields.iter().any(|f| f.name() == PROBE_HOST_COL) {
        fields.push(Arc::new(Field::new(PROBE_HOST_COL, DataType::Utf8, false)));
        columns.push(Arc::new(StringArray::from(vec![tags.host.as_str(); rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_ADDR_COL) {
        fields.push(Arc::new(Field::new(PROBE_ADDR_COL, DataType::Utf8, false)));
        columns.push(Arc::new(StringArray::from(vec![tags.addr.as_str(); rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_RANK_COL) {
        fields.push(Arc::new(Field::new(PROBE_RANK_COL, DataType::Int32, true)));
        columns.push(Arc::new(Int32Array::from(vec![tags.rank; rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_NODE_RANK_COL) {
        fields.push(Arc::new(Field::new(
            PROBE_NODE_RANK_COL,
            DataType::Int32,
            true,
        )));
        columns.push(Arc::new(Int32Array::from(vec![tags.node_rank; rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_LOCAL_RANK_COL) {
        fields.push(Arc::new(Field::new(
            PROBE_LOCAL_RANK_COL,
            DataType::Int32,
            true,
        )));
        columns.push(Arc::new(Int32Array::from(vec![tags.local_rank; rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_ROLE_COL) {
        fields.push(Arc::new(Field::new(PROBE_ROLE_COL, DataType::Utf8, true)));
        columns.push(Arc::new(StringArray::from(vec![tags.role.as_str(); rows])));
    }
    Ok(())
}

pub fn align_batch_to_schema(batch: RecordBatch, schema: &Schema) -> Result<RecordBatch> {
    if batch.schema().as_ref() == schema {
        return Ok(batch);
    }

    let mut columns = Vec::with_capacity(schema.fields().len());
    for field in schema.fields() {
        if let Ok(idx) = batch.schema().index_of(field.name()) {
            let existing = batch.column(idx);
            if existing.data_type() == field.data_type() {
                columns.push(existing.clone());
                continue;
            }
        }
        columns.push(empty_array_for_field(field, batch.num_rows())?);
    }

    record_batch(schema.clone(), columns, "align batch failed")
}

fn array_data_type(seq: &Seq) -> DataType {
    match seq {
        Seq::SeqI32(_) => DataType::Int32,
        Seq::SeqI64(_) | Seq::SeqDateTime(_) => DataType::Int64,
        Seq::SeqF32(_) => DataType::Float32,
        Seq::SeqF64(_) => DataType::Float64,
        Seq::SeqText(_) | Seq::Nil => DataType::Utf8,
        Seq::SeqBOOL(_) => DataType::Boolean,
    }
}

fn empty_array_for_field(field: &Field, rows: usize) -> Result<ArrayRef> {
    Ok(match field.data_type() {
        DataType::Int32 => Arc::new(Int32Array::from(vec![None::<i32>; rows])),
        DataType::Int64 => Arc::new(Int64Array::from(vec![None::<i64>; rows])),
        DataType::Float32 => Arc::new(Float32Array::from(vec![None::<f32>; rows])),
        DataType::Float64 => Arc::new(Float64Array::from(vec![None::<f64>; rows])),
        DataType::Utf8 | DataType::LargeUtf8 => {
            Arc::new(StringArray::from(vec![None::<&str>; rows]))
        }
        DataType::Boolean => Arc::new(BooleanArray::from(vec![None::<bool>; rows])),
        DataType::Timestamp(unit, _) => match unit {
            TimeUnit::Second => Arc::new(TimestampSecondArray::from(vec![None::<i64>; rows])),
            TimeUnit::Millisecond => {
                Arc::new(TimestampMillisecondArray::from(vec![None::<i64>; rows]))
            }
            TimeUnit::Microsecond => {
                Arc::new(TimestampMicrosecondArray::from(vec![None::<i64>; rows]))
            }
            TimeUnit::Nanosecond => {
                Arc::new(TimestampNanosecondArray::from(vec![None::<i64>; rows]))
            }
        },
        other => {
            return Err(DataFusionError::NotImplemented(format!(
                "unsupported federated column type: {other:?}"
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use probing_proto::prelude::Node;

    use super::*;
    use crate::core::cluster::{reset_cluster_for_tests, update_node};

    #[test]
    fn node_label_prefers_host() {
        assert_eq!(node_label("node-a", "10.0.0.1:8080"), "node-a");
    }

    #[test]
    fn node_label_falls_back_to_addr() {
        assert_eq!(node_label("", "10.0.0.2:8080"), "10.0.0.2:8080");
    }

    #[test]
    fn federated_schema_includes_six_tag_columns() {
        let local = Arc::new(Schema::new(vec![Field::new(
            "rank",
            DataType::Int32,
            false,
        )]));
        let schema = federated_output_schema(local);
        for col in FEDERATION_TAG_COLUMNS {
            assert!(schema.index_of(col).is_ok(), "missing tag column {col}");
        }
    }

    #[test]
    fn federation_tags_resolve_from_cluster_node() {
        reset_cluster_for_tests();
        update_node(Node {
            host: "host-a".into(),
            addr: "10.0.0.1:8080".into(),
            rank: Some(3),
            group_rank: Some(1),
            local_rank: Some(2),
            role: Some("dp=0".into()),
            ..Default::default()
        });
        let tags = federation_tags_for_endpoint("host-a", "10.0.0.1:8080");
        assert_eq!(tags.rank, 3);
        assert_eq!(tags.node_rank, 1);
        assert_eq!(tags.local_rank, 2);
        assert_eq!(tags.role, "dp=0");
    }

    #[test]
    fn tag_record_batch_adds_six_probe_columns() {
        let local = Arc::new(Schema::new(vec![Field::new(
            "rank",
            DataType::Int32,
            false,
        )]));
        let batch = RecordBatch::try_new(local, vec![Arc::new(Int32Array::from(vec![7]))]).unwrap();
        let tagged = tag_record_batch(batch, "host-a", "10.0.0.1:8080", Some(3)).unwrap();
        assert_eq!(tagged.num_columns(), 7);
        for col in FEDERATION_TAG_COLUMNS {
            assert!(tagged.schema().index_of(col).is_ok());
        }
    }

    #[test]
    fn extend_projection_honors_explicit_selection() {
        let local = Arc::new(Schema::new(vec![Field::new(
            "rank",
            DataType::Int32,
            false,
        )]));
        let schema = federated_output_schema(local);
        let extended = extend_projection_with_probe_tags(Some(&vec![0]), &schema).unwrap();
        assert_eq!(extended, vec![0]);
    }

    #[test]
    fn align_batch_fills_timestamp_column_for_empty_rows() {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("host", DataType::Utf8, false)])),
            vec![Arc::new(StringArray::from(Vec::<&str>::new()))],
        )
        .unwrap();
        let full = Schema::new(vec![
            Field::new("host", DataType::Utf8, false),
            Field::new(
                "timestamp",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
        ]);
        let aligned = align_batch_to_schema(batch, &full).unwrap();
        assert_eq!(aligned.num_columns(), 2);
        assert_eq!(aligned.num_rows(), 0);
    }
}
