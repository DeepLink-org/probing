use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Float32Array, Float64Array, Int32Array, Int64Array, RecordBatch,
    StringArray, TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datafusion::error::{DataFusionError, Result};
use probing_proto::prelude::{DataFrame, Seq};

/// Primary node identity column on `global.*` query results (hostname, or addr if host is empty).
pub const PROBE_NODE_COL: &str = "_probe_node";
pub const PROBE_HOST_COL: &str = "_host";
pub const PROBE_ADDR_COL: &str = "_addr";
/// Cluster `rank` from `cluster.nodes` for the row's source probing endpoint.
pub const PROBE_RANK_COL: &str = "_rank";
/// Parallel-role key (e.g. "dp=2,pp=1,tp=0") for the row's source endpoint.
pub const PROBE_ROLE_COL: &str = "_role";

#[cfg_attr(not(test), allow(dead_code))]
pub fn node_label(host: &str, addr: &str) -> String {
    if host.is_empty() {
        addr.to_string()
    } else {
        host.to_string()
    }
}

/// Resolve cluster rank for a probing endpoint (`host` + `addr` key in CLUSTER).
pub fn cluster_rank_for_endpoint(host: &str, addr: &str) -> Option<i32> {
    use crate::core::cluster::CLUSTER;

    CLUSTER
        .read()
        .ok()
        .and_then(|c| c.get_by_addr(host, addr).and_then(|n| n.rank))
        .or_else(|| {
            crate::core::cluster::get_nodes()
                .into_iter()
                .find(|n| n.addr == addr)
                .and_then(|n| n.rank)
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

pub fn federated_output_schema(local: SchemaRef) -> SchemaRef {
    let mut fields = local.fields().to_vec();
    for (name, dtype, nullable) in [
        (PROBE_HOST_COL, DataType::Utf8, false),
        (PROBE_ADDR_COL, DataType::Utf8, false),
        (PROBE_RANK_COL, DataType::Int32, true),
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
        PROBE_NODE_COL | PROBE_HOST_COL | PROBE_ADDR_COL | PROBE_RANK_COL | PROBE_ROLE_COL
    )
}

/// Attach federation node columns to an in-memory query result.
pub fn tag_proto_dataframe(df: &mut DataFrame, host: &str, addr: &str, rank: Option<i32>) {
    if df.is_empty() {
        return;
    }
    let rows = df.len();
    let role = cluster_role_for_endpoint(host, addr).unwrap_or_default();
    df.names.push(PROBE_HOST_COL.to_string());
    df.names.push(PROBE_ADDR_COL.to_string());
    df.names.push(PROBE_RANK_COL.to_string());
    df.names.push(PROBE_ROLE_COL.to_string());
    df.cols.push(Seq::SeqText(vec![host.to_string(); rows]));
    df.cols.push(Seq::SeqText(vec![addr.to_string(); rows]));
    df.cols.push(Seq::SeqI32(vec![rank.unwrap_or(-1); rows]));
    df.cols.push(Seq::SeqText(vec![role; rows]));
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
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .map_err(|e| DataFusionError::Execution(format!("proto dataframe conversion failed: {e}")))
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

    let rank = rank.or_else(|| cluster_rank_for_endpoint(host, addr));
    let role = cluster_role_for_endpoint(host, addr).unwrap_or_default();
    let mut columns = Vec::with_capacity(df.cols.len() + 4);
    let mut fields = Vec::with_capacity(df.names.len() + 4);

    for (name, col) in df.names.iter().zip(df.cols.iter()) {
        fields.push(Field::new(name, array_data_type(col), true));
        columns.push(seq_to_array(col)?);
    }

    let rows = df.len();
    fields.push(Field::new(PROBE_HOST_COL, DataType::Utf8, false));
    fields.push(Field::new(PROBE_ADDR_COL, DataType::Utf8, false));
    fields.push(Field::new(PROBE_RANK_COL, DataType::Int32, true));
    fields.push(Field::new(PROBE_ROLE_COL, DataType::Utf8, true));
    columns.push(Arc::new(StringArray::from(vec![host.to_string(); rows])));
    columns.push(Arc::new(StringArray::from(vec![addr.to_string(); rows])));
    columns.push(Arc::new(Int32Array::from(vec![rank; rows])));
    columns.push(Arc::new(StringArray::from(vec![role; rows])));

    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .map_err(|e| DataFusionError::Execution(format!("dataframe conversion failed: {e}")))
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

    let rank = rank.or_else(|| cluster_rank_for_endpoint(host, addr));
    let rows = batch.num_rows();
    let mut fields = batch.schema().fields().to_vec();
    let mut columns = batch.columns().to_vec();

    if !fields.iter().any(|f| f.name() == PROBE_HOST_COL) {
        fields.push(Arc::new(Field::new(PROBE_HOST_COL, DataType::Utf8, false)));
        columns.push(Arc::new(StringArray::from(vec![host.to_string(); rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_ADDR_COL) {
        fields.push(Arc::new(Field::new(PROBE_ADDR_COL, DataType::Utf8, false)));
        columns.push(Arc::new(StringArray::from(vec![addr.to_string(); rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_RANK_COL) {
        fields.push(Arc::new(Field::new(PROBE_RANK_COL, DataType::Int32, true)));
        columns.push(Arc::new(Int32Array::from(vec![rank; rows])));
    }
    if !fields.iter().any(|f| f.name() == PROBE_ROLE_COL) {
        let role = cluster_role_for_endpoint(host, addr).unwrap_or_default();
        fields.push(Arc::new(Field::new(PROBE_ROLE_COL, DataType::Utf8, true)));
        columns.push(Arc::new(StringArray::from(vec![role; rows])));
    }

    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)
        .map_err(|e| DataFusionError::Execution(format!("tagging batch failed: {e}")))
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

    RecordBatch::try_new(Arc::new(schema.clone()), columns)
        .map_err(|e| DataFusionError::Execution(format!("align batch failed: {e}")))
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

    use super::*;

    #[test]
    fn node_label_prefers_host() {
        assert_eq!(node_label("node-a", "10.0.0.1:8080"), "node-a");
    }

    #[test]
    fn node_label_falls_back_to_addr() {
        assert_eq!(node_label("", "10.0.0.2:8080"), "10.0.0.2:8080");
    }

    #[test]
    fn federated_schema_includes_tag_columns() {
        let local = Arc::new(Schema::new(vec![Field::new(
            "rank",
            DataType::Int32,
            false,
        )]));
        let schema = federated_output_schema(local);
        assert!(schema.index_of(PROBE_HOST_COL).is_ok());
        assert!(schema.index_of(PROBE_ADDR_COL).is_ok());
        assert!(schema.index_of(PROBE_RANK_COL).is_ok());
        assert!(schema.index_of(PROBE_ROLE_COL).is_ok());
        assert!(schema.index_of(PROBE_NODE_COL).is_err());
    }

    #[test]
    fn tag_record_batch_adds_probe_columns() {
        let local = Arc::new(Schema::new(vec![Field::new(
            "rank",
            DataType::Int32,
            false,
        )]));
        let batch = RecordBatch::try_new(local, vec![Arc::new(Int32Array::from(vec![7]))]).unwrap();
        let tagged = tag_record_batch(batch, "host-a", "10.0.0.1:8080", Some(3)).unwrap();
        assert_eq!(tagged.num_columns(), 5);
        assert_eq!(
            tagged
                .column(tagged.schema().index_of(PROBE_HOST_COL).unwrap())
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "host-a"
        );
        assert_eq!(
            tagged
                .column(tagged.schema().index_of(PROBE_ADDR_COL).unwrap())
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "10.0.0.1:8080"
        );
        assert_eq!(
            tagged
                .column(tagged.schema().index_of(PROBE_RANK_COL).unwrap())
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .value(0),
            3
        );
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
    fn extend_projection_honors_tag_only_selection() {
        let local = Arc::new(Schema::new(vec![Field::new(
            "rank",
            DataType::Int32,
            false,
        )]));
        let schema = federated_output_schema(local);
        let rank_idx = schema.index_of(PROBE_RANK_COL).unwrap();
        let extended = extend_projection_with_probe_tags(Some(&vec![rank_idx]), &schema).unwrap();
        assert_eq!(extended, vec![rank_idx]);
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
