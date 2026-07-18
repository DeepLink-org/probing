use std::collections::HashMap;
use std::ffi::CString;
use std::sync::Arc;

use log::{debug, error};
use probing_core::core::{
    ArrayRef, CustomNamespace, DataType, Field, Float64Array, Int64Array, NamespaceProbeDataSource,
    RecordBatch, Schema, SchemaRef, StringArray,
};
use probing_proto::prelude::CallFrame;
use pyo3::types::PyAnyMethods;
use pyo3::types::PyDict;
use pyo3::types::PyDictMethods;
use pyo3::types::PyFloat;
use pyo3::types::PyInt;
use pyo3::types::PyList;
use pyo3::types::PyString;
use pyo3::Bound;
use pyo3::PyAny;
use pyo3::Python;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PythonTableError {
    #[error("invalid Python expression: {0}")]
    InvalidExpression(String),
    #[error("internal error building record batch: missing column {0}")]
    MissingColumn(String),
    #[error("record batch build failed: {0}")]
    BatchBuild(String),
    #[error("backtrace capture failed")]
    Backtrace(#[source] anyhow::Error),
    #[error(transparent)]
    Py(#[from] pyo3::PyErr),
    #[error(transparent)]
    Nul(#[from] std::ffi::NulError),
}

pub type TableResult<T> = std::result::Result<T, PythonTableError>;

impl From<pyo3::CastError<'_, '_>> for PythonTableError {
    fn from(err: pyo3::CastError<'_, '_>) -> Self {
        Self::Py(err.into())
    }
}

pub(crate) fn try_record_batch(
    schema: SchemaRef,
    columns: Vec<ArrayRef>,
) -> TableResult<RecordBatch> {
    RecordBatch::try_new(schema, columns).map_err(|e| PythonTableError::BatchBuild(e.to_string()))
}

/// Single-row batch surfacing a hard failure in SQL (`SELECT _error FROM python.<expr>`).
fn error_batch(message: &str) -> Vec<RecordBatch> {
    let schema = SchemaRef::new(Schema::new(vec![Field::new(
        "_error",
        DataType::Utf8,
        false,
    )]));
    match RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(StringArray::from(vec![message]))],
    ) {
        Ok(batch) => vec![batch],
        Err(e) => {
            error!("failed to build python namespace error batch: {e}");
            const FALLBACK: &str = "python namespace error";
            match RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(vec![FALLBACK]))]) {
                Ok(batch) => vec![batch],
                Err(e2) => {
                    error!("failed to build fallback error batch: {e2}");
                    vec![]
                }
            }
        }
    }
}

#[derive(Default, Debug)]
pub struct PythonNamespace {}

impl PythonNamespace {
    fn get_backtrace_data() -> TableResult<Vec<RecordBatch>> {
        let frames =
            crate::extensions::python::backtrace(None).map_err(PythonTableError::Backtrace)?;

        if frames.is_empty() {
            return Ok(vec![]);
        }

        let mut ips: Vec<Option<String>> = Vec::new();
        let mut files: Vec<Option<String>> = Vec::new();
        let mut funcs: Vec<Option<String>> = Vec::new();
        let mut linenos: Vec<Option<i64>> = Vec::new();
        let mut depth: Vec<Option<i64>> = Vec::new(); // Renamed from depths
        let mut frame_types: Vec<Option<String>> = Vec::new(); // Added for frame type
        let mut current_depth_val: i64 = 0; // Renamed from current_depth to avoid conflict if depth was a scalar

        for frame in frames {
            match frame {
                CallFrame::CFrame {
                    ip,
                    file,
                    func,
                    lineno,
                    lang,
                } => {
                    ips.push(Some(ip));
                    files.push(Some(file));
                    funcs.push(Some(func));
                    linenos.push(Some(lineno));
                    depth.push(Some(current_depth_val));
                    let type_label = match lang.as_deref() {
                        Some("rust") => "Rust",
                        Some("cpp") => "Native",
                        _ => "Native",
                    };
                    frame_types.push(Some(type_label.to_string()));
                    current_depth_val += 1;
                }
                CallFrame::PyFrame {
                    file,
                    func,
                    lineno,
                    locals: _,
                } => {
                    ips.push(None);
                    files.push(Some(file));
                    funcs.push(Some(func));
                    linenos.push(Some(lineno));
                    depth.push(Some(current_depth_val)); // Use new variable name
                    frame_types.push(Some("Python".to_string())); // Add frame type
                    current_depth_val += 1;
                }
            }
        }

        let schema = SchemaRef::new(Schema::new(vec![
            Field::new("ip", DataType::Utf8, true),
            Field::new("file", DataType::Utf8, true),
            Field::new("func", DataType::Utf8, true),
            Field::new("lineno", DataType::Int64, true),
            Field::new("depth", DataType::Int64, true),
            Field::new("frame_type", DataType::Utf8, true), // Added frame_type field
        ]));

        let columns: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(ips)),
            Arc::new(StringArray::from(files)),
            Arc::new(StringArray::from(funcs)),
            Arc::new(Int64Array::from(linenos)),
            Arc::new(Int64Array::from(depth)), // Use new variable name
            Arc::new(StringArray::from(frame_types)), // Added frame_type array
        ];

        Ok(vec![try_record_batch(schema, columns)?])
    }

    fn data_from_python(expr: &str) -> TableResult<Vec<RecordBatch>> {
        Python::attach(|py| {
            let import_path = expr.split(['(', '[']).next().unwrap_or(expr);

            let parts: Vec<&str> = import_path
                .split('.')
                .filter(|segment| !segment.is_empty())
                .collect();

            if parts.is_empty() {
                return Err(PythonTableError::InvalidExpression(expr.to_string()));
            }

            // Import the top-level package first.
            let pkg_name = parts[0];
            let pkg = py.import(pkg_name)?;

            // Set up locals dict with the imported package
            let locals = PyDict::new(py);
            locals.set_item(pkg_name, pkg)?;

            // Ensure intermediate submodules are imported so attribute access works.
            for depth in 2..=parts.len() {
                let candidate = parts[..depth].join(".");
                match py.import(&candidate) {
                    Ok(_) => {}
                    Err(err) => {
                        if depth < parts.len() {
                            return Err(err.into());
                        }
                        break;
                    }
                }
            }

            // Evaluate the expression
            let expr = CString::new(expr)?;

            let result = py.eval(&expr, None, Some(&locals))?;

            // Handle different Python types
            if let Ok(list) = result.cast::<PyList>() {
                return Self::list_to_recordbatch(list);
            }

            if let Ok(dict) = result.cast::<PyDict>() {
                return Self::dict_to_recordbatch(dict);
            }

            // Handle other Python objects
            Self::object_to_recordbatch(result)
        })
    }
}

impl CustomNamespace for PythonNamespace {
    fn name() -> &'static str {
        "python"
    }

    fn list() -> Vec<String> {
        // Extern tables (`probing.ExternalTable`) are mmap-backed and served
        // by the mmap SQL catalog (`probing_core::core::memtable_sql`), not
        // by this namespace.
        vec![
            "backtrace".to_string(),
            "profile_capture".to_string(),
            "profile_hotspot".to_string(),
        ]
    }

    fn data(expr: &str) -> Vec<RecordBatch> {
        if expr == "backtrace" {
            match Self::get_backtrace_data() {
                Ok(batches) => batches,
                Err(PythonTableError::Backtrace(ref source))
                    if crate::features::stacktrace::is_backtrace_busy(source) =>
                {
                    debug!("python.backtrace skipped: {source}");
                    error_batch("callstack capture busy")
                }
                Err(e) => {
                    error!("Error getting backtrace data: {e:?}");
                    error_batch(&e.to_string())
                }
            }
        } else if expr == "profile_capture" {
            match super::profile_sql::profile_capture_batches() {
                Ok(batches) => batches,
                Err(e) => {
                    error!("python.profile_capture: {e:?}");
                    error_batch(&e.to_string())
                }
            }
        } else if expr == "profile_hotspot" {
            match super::profile_sql::profile_hotspot_batches() {
                Ok(batches) => batches,
                Err(e) => {
                    error!("python.profile_hotspot: {e:?}");
                    error_batch(&e.to_string())
                }
            }
        } else if !expr.contains('.') {
            // Extern mmap tables (`comm_collective`, `torch_trace`, …) — not Python imports.
            debug!("python.{expr}: no live data (mmap empty or not created yet)");
            error_batch(&format!(
                "python.{expr}: no live data (mmap empty or not created yet)"
            ))
        } else {
            match Self::data_from_python(expr) {
                Ok(batches) => batches,
                Err(e) => {
                    error!("Python dynamic expr {expr}: {e:?}");
                    error_batch(&e.to_string())
                }
            }
        }
    }
}

impl PythonNamespace {
    pub fn object_to_recordbatch(obj: Bound<'_, PyAny>) -> TableResult<Vec<RecordBatch>> {
        let mut fields: Vec<Field> = vec![];
        let mut columns: Vec<ArrayRef> = vec![];

        if obj.is_instance_of::<PyDict>() {
            let item = obj.cast::<PyDict>()?;
            for (key, value) in item.iter() {
                let key_str = key.extract::<String>()?;
                Self::add_field_and_array(&mut fields, &mut columns, key_str, value)?;
            }
        } else if obj.hasattr("_asdict")? {
            // Handle namedtuple or any object with _asdict method
            let dict = obj.call_method0("_asdict")?;
            return Self::object_to_recordbatch(dict);
        } else {
            // Handle primitive types or fallback to string representation
            let field_name = "value";
            Self::add_field_and_array(&mut fields, &mut columns, field_name.to_string(), obj)?;
        }

        let schema = SchemaRef::new(Schema::new(fields));
        let batches = vec![try_record_batch(schema, columns)?];

        Ok(batches)
    }

    // Helper function to handle Python value conversion and add appropriate field
    fn add_field_and_array(
        fields: &mut Vec<Field>,
        columns: &mut Vec<ArrayRef>,
        name: String,
        value: Bound<'_, PyAny>,
    ) -> TableResult<()> {
        if value.is_instance_of::<PyInt>() {
            let array = Int64Array::from(vec![value.extract::<i64>()?]);
            columns.push(Arc::new(array));
            fields.push(Field::new(name, DataType::Int64, true));
        } else if value.is_instance_of::<PyFloat>() {
            let array = Float64Array::from(vec![value.extract::<f64>()?]);
            columns.push(Arc::new(array));
            fields.push(Field::new(name, DataType::Float64, true));
        } else if value.is_instance_of::<PyString>() {
            let array = StringArray::from(vec![value.extract::<String>()?]);
            columns.push(Arc::new(array));
            fields.push(Field::new(name, DataType::Utf8, true));
        } else {
            let array = StringArray::from(vec![value.to_string()]);
            columns.push(Arc::new(array));
            fields.push(Field::new(name, DataType::Utf8, true));
        }
        Ok(())
    }

    pub fn dict_to_recordbatch(dict: &Bound<'_, PyDict>) -> TableResult<Vec<RecordBatch>> {
        let mut fields: Vec<Field> = vec![];
        let mut columns: Vec<ArrayRef> = vec![];

        for (key, value) in dict.iter() {
            let key_str = key.extract::<String>()?;
            Self::add_field_and_array(&mut fields, &mut columns, key_str, value)?;
        }

        let schema = SchemaRef::new(Schema::new(fields));
        let batches = vec![try_record_batch(schema, columns)?];

        Ok(batches)
    }

    pub fn list_to_recordbatch(list: &Bound<'_, PyList>) -> TableResult<Vec<RecordBatch>> {
        let mut names: Vec<String> = vec![];
        let mut datas: HashMap<String, Vec<Option<Bound<'_, PyAny>>>> = Default::default();

        for (index, item) in list.try_iter()?.enumerate() {
            let item = item?;
            let item = if let Ok(dict) = item.cast::<PyDict>() {
                Some(dict.clone())
            } else {
                match item.getattr("__dict__") {
                    Ok(dict) => Some(dict.cast::<PyDict>()?.clone()),
                    Err(_) => {
                        let dict = PyDict::new(item.py());
                        dict.set_item("value", item)?;
                        Some(dict)
                    }
                }
            };
            if let Some(ref item) = item {
                for (key, _) in item.iter() {
                    let key_str = key.extract::<String>()?;
                    if !datas.contains_key(&key_str) {
                        names.push(key_str.clone());
                        let value = vec![None; index];
                        datas.insert(key_str.clone(), value);
                    }
                }
            }

            for k in names.iter() {
                if let Some(item) = &item {
                    match item.get_item(k) {
                        Ok(value) => {
                            datas.entry(k.clone()).and_modify(|v| v.push(value));
                        }
                        Err(_) => {
                            datas.entry(k.clone()).and_modify(|v| v.push(None));
                        }
                    }
                } else {
                    datas.entry(k.clone()).and_modify(|v| v.push(None));
                }
            }
        }

        let mut fields: Vec<Field> = vec![];
        let mut columns: Vec<ArrayRef> = vec![];

        for name in names.iter() {
            let values = datas.get(name).ok_or_else(|| {
                error!("list_to_recordbatch: internal column map missing key {name}");
                PythonTableError::MissingColumn(name.clone())
            })?;
            let array = StringArray::from(
                values
                    .iter()
                    .map(|x| {
                        if let Some(x) = x {
                            match x.extract::<String>() {
                                Ok(val) => Some(val),
                                Err(_) => Some(x.to_string()),
                            }
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>(),
            );
            columns.push(Arc::new(array));
            fields.push(Field::new(name, DataType::Utf8, true));
        }

        let schema = SchemaRef::new(Schema::new(fields));
        let batches = vec![try_record_batch(schema, columns)?];

        Ok(batches)
    }
}

pub type PythonProbeDataSource = NamespaceProbeDataSource<PythonNamespace>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_namespace_default() {
        let ns = PythonNamespace::default();
        // Just verify it can be created
        assert_eq!(format!("{:?}", ns), "PythonNamespace");
    }

    #[test]
    fn test_import_path_parsing() {
        // Test the import path parsing logic used in data_from_python
        let expr = "sys.path";
        let import_path = expr.split(['(', '[']).next().unwrap_or(expr);
        let parts: Vec<&str> = import_path
            .split('.')
            .filter(|segment| !segment.is_empty())
            .collect();

        assert_eq!(parts, vec!["sys", "path"]);

        // Test with function call
        let expr2 = "sys.path.append('test')";
        let import_path2 = expr2.split(['(', '[']).next().unwrap_or(expr2);
        let parts2: Vec<&str> = import_path2
            .split('.')
            .filter(|segment| !segment.is_empty())
            .collect();

        assert_eq!(parts2, vec!["sys", "path", "append"]);

        // Test with empty expression
        let expr3 = "";
        let import_path3 = expr3.split(['(', '[']).next().unwrap_or(expr3);
        let parts3: Vec<&str> = import_path3
            .split('.')
            .filter(|segment| !segment.is_empty())
            .collect();

        assert!(parts3.is_empty());
    }

    #[test]
    fn test_error_batch_surfaces_message_in_sql_column() {
        let batches = error_batch("backtrace capture failed");
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.schema().field(0).name(), "_error");
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("utf8 column");
        assert_eq!(col.value(0), "backtrace capture failed");
    }

    #[test]
    fn test_python_namespace_data_returns_error_batch_on_invalid_expr() {
        pyo3::Python::initialize();
        let batches = PythonNamespace::data("not_a_valid..expression");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].schema().field(0).name(), "_error");
    }
}
