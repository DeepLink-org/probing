//! Python-facing `ExternalTable`, backed by **mmap memtables**.
//!
//! Each table is an [`ExposedTable`] (MEMT ring buffer) under
//! `<PROBING_DATA_DIR>/<pid>/`:
//!
//! - ``foo`` → ``python.foo`` → SQL ``python.foo``
//! - ``nccl.proxy_ops`` → ``nccl.proxy_ops`` → SQL ``nccl.proxy_ops``
//! - the training process only ever pays the cost of an mmap row write —
//!   query-side materialisation happens in whoever runs the SQL.
//!
//! The first appended row fixes the column dtypes (the Python API only
//! declares column names). A leading `timestamp` column (microseconds since
//! epoch, `I64`) is always present, matching the previous TimeSeries layout.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::features::native_bridge::with_detached_native;
use once_cell::sync::Lazy;
use probing_core::sync::lock_mutex;
use probing_memtable::discover::ExposedTable;
use probing_memtable::docs;
use probing_memtable::{infer_extern_column_dtype, DType, Schema as MtSchema, Value};
use probing_proto::prelude::Ele;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};
use pyo3::{pyclass, pymethods, Bound, PyResult, Python};
use thiserror::Error;

use crate::features::convert::{ele_to_python, python_to_ele};

type PyTableRow = (Py<PyAny>, Vec<Py<PyAny>>);

#[derive(Debug, Error)]
enum ExternTableError {
    #[error("column count mismatch")]
    ColumnMismatch,
    #[error("table not initialized")]
    NotInitialized,
    #[error("push_row failed: schema mismatch or row too large")]
    PushFailed,
    #[error(transparent)]
    Memtable(#[from] probing_memtable::MemtableError),
}

/// SQL schema (and filename prefix) for Python extern tables.
pub const EXTERN_TABLE_SCHEMA: &str = "python";

/// Mmap filename for an extern table.
///
/// - ``foo`` → ``python.foo`` (legacy Python plugin tables)
/// - ``nccl.proxy_ops`` → ``nccl.proxy_ops`` (schema-qualified, matches memtable discovery)
fn mmap_basename(name: &str) -> String {
    if name.contains('.') {
        name.to_string()
    } else {
        format!("{EXTERN_TABLE_SCHEMA}.{name}")
    }
}

/// Legacy ``python.*`` tables prepend a ``timestamp`` column; schema-qualified
/// tables (``nccl.proxy_ops``) match native writer layouts exactly.
fn uses_timestamp_column(name: &str) -> bool {
    !name.contains('.')
}

fn build_schema_with_docs(
    name: &str,
    columns: &[String],
    dtypes: &[DType],
    table_doc: Option<&str>,
    column_docs: &HashMap<String, String>,
) -> MtSchema {
    let mut schema = MtSchema::new();
    if let Some(doc) = table_doc {
        schema = schema.table_doc(doc);
    }
    if uses_timestamp_column(name) {
        schema = schema.col("timestamp", DType::I64);
    }
    for (col, dt) in columns.iter().zip(dtypes.iter()) {
        schema = if let Some(doc) = column_docs.get(col) {
            schema.col_doc(col, *dt, doc.as_str())
        } else {
            schema.col(col, *dt)
        };
    }
    schema
}

fn register_python_table_docs(
    name: &str,
    table_doc: Option<&str>,
    column_docs: &HashMap<String, String>,
) {
    let (table_schema, table_name) = if let Some((schema, table)) = name.split_once('.') {
        (schema.to_string(), table.to_string())
    } else {
        (EXTERN_TABLE_SCHEMA.to_string(), name.to_string())
    };
    let pairs: Vec<(String, String)> = column_docs
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    docs::register_column_docs(&table_schema, &table_name, table_doc, &pairs);
}

/// Ring layout: fixed chunk count; chunk byte size derives from capacity.
const NUM_CHUNKS: u32 = 8;
const MIN_CHUNK_BYTES: usize = 4 * 1024;
const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

fn value_to_object(py: Python, v: &Ele) -> Py<PyAny> {
    ele_to_python(py, v).unwrap_or_else(|e| {
        log::warn!("ExternalTable: failed to convert cell to Python ({e})");
        py.None()
    })
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

#[pyclass(from_py_object)]
#[derive(Clone)]
pub struct PyExternalTableConfig {
    #[pyo3(get)]
    chunk_size: usize,
    #[pyo3(get)]
    discard_threshold: usize,
    #[pyo3(get)]
    discard_strategy: String,
}

impl Default for PyExternalTableConfig {
    fn default() -> Self {
        PyExternalTableConfig {
            chunk_size: 10000,
            discard_threshold: default_discard_threshold_bytes(),
            discard_strategy: "BaseMemorySize".to_string(),
        }
    }
}

#[pymethods]
impl PyExternalTableConfig {
    #[new]
    fn new(chunk_size: usize, discard_threshold: usize, discard_strategy: String) -> Self {
        PyExternalTableConfig {
            chunk_size,
            discard_threshold,
            discard_strategy,
        }
    }

    #[allow(clippy::wrong_self_convention)] // Python-facing method name, kept for API compat
    fn into_py(&self, py: Python<'_>) -> Py<PyAny> {
        let dict = PyDict::new(py);
        if let Err(e) = dict.set_item("chunk_size", self.chunk_size) {
            log::error!("PyExternalTableConfig::into_py chunk_size: {e}");
        }
        if let Err(e) = dict.set_item("discard_threshold", self.discard_threshold) {
            log::error!("PyExternalTableConfig::into_py discard_threshold: {e}");
        }
        if let Err(e) = dict.set_item("discard_strategy", &self.discard_strategy) {
            log::error!("PyExternalTableConfig::into_py discard_strategy: {e}");
        }
        dict.into()
    }
}

const DEFAULT_DISCARD_THRESHOLD_BYTES: usize = 20 * 1024 * 1024;

fn default_discard_threshold_bytes() -> usize {
    static DEFAULT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *DEFAULT.get_or_init(|| {
        std::env::var("PROBING_TABLE_DEFAULT_MB")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|mb| *mb > 0)
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(DEFAULT_DISCARD_THRESHOLD_BYTES)
    })
}

fn resolve_discard_threshold(discard_threshold: Option<usize>) -> usize {
    discard_threshold.unwrap_or_else(default_discard_threshold_bytes)
}
/// Total ring capacity in bytes derived from the (legacy) discard config.
///
/// - `BaseMemorySize`: `discard_threshold` *is* a byte budget.
/// - `BaseElementCount`: estimate 64 bytes/row.
/// - anything else: 16 MiB default.
fn ring_capacity_bytes(discard_threshold: usize, strategy: &str) -> usize {
    let raw = match strategy {
        "BaseMemorySize" => discard_threshold,
        "BaseElementCount" => discard_threshold.saturating_mul(64),
        _ => 16 * 1024 * 1024,
    };
    raw.clamp(MIN_CHUNK_BYTES * NUM_CHUNKS as usize, 1 << 30)
}

fn ring_chunk_bytes(capacity: usize) -> u32 {
    (capacity / NUM_CHUNKS as usize).clamp(MIN_CHUNK_BYTES, MAX_CHUNK_BYTES) as u32
}

/// Column dtype inferred from the first appended value.
fn ele_dtype(e: &Ele) -> DType {
    match e {
        Ele::I32(_) => DType::I32,
        Ele::I64(_) => DType::I64,
        Ele::F32(_) => DType::F32,
        Ele::F64(_) => DType::F64,
        Ele::BOOL(_) => DType::U8,
        Ele::DataTime(_) => DType::U64,
        Ele::Text(_) | Ele::Url(_) | Ele::Nil => DType::Str,
    }
}

/// Owned cell value: coerced from an [`Ele`] to match the column dtype, so a
/// `Vec<Value>` row can borrow from it.
enum OwnedVal {
    U8(u8),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    U64(u64),
    S(String),
}

fn ele_to_owned(e: &Ele, dt: DType) -> OwnedVal {
    let as_f64 = |e: &Ele| match e {
        Ele::I32(v) => *v as f64,
        Ele::I64(v) => *v as f64,
        Ele::F32(v) => *v as f64,
        Ele::F64(v) => *v,
        Ele::BOOL(v) => *v as u8 as f64,
        Ele::DataTime(v) => *v as f64,
        _ => 0.0,
    };
    match dt {
        DType::U8 => OwnedVal::U8(match e {
            Ele::BOOL(v) => *v as u8,
            other => as_f64(other) as u8,
        }),
        DType::I32 => OwnedVal::I32(as_f64(e) as i32),
        DType::I64 => OwnedVal::I64(as_f64(e) as i64),
        DType::F32 => OwnedVal::F32(as_f64(e) as f32),
        DType::F64 => OwnedVal::F64(as_f64(e)),
        DType::U64 => OwnedVal::U64(as_f64(e) as u64),
        DType::U32 => OwnedVal::U64(as_f64(e) as u64),
        DType::Str | DType::Bytes => OwnedVal::S(match e {
            Ele::Text(s) | Ele::Url(s) => s.clone(),
            Ele::Nil => String::new(),
            other => other.to_string(),
        }),
    }
}

fn owned_to_value(o: &OwnedVal) -> Value<'_> {
    match o {
        OwnedVal::U8(v) => Value::U8(*v),
        OwnedVal::I32(v) => Value::I32(*v),
        OwnedVal::I64(v) => Value::I64(*v),
        OwnedVal::F32(v) => Value::F32(*v),
        OwnedVal::F64(v) => Value::F64(*v),
        OwnedVal::U64(v) => Value::U64(*v),
        OwnedVal::S(s) => Value::Str(s),
    }
}

/// State behind one extern table. The mmap ring is created lazily on the
/// first append because the Python API declares names but not types.
pub struct ExternBacking {
    name: String,
    columns: Vec<String>,
    capacity_bytes: usize,
    dtypes: Vec<DType>,
    table: Option<ExposedTable>,
    table_doc: Option<String>,
    column_docs: HashMap<String, String>,
}

impl ExternBacking {
    fn new(
        name: &str,
        columns: Vec<String>,
        capacity_bytes: usize,
        table_doc: Option<String>,
        column_docs: HashMap<String, String>,
    ) -> Self {
        if !column_docs.is_empty() || table_doc.is_some() {
            register_python_table_docs(name, table_doc.as_deref(), &column_docs);
        }
        Self {
            name: name.to_string(),
            columns,
            capacity_bytes,
            dtypes: vec![],
            table: None,
            table_doc,
            column_docs,
        }
    }

    fn ensure_registered(&mut self) -> Result<(), ExternTableError> {
        if self.table.is_some() {
            return Ok(());
        }
        self.dtypes = self
            .columns
            .iter()
            .map(|col| infer_extern_column_dtype(col))
            .collect();
        let schema = build_schema_with_docs(
            &self.name,
            &self.columns,
            &self.dtypes,
            self.table_doc.as_deref(),
            &self.column_docs,
        );
        let chunk_bytes = ring_chunk_bytes(self.capacity_bytes);
        let filename = mmap_basename(&self.name);
        let table = ExposedTable::create(&filename, &schema, chunk_bytes, NUM_CHUNKS)?;
        self.table = Some(table);
        Ok(())
    }

    fn ensure_table(&mut self, first_row: &[Ele]) -> Result<(), ExternTableError> {
        let dtypes: Vec<DType> = first_row.iter().map(ele_dtype).collect();
        if self.table.is_some() && self.dtypes == dtypes {
            return Ok(());
        }
        self.table = None;
        self.dtypes.clear();

        let dtypes: Vec<DType> = first_row.iter().map(ele_dtype).collect();
        let schema = build_schema_with_docs(
            &self.name,
            &self.columns,
            &dtypes,
            self.table_doc.as_deref(),
            &self.column_docs,
        );
        let chunk_bytes = ring_chunk_bytes(self.capacity_bytes);
        let filename = mmap_basename(&self.name);
        let table = ExposedTable::create(&filename, &schema, chunk_bytes, NUM_CHUNKS)?;
        self.dtypes = dtypes;
        self.table = Some(table);
        Ok(())
    }

    fn append(&mut self, timestamp: i64, values: &[Ele]) -> Result<(), ExternTableError> {
        if values.len() != self.columns.len() {
            return Err(ExternTableError::ColumnMismatch);
        }
        self.ensure_table(values)?;

        let owned: Vec<OwnedVal> = values
            .iter()
            .zip(self.dtypes.iter())
            .map(|(e, dt)| ele_to_owned(e, *dt))
            .collect();
        let mut row: Vec<Value> = Vec::with_capacity(owned.len() + 1);
        if uses_timestamp_column(&self.name) {
            row.push(Value::I64(timestamp));
        }
        row.extend(owned.iter().map(owned_to_value));

        // ExposedTable::push_row validates schema and auto-advances chunks.
        let Some(table) = self.table.as_mut() else {
            return Err(ExternTableError::NotInitialized);
        };
        if !table.push_row(&row) {
            return Err(ExternTableError::PushFailed);
        }
        Ok(())
    }

    fn read_row_values(&self, cursor: &mut probing_memtable::RowCursor<'_>) -> Vec<Ele> {
        self.dtypes
            .iter()
            .map(|dt| match dt {
                DType::U8 => Ele::BOOL(cursor.next_u8() != 0),
                DType::I32 => Ele::I32(cursor.next_i32()),
                DType::I64 => Ele::I64(cursor.next_i64()),
                DType::F32 => Ele::F32(cursor.next_f32()),
                DType::F64 => Ele::F64(cursor.next_f64()),
                DType::U64 => Ele::DataTime(cursor.next_u64()),
                DType::U32 => Ele::I64(cursor.next_u32() as i64),
                DType::Str => Ele::Text(cursor.next_str().to_string()),
                DType::Bytes => Ele::Text(String::from_utf8_lossy(cursor.next_bytes()).to_string()),
            })
            .collect()
    }

    /// Rows in chronological order; when `limit` is set, only the most
    /// recent `limit` rows are returned (still oldest → newest).
    fn take(&self, limit: Option<usize>) -> Vec<(Ele, Vec<Ele>)> {
        let Some(table) = &self.table else {
            return vec![];
        };
        let view = table.view();
        let mut out: Vec<(Ele, Vec<Ele>)> = Vec::new();
        for chunk in view.chunks_logical() {
            for row in view.rows(chunk) {
                let mut cursor = row.cursor();
                let (ts, vals) = if uses_timestamp_column(&self.name) {
                    let ts = Ele::I64(cursor.next_i64());
                    let vals = self.read_row_values(&mut cursor);
                    (ts, vals)
                } else {
                    let vals = self.read_row_values(&mut cursor);
                    let ts = vals.first().cloned().unwrap_or(Ele::Nil);
                    (ts, vals)
                };
                out.push((ts, vals));
            }
        }
        if let Some(limit) = limit {
            if out.len() > limit {
                out.drain(..out.len() - limit);
            }
        }
        out
    }
}

impl std::fmt::Debug for ExternBacking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternBacking")
            .field("name", &self.name)
            .field("columns", &self.columns)
            .field("created", &self.table.is_some())
            .finish()
    }
}

pub static EXTERN_TABLES: Lazy<Mutex<HashMap<String, Arc<Mutex<ExternBacking>>>>> =
    Lazy::new(|| Mutex::new(Default::default()));

fn lock_extern_tables() -> MutexGuard<'static, HashMap<String, Arc<Mutex<ExternBacking>>>> {
    lock_mutex(&EXTERN_TABLES, "EXTERN_TABLES")
}

fn lock_backing(backing: &Mutex<ExternBacking>) -> MutexGuard<'_, ExternBacking> {
    lock_mutex(backing, "ExternBacking")
}

#[pyclass(from_py_object)]
#[derive(Clone, Debug)]
pub struct ExternalTable(Option<Arc<Mutex<ExternBacking>>>, usize);

const BRIDGE_DEGRADED_MSG: &str =
    "probing native bridge unavailable; external table operations are disabled";

fn require_backing(
    backing: &Option<Arc<Mutex<ExternBacking>>>,
) -> PyResult<&Arc<Mutex<ExternBacking>>> {
    backing
        .as_ref()
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err(BRIDGE_DEGRADED_MSG))
}

impl ExternalTable {
    fn extract_eles(values: Vec<Py<PyAny>>) -> PyResult<Vec<Ele>> {
        Python::attach(|py| {
            values
                .into_iter()
                .map(|v| python_to_ele(v.bind(py)))
                .collect()
        })
    }

    fn create_backing(
        name: &str,
        columns: Vec<String>,
        discard_threshold: usize,
        discard_strategy: &str,
        table_doc: Option<String>,
        column_docs: HashMap<String, String>,
    ) -> PyResult<Arc<Mutex<ExternBacking>>> {
        let capacity = ring_capacity_bytes(discard_threshold, discard_strategy);
        let backing = Arc::new(Mutex::new(ExternBacking::new(
            name,
            columns,
            capacity,
            table_doc,
            column_docs,
        )));
        lock_backing(backing.as_ref())
            .ensure_registered()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        Ok(backing)
    }
}

#[pymethods]
impl ExternalTable {
    #[new]
    #[pyo3(signature = (name, columns, chunk_size = 10000, discard_threshold = None, discard_strategy = "BaseMemorySize".to_string(), table_doc = None, column_docs = None))]
    fn new(
        name: &str,
        columns: Vec<String>,
        chunk_size: usize,
        discard_threshold: Option<usize>,
        discard_strategy: String,
        table_doc: Option<String>,
        column_docs: Option<HashMap<String, String>>,
    ) -> PyResult<Self> {
        let _ = chunk_size; // ring chunking is byte-based; kept for API compat
        let discard_threshold = resolve_discard_threshold(discard_threshold);
        let name = name.to_string();
        with_detached_native(move || {
            let ncolumn = columns.len();
            let backing = Self::create_backing(
                &name,
                columns,
                discard_threshold,
                &discard_strategy,
                table_doc,
                column_docs.unwrap_or_default(),
            )?;
            lock_extern_tables().insert(name, backing.clone());
            Ok(ExternalTable(Some(backing), ncolumn))
        })
    }

    #[classmethod]
    fn get(_cls: &Bound<'_, PyType>, name: &str) -> PyResult<ExternalTable> {
        let name = name.to_string();
        with_detached_native(move || {
            let binding = lock_extern_tables();
            if let Some(backing) = binding.get(&name) {
                let ncolumn = lock_backing(backing.as_ref()).columns.len();
                Ok(ExternalTable(Some(backing.clone()), ncolumn))
            } else {
                Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "table {name} not found"
                )))
            }
        })
    }

    #[classmethod]
    #[pyo3(signature = (name, columns, chunk_size = 10000, discard_threshold = None, discard_strategy = "BaseMemorySize".to_string(), table_doc = None, column_docs = None))]
    #[allow(clippy::too_many_arguments)]
    fn get_or_create(
        _cls: &Bound<'_, PyType>,
        name: &str,
        columns: Vec<String>,
        chunk_size: usize,
        discard_threshold: Option<usize>,
        discard_strategy: String,
        table_doc: Option<String>,
        column_docs: Option<HashMap<String, String>>,
    ) -> PyResult<ExternalTable> {
        let _ = chunk_size;
        let discard_threshold = resolve_discard_threshold(discard_threshold);
        let name = name.to_string();
        with_detached_native(move || {
            let mut binding = lock_extern_tables();
            if let Some(backing) = binding.get(&name) {
                let ncolumn = lock_backing(backing.as_ref()).columns.len();
                Ok(ExternalTable(Some(backing.clone()), ncolumn))
            } else {
                let ncolumn = columns.len();
                let backing = Self::create_backing(
                    &name,
                    columns,
                    discard_threshold,
                    &discard_strategy,
                    table_doc,
                    column_docs.unwrap_or_default(),
                )?;
                binding.insert(name, backing.clone());
                Ok(ExternalTable(Some(backing), ncolumn))
            }
        })
    }

    #[classmethod]
    fn drop(_cls: &Bound<'_, PyType>, name: &str) -> PyResult<()> {
        let name = name.to_string();
        with_detached_native(move || {
            let _ = lock_extern_tables().remove(&name);
            Ok(())
        })
    }

    fn names(&self) -> PyResult<Vec<String>> {
        let backing = require_backing(&self.0)?.clone();
        with_detached_native(move || Ok(lock_backing(backing.as_ref()).columns.clone()))
    }

    fn append(&mut self, values: Vec<Py<PyAny>>) -> PyResult<()> {
        if values.len() != self.1 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "column count mismatch",
            ));
        }
        let eles = Self::extract_eles(values)?;
        let backing = require_backing(&self.0)?.clone();
        with_detached_native(move || {
            lock_backing(backing.as_ref())
                .append(now_micros(), &eles)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
        })
    }

    fn append_ts(&mut self, t: i64, values: Vec<Py<PyAny>>) -> PyResult<()> {
        if values.len() != self.1 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "column count mismatch",
            ));
        }
        let eles = Self::extract_eles(values)?;
        let backing = require_backing(&self.0)?.clone();
        with_detached_native(move || {
            lock_backing(backing.as_ref())
                .append(t, &eles)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
        })
    }

    fn append_many(&mut self, rows: Vec<Vec<Py<PyAny>>>) -> PyResult<()> {
        for row in rows {
            self.append(row)?;
        }
        Ok(())
    }

    #[pyo3(signature = (limit=None))]
    fn take(&self, limit: Option<usize>) -> PyResult<Vec<PyTableRow>> {
        let backing = require_backing(&self.0)?.clone();
        with_detached_native(move || {
            let rows = lock_backing(backing.as_ref()).take(limit);
            let result = rows
                .iter()
                .map(|(t, vals)| {
                    Python::attach(|py| {
                        let t = value_to_object(py, t);
                        let vals = vals
                            .iter()
                            .map(|v| value_to_object(py, v))
                            .collect::<Vec<_>>();
                        (t, vals)
                    })
                })
                .collect();
            Ok(result)
        })
    }
}

/// Register table/column documentation for SQL `DESCRIBE` (without creating a table).
#[pyfunction]
#[pyo3(signature = (qualified_name, table_doc=None, column_docs=None))]
pub fn register_table_docs(
    qualified_name: &str,
    table_doc: Option<&str>,
    column_docs: Option<HashMap<String, String>>,
) -> PyResult<()> {
    register_python_table_docs(qualified_name, table_doc, &column_docs.unwrap_or_default());
    Ok(())
}

#[cfg(test)]
mod register_docs_tests {
    use super::*;
    use probing_memtable::docs;

    #[test]
    fn register_table_docs_exposes_python_schema() {
        let table = format!("py_doc_test_{}", std::process::id());
        let qualified = format!("python.{table}");
        let mut column_docs = HashMap::new();
        column_docs.insert("latency_ms".to_string(), "latency in ms".to_string());
        register_table_docs(&qualified, Some("Python doc test table"), Some(column_docs)).unwrap();
        let rows = docs::snapshot();
        let row = rows
            .iter()
            .find(|r| r.table_schema == "python" && r.table_name == table)
            .expect("python table docs");
        assert_eq!(row.description.as_deref(), Some("Python doc test table"));
        assert_eq!(
            row.columns.get("latency_ms"),
            Some(&"latency in ms".to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::python::PythonProbeDataSource;
    use probing_core::core::{Engine, UnifiedMemtableProbeDataSource};
    use pyo3::ffi::c_str;

    /// Route all mmap files of this test process into one tempdir.
    static TEST_DATA_DIR: Lazy<tempfile::TempDir> = Lazy::new(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("PROBING_DATA_DIR", dir.path());
        dir
    });

    fn setup() {
        let _ = &*TEST_DATA_DIR;
        pyo3::Python::initialize();
        Python::attach(|py| {
            use pyo3::types::PyModule;
            use pyo3::PyTypeInfo;

            let sys = PyModule::import(py, "sys").unwrap();
            let modules = sys.getattr("modules").unwrap();

            let probing = if modules.contains("probing").unwrap_or(false) {
                PyModule::import(py, "probing").unwrap()
            } else {
                let m = PyModule::new(py, "probing").unwrap();
                modules.set_item("probing", &m).unwrap();
                m
            };

            if !probing.hasattr("ExternalTable").unwrap_or(false) {
                probing
                    .setattr("ExternalTable", ExternalTable::type_object(py))
                    .unwrap();
            }
        });
    }

    /// Create a table with a unique name and three rows; idempotent per name.
    fn setup_table(name: &str) {
        setup();
        Python::attach(|py| {
            py.run(
                &std::ffi::CString::new(format!(
                    r#"
import probing
if not hasattr(probing, "_made_{name}"):
    t = probing.ExternalTable.get_or_create("{name}", ["a", "b"])
    t.append([1, 2])
    t.append([3, 4])
    t.append([5, 6])
    probing._made_{name} = True
"#
                ))
                .unwrap(),
                None,
                None,
            )
            .unwrap();
        });
    }

    async fn engine_with_python() -> Engine {
        Engine::builder()
            .with_default_namespace("probe")
            .with_data_source(PythonProbeDataSource::create("python"))
            .with_data_source(Arc::new(UnifiedMemtableProbeDataSource))
            .build()
            .await
            .unwrap()
    }

    #[test]
    fn test_create_new_table() {
        setup();
        let table = ExternalTable::new(
            "table1",
            vec!["a".to_string(), "b".to_string()],
            10000,
            Some(20000000),
            "BaseMemorySize".to_string(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(table.names().unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn ensure_registered_infers_numeric_dtypes() {
        setup();
        let name = format!("comm_like_{}", std::process::id());
        let _table = ExternalTable::new(
            &name,
            vec![
                "rank".to_string(),
                "duration_ms".to_string(),
                "op".to_string(),
            ],
            10000,
            Some(20_000_000),
            "BaseMemorySize".to_string(),
            None,
            None,
        )
        .unwrap();
        let binding = lock_extern_tables();
        let backing = binding.get(&name).expect("backing");
        let guard = lock_backing(backing.as_ref());
        assert_eq!(
            guard.dtypes,
            vec![DType::I64, DType::F64, DType::Str],
            "placeholder mmap must not default all columns to Str"
        );
    }

    #[test]
    fn test_create_table_in_python() {
        setup();
        Python::attach(|py| {
            py.run(
                c_str!(
                    r#"
import probing
table = probing.ExternalTable.get_or_create("table2", ["a", "b"])
"#
                ),
                None,
                None,
            )
            .unwrap();
            let binding = EXTERN_TABLES.lock().unwrap();
            assert!(binding.contains_key("table2"));
        });
    }

    #[test]
    fn test_drop_table_in_python() {
        setup();
        Python::attach(|py| {
            py.run(
                c_str!(
                    r#"
import probing
probing.ExternalTable.get_or_create("table_to_drop", ["a", "b"])
probing.ExternalTable.drop("table_to_drop")
                    "#
                ),
                None,
                None,
            )
            .unwrap();
            let binding = EXTERN_TABLES.lock().unwrap();
            assert!(!binding.contains_key("table_to_drop"));
        });
    }

    #[test]
    fn test_append_take_roundtrip_and_mmap_file() {
        setup();
        let mut table = ExternalTable::new(
            "roundtrip",
            vec!["x".to_string(), "msg".to_string()],
            10000,
            Some(1_000_000),
            "BaseMemorySize".to_string(),
            None,
            None,
        )
        .unwrap();
        Python::attach(|py| {
            let vals: Vec<Py<PyAny>> = vec![
                1i64.into_pyobject(py).unwrap().into_any().unbind(),
                "hello".into_pyobject(py).unwrap().into_any().unbind(),
            ];
            table.append(vals).unwrap();
            let vals: Vec<Py<PyAny>> = vec![
                2i64.into_pyobject(py).unwrap().into_any().unbind(),
                "world".into_pyobject(py).unwrap().into_any().unbind(),
            ];
            table.append(vals).unwrap();
        });

        // mmap file exists on disk under <data_dir>/<pid>/python.roundtrip
        let path = probing_memtable::discover::default_dir()
            .join(std::process::id().to_string())
            .join("python.roundtrip");
        assert!(path.is_file(), "mmap file missing: {path:?}");

        // Qualified schema.table → mmap basename used as-is
        let mut nccl = ExternalTable::new(
            "nccl.proxy_ops",
            vec!["rank".to_string()],
            10000,
            Some(1_000_000),
            "BaseMemorySize".to_string(),
            None,
            None,
        )
        .unwrap();
        Python::attach(|py| {
            let vals: Vec<Py<PyAny>> = vec![1i64.into_pyobject(py).unwrap().into_any().unbind()];
            nccl.append(vals).unwrap();
        });
        let nccl_path = probing_memtable::discover::default_dir()
            .join(std::process::id().to_string())
            .join("nccl.proxy_ops");
        assert!(nccl_path.is_file(), "mmap file missing: {nccl_path:?}");

        // take() returns rows oldest → newest, with coerced values
        let rows = table.take(None).unwrap();
        assert_eq!(rows.len(), 2);
        Python::attach(|py| {
            let (_, vals) = &rows[0];
            assert_eq!(vals[0].extract::<i64>(py).unwrap(), 1);
            assert_eq!(vals[1].extract::<String>(py).unwrap(), "hello");
            let (_, vals) = &rows[1];
            assert_eq!(vals[0].extract::<i64>(py).unwrap(), 2);
            assert_eq!(vals[1].extract::<String>(py).unwrap(), "world");
        });

        // take(limit) keeps the most recent rows
        let rows = table.take(Some(1)).unwrap();
        assert_eq!(rows.len(), 1);
        Python::attach(|py| {
            assert_eq!(rows[0].1[1].extract::<String>(py).unwrap(), "world");
        });
    }

    #[test]
    fn test_see_py_table_data_in_engine() {
        setup_table("table4");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        let engine = rt.block_on(engine_with_python());
        let tables = rt.block_on(async {
            engine
                .async_query("select * from python.table4 ")
                .await
                .unwrap()
        });
        let df = tables.expect("Table 'table4' should be queryable");
        assert_eq!(df.len(), 3, "Should have 3 rows");
        // timestamp + a + b
        assert_eq!(df.names.len(), 3, "Should have 3 columns: {:?}", df.names);
        assert_eq!(df.names[0], "timestamp");
    }

    #[test]
    fn test_calculate_in_sql_with_filter() {
        setup_table("table5");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        let engine = rt.block_on(engine_with_python());
        let tables = rt.block_on(async {
            engine
                .async_query("select a + b as c from python.table5 where a > 1")
                .await
                .unwrap()
        });
        let df = tables.expect("Query should return results");
        assert_eq!(df.len(), 2, "Should have 2 rows where a > 1");
    }

    #[test]
    fn test_aggregate_in_sql() {
        setup_table("table6");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        let engine = rt.block_on(engine_with_python());
        let tables = rt.block_on(async {
            engine
                .async_query("select sum(a), sum(b) from python.table6")
                .await
                .unwrap()
        });
        let df = tables.expect("Aggregation query should return results");
        assert!(!df.cols.is_empty(), "Should have aggregation results");
    }

    #[test]
    fn test_static_python_tables_not_shadowed() {
        // Extern mmap tables under schema `python` must not hide the static
        // namespace (backtrace, expression tables) — the merged catalog
        // resolves mmap first, then falls through to the inner provider.
        setup_table("table7");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let engine = rt.block_on(engine_with_python());
        // `python.\`time.time()\`` is served by the static namespace's
        // expression path; it must still resolve with extern tables present.
        let result = rt.block_on(async {
            engine
                .async_query("select * from python.`time.time()`")
                .await
        });
        assert!(
            result.is_ok(),
            "static python namespace shadowed: {result:?}"
        );
    }
}
