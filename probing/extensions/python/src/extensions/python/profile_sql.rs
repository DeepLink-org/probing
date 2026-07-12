//! Typed RecordBatch builders for live `python.profile_*` virtual tables.

use std::sync::Arc;

use probing_core::core::{
    ArrayRef, DataType, Field, Float64Array, Int64Array, RecordBatch, Schema, SchemaRef,
    StringArray,
};
use pyo3::types::{PyAnyMethods, PyDict, PyDictMethods, PyList, PyListMethods};
use pyo3::{Bound, Python};

use super::tbls::{try_record_batch, PythonTableError, TableResult};

fn profile_capture_schema() -> SchemaRef {
    SchemaRef::new(Schema::new(vec![
        Field::new("capture_id", DataType::Utf8, true),
        Field::new("local_step", DataType::Int64, true),
        Field::new("global_step", DataType::Int64, true),
        Field::new("rank", DataType::Int64, true),
        Field::new("world_size", DataType::Int64, true),
        Field::new("role", DataType::Utf8, true),
        Field::new("trigger", DataType::Utf8, true),
        Field::new("steps_profiled", DataType::Int64, true),
        Field::new("wall_us", DataType::Int64, true),
        Field::new("started_at_us", DataType::Int64, true),
        Field::new("ended_at_us", DataType::Int64, true),
        Field::new("status", DataType::Utf8, true),
        Field::new("truncated", DataType::Int64, true),
        Field::new("event_count", DataType::Int64, true),
        Field::new("error", DataType::Utf8, true),
    ]))
}

fn profile_hotspot_schema() -> SchemaRef {
    SchemaRef::new(Schema::new(vec![
        Field::new("capture_id", DataType::Utf8, true),
        Field::new("local_step", DataType::Int64, true),
        Field::new("global_step", DataType::Int64, true),
        Field::new("rank", DataType::Int64, true),
        Field::new("bucket_kind", DataType::Utf8, true),
        Field::new("bucket_name", DataType::Utf8, true),
        Field::new("self_us", DataType::Int64, true),
        Field::new("wall_us", DataType::Int64, true),
        Field::new("calls", DataType::Int64, true),
        Field::new("pct_of_capture", DataType::Float64, true),
        Field::new("module_hint", DataType::Utf8, true),
    ]))
}

fn dict_opt_i64(dict: &Bound<'_, PyDict>, key: &str) -> Option<i64> {
    dict.get_item(key)
        .ok()
        .flatten()
        .and_then(|v| v.extract::<i64>().ok())
}

fn dict_opt_f64(dict: &Bound<'_, PyDict>, key: &str) -> Option<f64> {
    dict.get_item(key)
        .ok()
        .flatten()
        .and_then(|v| v.extract::<f64>().ok())
}

fn dict_opt_str(dict: &Bound<'_, PyDict>, key: &str) -> Option<String> {
    dict.get_item(key)
        .ok()
        .flatten()
        .and_then(|v| v.extract::<String>().ok())
}

fn empty_batch(schema: SchemaRef) -> TableResult<RecordBatch> {
    let columns: Vec<ArrayRef> = schema
        .fields()
        .iter()
        .map(|field| match field.data_type() {
            DataType::Utf8 => Arc::new(StringArray::from(Vec::<Option<String>>::new())) as ArrayRef,
            DataType::Float64 => {
                Arc::new(Float64Array::from(Vec::<Option<f64>>::new())) as ArrayRef
            }
            _ => Arc::new(Int64Array::from(Vec::<Option<i64>>::new())) as ArrayRef,
        })
        .collect();
    try_record_batch(schema, columns)
}

pub fn profile_capture_batches() -> TableResult<Vec<RecordBatch>> {
    Python::attach(|py| {
        let schema = profile_capture_schema();
        let module = py.import("probing.profiling.torch_profiler.sql")?;
        let raw = module.call_method0("profile_capture_rows")?;
        let rows = raw.cast::<PyList>()?;
        if rows.is_empty() {
            return Ok(vec![empty_batch(schema)?]);
        }

        let mut capture_id = Vec::new();
        let mut local_step = Vec::new();
        let mut global_step = Vec::new();
        let mut rank = Vec::new();
        let mut world_size = Vec::new();
        let mut role = Vec::new();
        let mut trigger = Vec::new();
        let mut steps_profiled = Vec::new();
        let mut wall_us = Vec::new();
        let mut started_at_us = Vec::new();
        let mut ended_at_us = Vec::new();
        let mut status = Vec::new();
        let mut truncated = Vec::new();
        let mut event_count = Vec::new();
        let mut error = Vec::new();

        for item in rows.iter() {
            let dict = item
                .cast::<PyDict>()
                .map_err(|_| PythonTableError::BatchBuild("profile_capture row not dict".into()))?;
            capture_id.push(dict_opt_str(dict, "capture_id"));
            local_step.push(dict_opt_i64(dict, "local_step"));
            global_step.push(dict_opt_i64(dict, "global_step"));
            rank.push(dict_opt_i64(dict, "rank"));
            world_size.push(dict_opt_i64(dict, "world_size"));
            role.push(dict_opt_str(dict, "role"));
            trigger.push(dict_opt_str(dict, "trigger"));
            steps_profiled.push(dict_opt_i64(dict, "steps_profiled"));
            wall_us.push(dict_opt_i64(dict, "wall_us"));
            started_at_us.push(dict_opt_i64(dict, "started_at_us"));
            ended_at_us.push(dict_opt_i64(dict, "ended_at_us"));
            status.push(dict_opt_str(dict, "status"));
            truncated.push(dict_opt_i64(dict, "truncated"));
            event_count.push(dict_opt_i64(dict, "event_count"));
            error.push(dict_opt_str(dict, "error"));
        }

        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(capture_id)),
            Arc::new(Int64Array::from(local_step)),
            Arc::new(Int64Array::from(global_step)),
            Arc::new(Int64Array::from(rank)),
            Arc::new(Int64Array::from(world_size)),
            Arc::new(StringArray::from(role)),
            Arc::new(StringArray::from(trigger)),
            Arc::new(Int64Array::from(steps_profiled)),
            Arc::new(Int64Array::from(wall_us)),
            Arc::new(Int64Array::from(started_at_us)),
            Arc::new(Int64Array::from(ended_at_us)),
            Arc::new(StringArray::from(status)),
            Arc::new(Int64Array::from(truncated)),
            Arc::new(Int64Array::from(event_count)),
            Arc::new(StringArray::from(error)),
        ];
        Ok(vec![try_record_batch(schema, columns)?])
    })
}

pub fn profile_hotspot_batches() -> TableResult<Vec<RecordBatch>> {
    Python::attach(|py| {
        let schema = profile_hotspot_schema();
        let module = py.import("probing.profiling.torch_profiler.sql")?;
        let raw = module.call_method0("profile_hotspot_rows")?;
        let rows = raw.cast::<PyList>()?;
        if rows.is_empty() {
            return Ok(vec![empty_batch(schema)?]);
        }

        let mut capture_id = Vec::new();
        let mut local_step = Vec::new();
        let mut global_step = Vec::new();
        let mut rank = Vec::new();
        let mut bucket_kind = Vec::new();
        let mut bucket_name = Vec::new();
        let mut self_us = Vec::new();
        let mut wall_us = Vec::new();
        let mut calls = Vec::new();
        let mut pct_of_capture = Vec::new();
        let mut module_hint = Vec::new();

        for item in rows.iter() {
            let dict = item
                .cast::<PyDict>()
                .map_err(|_| PythonTableError::BatchBuild("profile_hotspot row not dict".into()))?;
            capture_id.push(dict_opt_str(dict, "capture_id"));
            local_step.push(dict_opt_i64(dict, "local_step"));
            global_step.push(dict_opt_i64(dict, "global_step"));
            rank.push(dict_opt_i64(dict, "rank"));
            bucket_kind.push(dict_opt_str(dict, "bucket_kind"));
            bucket_name.push(dict_opt_str(dict, "bucket_name"));
            self_us.push(dict_opt_i64(dict, "self_us"));
            wall_us.push(dict_opt_i64(dict, "wall_us"));
            calls.push(dict_opt_i64(dict, "calls"));
            pct_of_capture.push(dict_opt_f64(dict, "pct_of_capture"));
            module_hint.push(dict_opt_str(dict, "module_hint"));
        }

        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(capture_id)),
            Arc::new(Int64Array::from(local_step)),
            Arc::new(Int64Array::from(global_step)),
            Arc::new(Int64Array::from(rank)),
            Arc::new(StringArray::from(bucket_kind)),
            Arc::new(StringArray::from(bucket_name)),
            Arc::new(Int64Array::from(self_us)),
            Arc::new(Int64Array::from(wall_us)),
            Arc::new(Int64Array::from(calls)),
            Arc::new(Float64Array::from(pct_of_capture)),
            Arc::new(StringArray::from(module_hint)),
        ];
        Ok(vec![try_record_batch(schema, columns)?])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_capture_schema_has_expected_columns() {
        let schema = profile_capture_schema();
        for col in [
            "capture_id",
            "local_step",
            "global_step",
            "rank",
            "status",
            "truncated",
            "event_count",
        ] {
            assert!(schema.field_with_name(col).is_ok(), "missing column {col}");
        }
    }

    #[test]
    fn profile_hotspot_schema_has_expected_columns() {
        let schema = profile_hotspot_schema();
        for col in [
            "capture_id",
            "bucket_kind",
            "bucket_name",
            "self_us",
            "pct_of_capture",
        ] {
            assert!(schema.field_with_name(col).is_ok(), "missing column {col}");
        }
    }

    #[test]
    fn empty_capture_batch_is_zero_rows_with_schema() {
        let schema = profile_capture_schema();
        let batch = empty_batch(schema).expect("empty capture batch");
        assert_eq!(batch.num_rows(), 0);
        assert!(batch.schema().field_with_name("capture_id").is_ok());
    }

    #[test]
    fn empty_hotspot_batch_is_zero_rows_with_schema() {
        let schema = profile_hotspot_schema();
        let batch = empty_batch(schema).expect("empty hotspot batch");
        assert_eq!(batch.num_rows(), 0);
        assert!(batch.schema().field_with_name("bucket_name").is_ok());
    }
}
