use std::sync::Arc;

use datafusion::arrow::array::{Int32Array, Int64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::arrow::record_batch::RecordBatch;

use probing_core::core::{CustomTable, TableProbeDataSource};

use super::collector::cached_devices;

#[derive(Default, Debug)]
pub struct GpuDevicesTable {}

impl CustomTable for GpuDevicesTable {
    fn name() -> &'static str {
        "devices"
    }

    fn schema() -> SchemaRef {
        SchemaRef::new(Schema::new(vec![
            Field::new("backend", DataType::Utf8, false),
            Field::new("device_id", DataType::Int32, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("memory_model", DataType::Utf8, false),
            Field::new("chip", DataType::Utf8, true),
            Field::new("uuid", DataType::Utf8, true),
            Field::new("compute_capability", DataType::Utf8, true),
            Field::new("registry_id", DataType::Int64, true),
            Field::new("total_mem_bytes", DataType::Int64, false),
        ]))
    }

    fn data() -> Vec<RecordBatch> {
        let devices = cached_devices();
        if devices.is_empty() {
            return Vec::new();
        }

        let mut backends = Vec::with_capacity(devices.len());
        let mut ids = Vec::with_capacity(devices.len());
        let mut names = Vec::with_capacity(devices.len());
        let mut memory_models = Vec::with_capacity(devices.len());
        let mut chips: Vec<Option<&str>> = Vec::with_capacity(devices.len());
        let mut uuids: Vec<Option<&str>> = Vec::with_capacity(devices.len());
        let mut compute_caps: Vec<Option<&str>> = Vec::with_capacity(devices.len());
        let mut registry_ids: Vec<Option<i64>> = Vec::with_capacity(devices.len());
        let mut total_mem = Vec::with_capacity(devices.len());

        for device in &devices {
            backends.push(device.backend.as_str());
            ids.push(device.ordinal);
            names.push(device.name.as_str());
            memory_models.push(device.memory_model.as_str());
            chips.push(device.chip.as_deref());
            uuids.push(device.uuid.as_deref());
            compute_caps.push(device.compute_capability.as_deref());
            registry_ids.push(device.registry_id.map(|v| v as i64));
            total_mem.push(device.total_mem_bytes as i64);
        }

        let batch = RecordBatch::try_new(
            Self::schema(),
            vec![
                Arc::new(StringArray::from(backends)),
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(names)),
                Arc::new(StringArray::from(memory_models)),
                Arc::new(StringArray::from(chips)),
                Arc::new(StringArray::from(uuids)),
                Arc::new(StringArray::from(compute_caps)),
                Arc::new(Int64Array::from(registry_ids)),
                Arc::new(Int64Array::from(total_mem)),
            ],
        );

        batch.map(|b| vec![b]).unwrap_or_default()
    }
}

pub type GpuDevicesProbeDataSource = TableProbeDataSource<GpuDevicesTable>;
