//! Mmap memtable ↔ SQL catalog integration.
//!
//! Exposes mmap'd memtable files (MEMT rings / MEMH hash tables) under
//! `<data_dir>/<pid>/` as DataFusion tables. Shared by the server and the
//! language extensions so that every data producer writes through
//! `probing-memtable` and every consumer queries through this module.
//!
//! ## File → SQL mapping (no hard-coded product prefix)
//!
//! - **First `.` splits schema vs table** — `acme.actors` → schema `acme`, table `actors`;
//!   `foo.bar.baz` → schema `foo`, table `bar.baz` (on-disk name is the full filename).
//! - **No `.`** — exposed as `memtable.<filename>` (e.g. `metrics` → `memtable.metrics`).
//!
//! Schema head and table tail must be non-empty; only ASCII letters, digits, `_`, and
//! `.` inside the table tail are allowed (no `/`, `\\`). Leading-dot names are ignored.
//!
//! ## Read semantics (ring tables)
//!
//! - Files are **mmap'd read-only** (no full-file heap copy); only touched
//!   pages are faulted in.
//! - Chunks are materialised in **logical (oldest → newest) write order**
//!   via [`MemTableView::chunks_logical`], one Arrow `RecordBatch` per chunk.
//! - Each chunk's `generation` is re-checked after reading: a chunk recycled
//!   by the writer mid-read is **discarded** instead of surfacing torn rows.
//! - When the table has a designated timestamp column, chunks whose
//!   `[min_ts, max_ts]` range cannot satisfy the query's time predicates are
//!   **pruned** before materialisation ([`RingMmapTable`]).

use std::any::Any;
use std::collections::{BTreeSet, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use datafusion::arrow::array::{
    ArrayRef, BinaryArray, BinaryBuilder, Float32Array, Float32Builder, Float64Array,
    Float64Builder, GenericStringBuilder, Int32Array, Int32Builder, Int64Array, Int64Builder,
    RecordBatch, StringArray, UInt32Array, UInt32Builder, UInt64Array, UInt64Builder, UInt8Array,
    UInt8Builder,
};
use datafusion::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use datafusion::catalog::CatalogProvider;
use datafusion::catalog::SchemaProvider;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::DataFusionError;
use datafusion::error::Result as DfResult;
use datafusion::logical_expr::{Expr, Operator, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;
use datafusion::scalar::ScalarValue;

use probing_memtable::discover::{default_dir, MappedFile};
use probing_memtable::memc::{ColdStats, ColdStore, ColumnData, Compactor, CompactorConfig, SegmentReader};
use probing_memtable::{detect_table, DType, MemTableView, MemhView, TableKind, TypedValue};

use super::plugin_advanced::{scan_memory_partitions, supports_filters_pushdown_for_schema};
use super::{
    EngineCall, EngineDatasource, EngineError, EngineExtension, EngineExtensionOption, Maybe,
    Plugin, PluginAdvancedTable, PluginType,
};
use probing_macros::EngineExtension as EngineExtensionDerive;

/// SQL schema used for mmap files whose basename contains no `.`.
pub const DEFAULT_UNDOTTED_SCHEMA: &str = "memtable";

fn self_dir() -> std::path::PathBuf {
    default_dir().join(std::process::id().to_string())
}

/// Cold-segment directory for this process: `<data_dir>/<pid>/cold`.
///
/// Co-located with (and scoped like) the hot ring files so cold data never
/// mixes across processes, and the compactor writer and this read path agree
/// on one location without extra configuration.
pub fn cold_dir() -> std::path::PathBuf {
    self_dir().join("cold")
}

#[inline]
fn valid_schema_head(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[inline]
fn valid_table_tail(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains('\\')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
}

/// Map basename `filename` → `(schema, table)` for routing; [`None`] if skipped.
pub fn classify_mmap_basename(filename: &str) -> Option<(String, String)> {
    if filename.starts_with('.') {
        return None;
    }
    if let Some((head, tail)) = filename.split_once('.') {
        if valid_schema_head(head) && valid_table_tail(tail) {
            return Some((head.to_string(), tail.to_string()));
        }
        return None;
    }
    if valid_schema_head(filename) {
        Some((DEFAULT_UNDOTTED_SCHEMA.to_string(), filename.to_string()))
    } else {
        None
    }
}

/// On-disk filename for a `(schema, table)` pair.
pub fn mmap_filename_for(schema: &str, table: &str) -> String {
    if schema == DEFAULT_UNDOTTED_SCHEMA {
        table.to_string()
    } else {
        format!("{schema}.{table}")
    }
}

fn tables_in_schema(target_schema: &str) -> Vec<String> {
    let dir = self_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        if !e.path().is_file() {
            continue;
        }
        let n = e.file_name().to_string_lossy().to_string();
        if let Some((sch, tbl)) = classify_mmap_basename(&n) {
            if sch == target_schema {
                out.push(tbl);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn discover_all_schemas() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let dir = self_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if !e.path().is_file() {
                continue;
            }
            let n = e.file_name().to_string_lossy().to_string();
            if let Some((sch, _)) = classify_mmap_basename(&n) {
                out.insert(sch);
            }
        }
    }
    out.insert(DEFAULT_UNDOTTED_SCHEMA.to_string());
    out
}

/// Whether an mmap file backs `schema.table` right now (validates the table
/// name first so user-supplied SQL identifiers can never escape the data dir).
fn mmap_table_exists(schema: &str, table: &str) -> bool {
    if !valid_table_tail(table) {
        return false;
    }
    self_dir().join(mmap_filename_for(schema, table)).is_file()
}

/// Mmap ring / MEMH → Arrow batches, then a [`PluginAdvancedTable`] so DataFusion can push
/// filters and limits into the scan path.
pub fn bytes_to_pushdown_table(data: &[u8], logical_name: &str) -> Arc<dyn TableProvider> {
    match detect_table(data) {
        Some(TableKind::Ring) => {
            let view = match MemTableView::new(data) {
                Ok(v) => v,
                Err(_) => return Arc::new(PluginAdvancedTable::empty_sentinel(logical_name)),
            };
            let schema = view_to_arrow_schema(&view);
            let batches = view_to_recordbatches(&view);
            match PluginAdvancedTable::try_new(logical_name, schema, batches) {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    log::error!("memtable PluginAdvancedTable (ring): {e}");
                    Arc::new(PluginAdvancedTable::empty_sentinel(logical_name))
                }
            }
        }
        Some(TableKind::Hash) => {
            let view = match MemhView::new(data) {
                Ok(v) => v,
                Err(_) => return Arc::new(PluginAdvancedTable::empty_sentinel(logical_name)),
            };
            let schema = memh_kv_schema();
            let batches = memh_view_to_recordbatch(&view);
            if batches.is_empty() {
                return Arc::new(PluginAdvancedTable::empty_sentinel(logical_name));
            }
            match PluginAdvancedTable::try_new(logical_name, schema, batches) {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    log::error!("memtable PluginAdvancedTable (memh): {e}");
                    Arc::new(PluginAdvancedTable::empty_sentinel(logical_name))
                }
            }
        }
        None => Arc::new(PluginAdvancedTable::empty_sentinel(logical_name)),
    }
}

fn dtype_to_arrow(dt: DType) -> DataType {
    match dt {
        DType::U8 => DataType::UInt8,
        DType::U32 => DataType::UInt32,
        DType::I32 => DataType::Int32,
        DType::I64 => DataType::Int64,
        DType::F32 => DataType::Float32,
        DType::F64 => DataType::Float64,
        DType::U64 => DataType::UInt64,
        DType::Str => DataType::Utf8,
        DType::Bytes => DataType::Binary,
    }
}

/// Arrow schema mirroring a ring table's column layout.
pub fn view_to_arrow_schema(view: &MemTableView) -> SchemaRef {
    let s = view.schema();
    let fields: Vec<Field> = s
        .cols
        .iter()
        .map(|c| Field::new(&c.name, dtype_to_arrow(c.dtype), true))
        .collect();
    SchemaRef::new(Schema::new(fields))
}

enum ColBuilder {
    U8(UInt8Builder),
    U32(UInt32Builder),
    I32(Int32Builder),
    I64(Int64Builder),
    F32(Float32Builder),
    F64(Float64Builder),
    U64(UInt64Builder),
    Str(GenericStringBuilder<i32>),
    Bytes(BinaryBuilder),
}

fn make_builders(view: &MemTableView) -> Vec<ColBuilder> {
    view.schema()
        .cols
        .iter()
        .map(|c| match c.dtype {
            DType::U8 => ColBuilder::U8(UInt8Builder::new()),
            DType::U32 => ColBuilder::U32(UInt32Builder::new()),
            DType::I32 => ColBuilder::I32(Int32Builder::new()),
            DType::I64 => ColBuilder::I64(Int64Builder::new()),
            DType::F32 => ColBuilder::F32(Float32Builder::new()),
            DType::F64 => ColBuilder::F64(Float64Builder::new()),
            DType::U64 => ColBuilder::U64(UInt64Builder::new()),
            DType::Str => ColBuilder::Str(GenericStringBuilder::new()),
            DType::Bytes => ColBuilder::Bytes(BinaryBuilder::new()),
        })
        .collect()
}

/// Materialise one chunk into a `RecordBatch`.
///
/// Returns [`None`] when the chunk was recycled while being read (its
/// generation moved), or when reading panicked on a torn ref — both mean
/// the bytes can no longer be trusted, so the whole chunk is dropped
/// rather than surfacing corrupt rows to SQL.
fn chunk_to_recordbatch(
    view: &MemTableView,
    chunk: usize,
    arrow_schema: &SchemaRef,
) -> Option<RecordBatch> {
    let generation_before = view.chunk_generation(chunk);

    let arrays = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut builders = make_builders(view);
        // RowIter itself stops yielding once it observes a generation change;
        // rows read before that may still be torn, hence the re-check below.
        for row in view.rows(chunk) {
            let mut cursor = row.cursor();
            for builder in builders.iter_mut() {
                match builder {
                    ColBuilder::U8(b) => b.append_value(cursor.next_u8()),
                    ColBuilder::U32(b) => b.append_value(cursor.next_u32()),
                    ColBuilder::I32(b) => b.append_value(cursor.next_i32()),
                    ColBuilder::I64(b) => b.append_value(cursor.next_i64()),
                    ColBuilder::F32(b) => b.append_value(cursor.next_f32()),
                    ColBuilder::F64(b) => b.append_value(cursor.next_f64()),
                    ColBuilder::U64(b) => b.append_value(cursor.next_u64()),
                    ColBuilder::Str(b) => b.append_value(cursor.next_str()),
                    ColBuilder::Bytes(b) => b.append_value(cursor.next_bytes()),
                }
            }
        }
        builders
            .into_iter()
            .map(|b| -> ArrayRef {
                match b {
                    ColBuilder::U8(mut b) => Arc::new(b.finish()),
                    ColBuilder::U32(mut b) => Arc::new(b.finish()),
                    ColBuilder::I32(mut b) => Arc::new(b.finish()),
                    ColBuilder::I64(mut b) => Arc::new(b.finish()),
                    ColBuilder::F32(mut b) => Arc::new(b.finish()),
                    ColBuilder::F64(mut b) => Arc::new(b.finish()),
                    ColBuilder::U64(mut b) => Arc::new(b.finish()),
                    ColBuilder::Str(mut b) => Arc::new(b.finish()),
                    ColBuilder::Bytes(mut b) => Arc::new(b.finish()),
                }
            })
            .collect::<Vec<ArrayRef>>()
    }))
    .map_err(|_| {
        log::debug!("memtable chunk {chunk} recycled mid-read; dropping");
    })
    .ok()?;

    if view.chunk_generation(chunk) != generation_before {
        log::debug!("memtable chunk {chunk} recycled during materialisation; dropping");
        return None;
    }

    match RecordBatch::try_new(arrow_schema.clone(), arrays) {
        Ok(batch) if batch.num_rows() > 0 => Some(batch),
        Ok(_) => None,
        Err(e) => {
            log::error!("memtable chunk {chunk} → RecordBatch failed: {e}");
            None
        }
    }
}

/// Materialise a ring view as record batches in **logical (oldest → newest)
/// order**, one batch per surviving chunk.
///
/// Always returns at least one (possibly empty) batch so the table keeps its
/// real schema even when no rows are visible.
pub fn view_to_recordbatches(view: &MemTableView) -> Vec<RecordBatch> {
    let arrow_schema = view_to_arrow_schema(view);
    let mut batches: Vec<RecordBatch> = view
        .chunks_logical()
        .into_iter()
        .filter_map(|chunk| chunk_to_recordbatch(view, chunk, &arrow_schema))
        .collect();
    if batches.is_empty() {
        batches.push(RecordBatch::new_empty(arrow_schema));
    }
    batches
}

// ── Time-range pruning (chunk level) ──────────────────────────────────

/// Inclusive time window extracted from query predicates on the designated
/// timestamp column. `None` on either side = unbounded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TsBounds {
    pub lower: Option<i64>,
    pub upper: Option<i64>,
}

impl TsBounds {
    fn is_unbounded(&self) -> bool {
        self.lower.is_none() && self.upper.is_none()
    }

    fn tighten_lower(&mut self, v: i64) {
        self.lower = Some(self.lower.map_or(v, |cur| cur.max(v)));
    }

    fn tighten_upper(&mut self, v: i64) {
        self.upper = Some(self.upper.map_or(v, |cur| cur.min(v)));
    }
}

/// Integer value of a literal usable as a timestamp bound.
fn literal_as_i64(expr: &Expr) -> Option<i64> {
    let Expr::Literal(scalar, _) = expr else {
        return None;
    };
    match scalar {
        ScalarValue::Int64(Some(v)) => Some(*v),
        ScalarValue::Int32(Some(v)) => Some(*v as i64),
        ScalarValue::UInt32(Some(v)) => Some(*v as i64),
        ScalarValue::UInt64(Some(v)) => i64::try_from(*v).ok(),
        ScalarValue::TimestampMicrosecond(Some(v), _) => Some(*v),
        _ => None,
    }
}

fn is_ts_column(expr: &Expr, ts_name: &str) -> bool {
    matches!(expr, Expr::Column(c) if c.name == ts_name)
}

/// Fold one predicate into `bounds`. Conservative: comparisons are widened
/// to inclusive bounds (`>` treated as `>=`), unrecognised shapes are
/// ignored — pruning may keep too much, never too little.
fn fold_ts_predicate(expr: &Expr, ts_name: &str, bounds: &mut TsBounds) {
    match expr {
        Expr::BinaryExpr(be) if be.op == Operator::And => {
            fold_ts_predicate(&be.left, ts_name, bounds);
            fold_ts_predicate(&be.right, ts_name, bounds);
        }
        Expr::BinaryExpr(be) => {
            let (op, lit) = if is_ts_column(&be.left, ts_name) {
                let Some(v) = literal_as_i64(&be.right) else {
                    return;
                };
                (be.op, v)
            } else if is_ts_column(&be.right, ts_name) {
                // `lit op ts` — mirror the comparison.
                let Some(v) = literal_as_i64(&be.left) else {
                    return;
                };
                let mirrored = match be.op {
                    Operator::Gt => Operator::Lt,
                    Operator::GtEq => Operator::LtEq,
                    Operator::Lt => Operator::Gt,
                    Operator::LtEq => Operator::GtEq,
                    other => other,
                };
                (mirrored, v)
            } else {
                return;
            };
            match op {
                Operator::Gt | Operator::GtEq => bounds.tighten_lower(lit),
                Operator::Lt | Operator::LtEq => bounds.tighten_upper(lit),
                Operator::Eq => {
                    bounds.tighten_lower(lit);
                    bounds.tighten_upper(lit);
                }
                _ => {}
            }
        }
        Expr::Between(b) if !b.negated && is_ts_column(&b.expr, ts_name) => {
            if let Some(lo) = literal_as_i64(&b.low) {
                bounds.tighten_lower(lo);
            }
            if let Some(hi) = literal_as_i64(&b.high) {
                bounds.tighten_upper(hi);
            }
        }
        _ => {}
    }
}

/// Extract the time window implied by `filters` (each entry is ANDed by
/// DataFusion) on the column named `ts_name`.
pub fn ts_bounds_from_filters(filters: &[Expr], ts_name: &str) -> TsBounds {
    let mut bounds = TsBounds::default();
    for f in filters {
        fold_ts_predicate(f, ts_name, &mut bounds);
    }
    bounds
}

/// `false` only when the chunk's committed `[min_ts, max_ts]` provably lies
/// outside `bounds`. Races with the writer resolve to `true` (keep the
/// chunk) — materialisation re-validates the generation anyway.
fn chunk_may_match(view: &MemTableView, chunk: usize, bounds: &TsBounds) -> bool {
    if bounds.is_unbounded() {
        return true;
    }
    let generation_before = view.chunk_generation(chunk);
    let Some((min_ts, max_ts)) = view.chunk_ts_range(chunk) else {
        return true;
    };
    if view.chunk_generation(chunk) != generation_before {
        return true; // recycled mid-read: range untrustworthy, do not prune
    }
    !(bounds.lower.is_some_and(|lo| max_ts < lo) || bounds.upper.is_some_and(|hi| min_ts > hi))
}

/// Like [`view_to_recordbatches`], skipping chunks outside `bounds`.
pub fn view_to_recordbatches_pruned(view: &MemTableView, bounds: &TsBounds) -> Vec<RecordBatch> {
    let arrow_schema = view_to_arrow_schema(view);
    let mut batches: Vec<RecordBatch> = view
        .chunks_logical()
        .into_iter()
        .filter(|&chunk| chunk_may_match(view, chunk, bounds))
        .filter_map(|chunk| chunk_to_recordbatch(view, chunk, &arrow_schema))
        .collect();
    if batches.is_empty() {
        batches.push(RecordBatch::new_empty(arrow_schema));
    }
    batches
}

/// Like [`view_to_recordbatches_pruned`], additionally skipping any chunk
/// whose `(index, current generation)` is in `excluded` — used to drop hot
/// chunks already materialised from the cold tier, so a hot∪cold union counts
/// each row exactly once even while a compacted chunk still lives in the ring.
fn view_to_recordbatches_pruned_excluding(
    view: &MemTableView,
    bounds: &TsBounds,
    excluded: &HashSet<(usize, u64)>,
) -> Vec<RecordBatch> {
    let arrow_schema = view_to_arrow_schema(view);
    let mut batches: Vec<RecordBatch> = view
        .chunks_logical()
        .into_iter()
        .filter(|&chunk| chunk_may_match(view, chunk, bounds))
        .filter(|&chunk| !excluded.contains(&(chunk, view.chunk_generation(chunk))))
        .filter_map(|chunk| chunk_to_recordbatch(view, chunk, &arrow_schema))
        .collect();
    if batches.is_empty() {
        batches.push(RecordBatch::new_empty(arrow_schema));
    }
    batches
}

// ── Lazy ring TableProvider (prunes + materialises at scan time) ──────

/// [`TableProvider`] over an mmap'd MEMT ring file that defers Arrow
/// materialisation to `scan()`, where the query's filters are known:
/// chunks whose `[min_ts, max_ts]` cannot match the time predicates are
/// skipped without faulting in their pages.
#[derive(Debug)]
pub struct RingMmapTable {
    mapped: Arc<MappedFile>,
    schema: SchemaRef,
}

impl RingMmapTable {
    pub fn try_new(mapped: MappedFile) -> Result<Self, &'static str> {
        let view = MemTableView::new(mapped.as_bytes())?;
        let schema = view_to_arrow_schema(&view);
        Ok(Self {
            mapped: Arc::new(mapped),
            schema,
        })
    }

    /// Time window implied by `filters` on this ring's designated timestamp
    /// column (unbounded when there is no ts column or the file is torn).
    pub fn bounds_for(&self, filters: &[Expr]) -> TsBounds {
        match MemTableView::new(self.mapped.as_bytes()) {
            Ok(view) => view
                .ts_col()
                .map(|i| ts_bounds_from_filters(filters, view.col_name(i)))
                .unwrap_or_default(),
            Err(_) => TsBounds::default(),
        }
    }

    /// Materialise surviving chunks within `bounds`, one batch per chunk.
    pub fn pruned_batches(&self, bounds: &TsBounds) -> Vec<RecordBatch> {
        match MemTableView::new(self.mapped.as_bytes()) {
            Ok(view) => view_to_recordbatches_pruned(&view, bounds),
            Err(_) => vec![RecordBatch::new_empty(Arc::clone(&self.schema))],
        }
    }

    /// Like [`pruned_batches`](Self::pruned_batches), skipping chunks whose
    /// `(index, generation)` is already represented in the cold tier.
    fn pruned_batches_excluding(
        &self,
        bounds: &TsBounds,
        excluded: &HashSet<(usize, u64)>,
    ) -> Vec<RecordBatch> {
        match MemTableView::new(self.mapped.as_bytes()) {
            Ok(view) => view_to_recordbatches_pruned_excluding(&view, bounds, excluded),
            Err(_) => vec![RecordBatch::new_empty(Arc::clone(&self.schema))],
        }
    }
}

#[async_trait]
impl TableProvider for RingMmapTable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DfResult<Vec<TableProviderFilterPushDown>> {
        supports_filters_pushdown_for_schema(&self.schema, filters)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let bounds = self.bounds_for(filters);
        let batches = self.pruned_batches(&bounds);
        scan_memory_partitions(
            state,
            Arc::clone(&self.schema),
            &[batches],
            projection,
            filters,
            limit,
        )
        .await
    }
}

// ── Cold segments (MEMC) → Arrow, with two-level time pruning ─────────

/// `.memc` segment paths in `dir`, or empty if the dir does not exist.
/// Read-only: never creates the directory (unlike `ColdStore::open`).
fn cold_segment_paths(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("memc") {
                out.push(p);
            }
        }
    }
    out
}

/// One decoded cold column → an Arrow array (schema order is preserved).
fn cold_column_to_array(col: ColumnData) -> ArrayRef {
    match col {
        ColumnData::U8(v) => Arc::new(UInt8Array::from(v)),
        ColumnData::U32(v) => Arc::new(UInt32Array::from(v)),
        ColumnData::I32(v) => Arc::new(Int32Array::from(v)),
        ColumnData::I64(v) => Arc::new(Int64Array::from(v)),
        ColumnData::F32(v) => Arc::new(Float32Array::from(v)),
        ColumnData::F64(v) => Arc::new(Float64Array::from(v)),
        ColumnData::U64(v) => Arc::new(UInt64Array::from(v)),
        ColumnData::Str(v) => Arc::new(StringArray::from_iter_values(v)),
        ColumnData::Bytes(v) => Arc::new(BinaryArray::from_iter_values(v)),
    }
}

/// Decode the cold pages of `table` within `bounds`, returning the batches and
/// the set of hot-ring `(chunk index, generation)` those pages came from.
///
/// Two-level pruning mirrors the hot ring: sealed segments whose header
/// `ts_range` cannot match are skipped without reading pages, then each
/// segment's page directory is pruned per-page before decode. The returned
/// `covered` set lets the caller drop the corresponding still-resident hot
/// chunks so a hot∪cold union never double-counts a compacted chunk.
fn cold_scan(
    dir: &std::path::Path,
    table: &str,
    schema: &SchemaRef,
    bounds: &TsBounds,
) -> (Vec<RecordBatch>, HashSet<(usize, u64)>) {
    let mut out = Vec::new();
    let mut covered: HashSet<(usize, u64)> = HashSet::new();
    for path in cold_segment_paths(dir) {
        let Ok(reader) = SegmentReader::open(&path) else {
            continue; // unreadable/foreign file: skip rather than fail the scan
        };
        if let Some((smin, smax)) = reader.ts_range() {
            if bounds.lower.is_some_and(|lo| smax < lo) || bounds.upper.is_some_and(|hi| smin > hi)
            {
                continue; // segment-level prune: whole file out of range
            }
        }
        let Some(tid) = reader.table_id_by_name(table) else {
            continue; // this segment holds no pages for the queried table
        };
        let pages = reader.pages();
        for idx in reader.pages_in_range(tid, bounds.lower, bounds.upper) {
            if let Some(p) = pages.get(idx) {
                if p.source_chunk != probing_memtable::memc::SOURCE_CHUNK_NONE {
                    covered.insert((p.source_chunk as usize, p.source_gen));
                }
            }
            match reader.read_page(idx) {
                Ok(cols) => {
                    let arrays: Vec<ArrayRef> =
                        cols.into_iter().map(cold_column_to_array).collect();
                    match RecordBatch::try_new(Arc::clone(schema), arrays) {
                        Ok(b) if b.num_rows() > 0 => out.push(b),
                        Ok(_) => {}
                        Err(e) => log::error!("cold page {idx} → RecordBatch failed: {e}"),
                    }
                }
                Err(e) => log::debug!("cold page {idx} decode skipped: {e}"),
            }
        }
    }
    (out, covered)
}

/// [`TableProvider`] unioning a hot ring with its cold MEMC segments under one
/// logical table. A single time predicate prunes both tiers: hot chunks by
/// `[min_ts, max_ts]`, cold segments/pages by their recorded ranges. Hot and
/// cold batches are handed to the scan as two partitions, so projection,
/// filter, and limit pushdown apply uniformly across both.
#[derive(Debug)]
pub struct HotColdTable {
    hot: RingMmapTable,
    cold_dir: std::path::PathBuf,
    table: String,
    schema: SchemaRef,
}

impl HotColdTable {
    pub fn new(hot: RingMmapTable, cold_dir: std::path::PathBuf, table: impl Into<String>) -> Self {
        let schema = hot.schema();
        Self {
            hot,
            cold_dir,
            table: table.into(),
            schema,
        }
    }
}

#[async_trait]
impl TableProvider for HotColdTable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DfResult<Vec<TableProviderFilterPushDown>> {
        supports_filters_pushdown_for_schema(&self.schema, filters)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let bounds = self.hot.bounds_for(filters);
        let (cold, covered) = cold_scan(&self.cold_dir, &self.table, &self.schema, &bounds);
        // Drop hot chunks already in cold so each row is counted once.
        let hot = self.hot.pruned_batches_excluding(&bounds, &covered);

        let partitions: Vec<Vec<RecordBatch>> = if cold.is_empty() {
            vec![hot]
        } else {
            vec![hot, cold]
        };
        scan_memory_partitions(
            state,
            Arc::clone(&self.schema),
            &partitions,
            projection,
            filters,
            limit,
        )
        .await
    }
}

/// Route an mmap'd file to its [`TableProvider`]: MEMT rings get the lazy
/// pruning provider; MEMH (and anything else) keeps the eager path.
pub fn mapped_file_to_table(mapped: MappedFile, logical_name: &str) -> Arc<dyn TableProvider> {
    match detect_table(mapped.as_bytes()) {
        Some(TableKind::Ring) => match RingMmapTable::try_new(mapped) {
            Ok(t) => Arc::new(t),
            Err(_) => Arc::new(PluginAdvancedTable::empty_sentinel(logical_name)),
        },
        _ => bytes_to_pushdown_table(mapped.as_bytes(), logical_name),
    }
}

// ── MEMH: key-value table → two-column RecordBatch ────────────────────

/// Fixed Arrow schema for MEMH tables: `key` (Utf8) + `value` (Utf8).
///
/// All MEMH values are serialised to strings so that heterogeneous value types
/// (scalars, strings, bytes) can be represented in a single column and queried
/// with SQL string predicates.
fn memh_kv_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("key", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]))
}

fn typed_value_to_str(v: &TypedValue<'_>) -> String {
    match v {
        TypedValue::U8(n) => n.to_string(),
        TypedValue::I32(n) => n.to_string(),
        TypedValue::I64(n) => n.to_string(),
        TypedValue::F32(n) => n.to_string(),
        TypedValue::F64(n) => n.to_string(),
        TypedValue::U64(n) => n.to_string(),
        TypedValue::U32(n) => n.to_string(),
        TypedValue::Str(s) => s.to_string(),
        TypedValue::Bytes(b) => {
            // Hex-encode without adding a dep; e.g. "0xdeadbeef"
            let mut out = String::with_capacity(2 + b.len() * 2);
            out.push_str("0x");
            for byte in *b {
                use std::fmt::Write;
                let _ = write!(out, "{byte:02x}");
            }
            out
        }
    }
}

fn memh_view_to_recordbatch(view: &MemhView<'_>) -> Vec<RecordBatch> {
    let schema = memh_kv_schema();
    let mut keys: GenericStringBuilder<i32> = GenericStringBuilder::new();
    let mut values: GenericStringBuilder<i32> = GenericStringBuilder::new();

    for (k, v) in view.iter() {
        keys.append_value(k);
        values.append_value(typed_value_to_str(&v));
    }

    match RecordBatch::try_new(
        schema,
        vec![Arc::new(keys.finish()), Arc::new(values.finish())],
    ) {
        Ok(batch) => vec![batch],
        Err(e) => {
            log::error!("memh → RecordBatch failed: {e}");
            vec![]
        }
    }
}

// ── Dynamic schemas from mmap filenames ───────────────────────────────

/// One DataFusion schema combining mmap-backed tables with an optional inner
/// (static) provider.
///
/// Lookup order: mmap file first, then `inner`. Mmap files only exist when a
/// producer explicitly created them, so they take precedence over static
/// providers — some of which (e.g. lazy namespaces) claim every name exists.
#[derive(Debug)]
pub struct MmapFileSchemaProvider {
    schema: String,
    inner: Option<Arc<dyn SchemaProvider>>,
}

impl MmapFileSchemaProvider {
    pub fn new(schema: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            inner: None,
        }
    }

    /// Merge with a static provider: mmap tables shadow `inner` only on
    /// exact-name collision; everything else falls through.
    pub fn with_inner(schema: impl Into<String>, inner: Option<Arc<dyn SchemaProvider>>) -> Self {
        Self {
            schema: schema.into(),
            inner,
        }
    }
}

#[async_trait]
impl SchemaProvider for MmapFileSchemaProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        let mut names = tables_in_schema(&self.schema);
        if let Some(inner) = &self.inner {
            names.extend(inner.table_names());
        }
        names.sort();
        names.dedup();
        names
    }

    async fn table(&self, name: &str) -> DfResult<Option<Arc<dyn TableProvider>>> {
        if mmap_table_exists(&self.schema, name) {
            let basename = mmap_filename_for(&self.schema, name);
            let path = self_dir().join(&basename);
            // Zero-copy read: map the file instead of copying it to the heap.
            // Ring files materialise lazily at scan() time with chunk-level
            // time pruning; only surviving chunk bytes get faulted in. A ring
            // is unioned with its cold MEMC segments (keyed by the unique
            // on-disk basename) so one query spans both tiers.
            if let Ok(mapped) = MappedFile::open(&path) {
                if let Some(TableKind::Ring) = detect_table(mapped.as_bytes()) {
                    return Ok(Some(match RingMmapTable::try_new(mapped) {
                        Ok(ring) => Arc::new(HotColdTable::new(ring, cold_dir(), basename)),
                        Err(_) => Arc::new(PluginAdvancedTable::empty_sentinel(name)),
                    }));
                }
                return Ok(Some(mapped_file_to_table(mapped, name)));
            }
        }
        match &self.inner {
            Some(inner) => inner.table(name).await,
            None => Ok(None),
        }
    }

    fn register_table(
        &self,
        name: String,
        table: Arc<dyn TableProvider>,
    ) -> DfResult<Option<Arc<dyn TableProvider>>> {
        match &self.inner {
            Some(inner) => inner.register_table(name, table),
            None => Err(DataFusionError::NotImplemented(
                "unable to create tables".to_string(),
            )),
        }
    }

    fn deregister_table(&self, name: &str) -> DfResult<Option<Arc<dyn TableProvider>>> {
        match &self.inner {
            Some(inner) => inner.deregister_table(name),
            None => Err(DataFusionError::NotImplemented(
                "unable to drop tables".to_string(),
            )),
        }
    }

    fn table_exist(&self, name: &str) -> bool {
        mmap_table_exists(&self.schema, name)
            || self
                .inner
                .as_ref()
                .map(|inner| inner.table_exist(name))
                .unwrap_or(false)
    }
}

/// Wraps the `probe` catalog: static schemas (python, cluster, …) keep
/// working, mmap-backed schemas are discovered at query time, and when both
/// exist for the same name they are **merged** (mmap tables first) instead of
/// the mmap side shadowing the static provider.
#[derive(Debug)]
struct DynamicMmapCatalog {
    inner: Arc<dyn CatalogProvider>,
}

impl CatalogProvider for DynamicMmapCatalog {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema_names(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = self.inner.schema_names().into_iter().collect();
        for sch in discover_all_schemas() {
            names.insert(sch);
        }
        names.into_iter().collect()
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        let inner = self.inner.schema(name);
        let has_mmap = name == DEFAULT_UNDOTTED_SCHEMA || !tables_in_schema(name).is_empty();
        match (has_mmap, inner) {
            (true, inner) => Some(Arc::new(MmapFileSchemaProvider::with_inner(name, inner))),
            (false, Some(inner)) => Some(inner),
            (false, None) => None,
        }
    }

    fn register_schema(
        &self,
        name: &str,
        schema: Arc<dyn SchemaProvider>,
    ) -> DfResult<Option<Arc<dyn SchemaProvider>>> {
        self.inner.register_schema(name, schema)
    }
}

/// Namespace plugin that wraps the `probe` catalog with [`DynamicMmapCatalog`]
/// for dynamic schema discovery from mmap files at query time.
#[derive(Debug, Default)]
pub struct UnifiedMemtablePlugin;

impl Plugin for UnifiedMemtablePlugin {
    fn name(&self) -> String {
        "mmap_memtables".into()
    }
    fn kind(&self) -> PluginType {
        PluginType::Namespace
    }
    fn namespace(&self) -> String {
        "memtable".into()
    }

    fn provide_catalog(&self, inner: Arc<dyn CatalogProvider>) -> Option<Arc<dyn CatalogProvider>> {
        Some(Arc::new(DynamicMmapCatalog { inner }))
    }
}

// ── Cold compaction runtime owner ─────────────────────────────────────

/// Tunables for the background hot→cold compactor.
#[derive(Clone, Debug)]
pub struct ColdRuntimeConfig {
    /// Whether the background compactor thread runs.
    pub enabled: bool,
    /// Sleep between drain passes.
    pub poll: Duration,
    /// Seal + roll a segment once it reaches this size (fragmentation knob).
    pub target_segment_bytes: u64,
    /// Seal an idle open segment after this long so it becomes queryable.
    pub max_segment_age: Duration,
    /// Cold-store byte budget; oldest segments evicted past it.
    pub max_total_bytes: Option<u64>,
    /// Drop cold segments older than this.
    pub ttl: Option<Duration>,
}

impl Default for ColdRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll: Duration::from_secs(2),
            target_segment_bytes: 64 * 1024 * 1024,
            max_segment_age: Duration::from_secs(300),
            max_total_bytes: None,
            ttl: None,
        }
    }
}

impl ColdRuntimeConfig {
    fn to_compactor(&self) -> CompactorConfig {
        CompactorConfig {
            target_segment_bytes: self.target_segment_bytes,
            max_segment_age: self.max_segment_age,
            poll_interval: self.poll,
            max_total_bytes: self.max_total_bytes,
            ttl: self.ttl,
        }
    }

    /// Build a config from `PROBING_COLD*` environment variables, used to
    /// auto-start compaction at engine init (opt-in, off by default).
    pub fn from_env() -> Self {
        fn env_u64(k: &str) -> Option<u64> {
            std::env::var(k).ok().and_then(|v| v.trim().parse().ok())
        }
        let mut c = Self::default();
        if let Ok(v) = std::env::var("PROBING_COLD") {
            c.enabled = matches!(v.trim(), "1" | "on" | "true" | "yes");
        }
        if let Some(mb) = env_u64("PROBING_COLD_TARGET_MB") {
            c.target_segment_bytes = mb.saturating_mul(1024 * 1024);
        }
        if let Some(mb) = env_u64("PROBING_COLD_MAX_TOTAL_MB") {
            c.max_total_bytes = Some(mb.saturating_mul(1024 * 1024));
        }
        if let Some(s) = env_u64("PROBING_COLD_TTL_SECS") {
            c.ttl = Some(Duration::from_secs(s));
        }
        if let Some(ms) = env_u64("PROBING_COLD_POLL_MS") {
            c.poll = Duration::from_millis(ms.max(50));
        }
        if let Some(s) = env_u64("PROBING_COLD_MAX_AGE_SECS") {
            c.max_segment_age = Duration::from_secs(s);
        }
        c
    }
}

/// Ring files under `self_dir()` that are candidate compaction sources,
/// returned as `(on-disk basename, path)`. The basename is the cold table
/// identity (matching the SQL read path), so names never collide across
/// schemas. The `cold/` subdir is skipped (it is a directory, not a file).
fn cold_source_candidates() -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(self_dir()) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if classify_mmap_basename(&name).is_some() {
                out.push((name, p));
            }
        }
    }
    out
}

/// Process-global owner of the background hot→cold compactor thread.
///
/// Modeled on the task-stats worker: a lazy singleton with start/stop, so the
/// compactor has a single lifecycle home regardless of how many producers
/// create hot tables. The loop rediscovers ring files each pass (tables appear
/// over time), drains newly-sealed chunks into the shared cold store, rolls
/// segments by age, and enforces the byte/TTL budget.
pub struct ColdCompactor {
    running: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl ColdCompactor {
    pub fn instance() -> &'static Self {
        static INSTANCE: Lazy<ColdCompactor> = Lazy::new(|| ColdCompactor {
            running: Arc::new(AtomicBool::new(false)),
            handle: Mutex::new(None),
        });
        &INSTANCE
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// (Re)apply `cfg`: stop any running thread, then start a fresh one when
    /// `cfg.enabled`. Idempotent and the single entry point for the config
    /// surface, so changing a knob simply restarts with the new settings.
    pub fn apply(&self, cfg: ColdRuntimeConfig) {
        self.stop();
        if cfg.enabled {
            self.start(cfg);
        }
    }

    fn start(&self, cfg: ColdRuntimeConfig) {
        if self.running.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let dir = cold_dir();
        let store = match ColdStore::open(&dir) {
            Ok(s) => s,
            Err(e) => {
                log::error!("cold compactor: cannot open {}: {e}", dir.display());
                self.running.store(false, Ordering::SeqCst);
                return;
            }
        };
        let mut compactor = Compactor::new(store, cfg.to_compactor());
        // Exactly-once across restarts: recover per-chunk watermarks from any
        // segments already on disk before draining.
        if let Err(e) = compactor.prime_from_cold() {
            log::warn!("cold compactor: prime_from_cold failed: {e}");
        }

        let running = self.running.clone();
        let poll = cfg.poll;
        let handle = std::thread::Builder::new()
            .name("memc-compactor".into())
            .spawn(move || {
                while running.load(Ordering::SeqCst) {
                    for (name, path) in cold_source_candidates() {
                        let Ok(mapped) = MappedFile::open(&path) else {
                            continue;
                        };
                        if !matches!(detect_table(mapped.as_bytes()), Some(TableKind::Ring)) {
                            continue; // only ring tables tier to cold
                        }
                        if let Ok(view) = MemTableView::new(mapped.as_bytes()) {
                            if let Err(e) = compactor.drain_view(&name, &view) {
                                log::debug!("cold compactor: drain {name}: {e}");
                            }
                        }
                    }
                    let _ = compactor.maybe_roll_on_age();
                    let _ = compactor.enforce();
                    sleep_interruptible(&running, poll);
                }
                // Final flush so the last open segment is sealed on shutdown.
                if let Err(e) = compactor.flush() {
                    log::debug!("cold compactor: final flush: {e}");
                }
            })
            .expect("spawn memc-compactor thread");
        *self.handle.lock().unwrap() = Some(handle);
    }

    /// Signal the thread to flush and exit, then join it.
    pub fn stop(&self) {
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Some(h) = self.handle.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    pub fn stats(&self) -> Option<ColdStats> {
        ColdStore::open(cold_dir()).ok().map(|s| s.stats())
    }
}

/// Sleep up to `total`, waking early (within ~200ms) if `running` is cleared.
fn sleep_interruptible(running: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(200);
    let mut left = total;
    while left > Duration::ZERO && running.load(Ordering::SeqCst) {
        let nap = left.min(step);
        std::thread::sleep(nap);
        left = left.saturating_sub(nap);
    }
}

/// Start (or stop) background compaction from `PROBING_COLD*` env vars.
/// Call once after the engine is built; off by default.
pub fn start_cold_compaction_from_env() {
    ColdCompactor::instance().apply(ColdRuntimeConfig::from_env());
}

// ── EngineExtension ────────────────────────────────────────────────────

/// Exposes mmap memtables to SQL and owns the cold-compaction config surface.
///
/// Config knobs (also settable via `SET memtable.<key> = ...`):
/// - `cold_compaction` (`on`/`off`) — run the background compactor.
/// - `cold_max_total_mb` — cold-store byte budget in MiB.
/// - `cold_ttl_secs` — evict cold segments older than this.
#[derive(Debug, Default, EngineExtensionDerive)]
pub struct MemTableExtension {
    /// Background hot→cold compaction switch: "on" or "off".
    #[option(aliases = ["cold.compaction"])]
    cold_compaction: Maybe<String>,
    /// Cold-store byte budget in MiB (oldest segments evicted past it).
    #[option(aliases = ["cold.max_total_mb"])]
    cold_max_total_mb: Maybe<i64>,
    /// Evict cold segments older than this many seconds.
    #[option(aliases = ["cold.ttl_secs"])]
    cold_ttl_secs: Maybe<i64>,
}

impl MemTableExtension {
    fn cold_enabled(&self) -> bool {
        matches!(
            self.cold_compaction,
            Maybe::Just(ref s) if matches!(s.trim(), "1" | "on" | "true" | "yes")
        )
    }

    /// Merge the current option fields over the env-derived defaults.
    fn cold_config(&self) -> ColdRuntimeConfig {
        let mut cfg = ColdRuntimeConfig::from_env();
        cfg.enabled = self.cold_enabled();
        if let Maybe::Just(mb) = self.cold_max_total_mb {
            cfg.max_total_bytes = (mb > 0).then(|| (mb as u64).saturating_mul(1024 * 1024));
        }
        if let Maybe::Just(s) = self.cold_ttl_secs {
            cfg.ttl = (s > 0).then(|| Duration::from_secs(s as u64));
        }
        cfg
    }

    fn apply_cold(&self) {
        ColdCompactor::instance().apply(self.cold_config());
    }

    fn set_cold_compaction(&mut self, v: Maybe<String>) -> Result<(), EngineError> {
        self.cold_compaction = v;
        self.apply_cold();
        Ok(())
    }

    fn set_cold_max_total_mb(&mut self, v: Maybe<i64>) -> Result<(), EngineError> {
        self.cold_max_total_mb = v;
        self.apply_cold();
        Ok(())
    }

    fn set_cold_ttl_secs(&mut self, v: Maybe<i64>) -> Result<(), EngineError> {
        self.cold_ttl_secs = v;
        self.apply_cold();
        Ok(())
    }
}

impl EngineCall for MemTableExtension {}

impl EngineDatasource for MemTableExtension {
    fn datasrc(
        &self,
        _namespace: &str,
        _name: Option<&str>,
    ) -> Option<Arc<dyn Plugin + Sync + Send>> {
        Some(Arc::new(UnifiedMemtablePlugin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{AsArray, Float64Array, Int32Array, Int64Array, UInt8Array};
    use probing_memtable::{MemTable, Schema as MtSchema, Value};
    use std::sync::Mutex;

    /// `PROBING_DATA_DIR` is process-global; serialize tests that mutate it.
    static PROBING_DATA_DIR_LOCK: Mutex<()> = Mutex::new(());

    fn concat_i64(batches: &[RecordBatch], col: usize) -> Vec<i64> {
        batches
            .iter()
            .flat_map(|b| {
                let a = b.column(col).as_any().downcast_ref::<Int64Array>().unwrap();
                (0..a.len()).map(|i| a.value(i)).collect::<Vec<_>>()
            })
            .collect()
    }

    fn collect_i32(batches: &[RecordBatch]) -> Vec<i32> {
        batches
            .iter()
            .flat_map(|b| {
                let a = b.column(0).as_any().downcast_ref::<Int32Array>().unwrap();
                (0..a.len()).map(|i| a.value(i)).collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn dtype_mapping_covers_all_variants() {
        assert_eq!(dtype_to_arrow(DType::U8), DataType::UInt8);
        assert_eq!(dtype_to_arrow(DType::U32), DataType::UInt32);
        assert_eq!(dtype_to_arrow(DType::I32), DataType::Int32);
        assert_eq!(dtype_to_arrow(DType::I64), DataType::Int64);
        assert_eq!(dtype_to_arrow(DType::F32), DataType::Float32);
        assert_eq!(dtype_to_arrow(DType::F64), DataType::Float64);
        assert_eq!(dtype_to_arrow(DType::U64), DataType::UInt64);
        assert_eq!(dtype_to_arrow(DType::Str), DataType::Utf8);
        assert_eq!(dtype_to_arrow(DType::Bytes), DataType::Binary);
    }

    #[test]
    fn recordbatch_from_mixed_types() {
        let schema = MtSchema::new()
            .col("id", DType::I32)
            .col("value", DType::F64)
            .col("tag", DType::Str);
        let mut t = MemTable::new(&schema, 4096, 2);
        t.push_row(&[Value::I32(1), Value::F64(3.14), Value::Str("hello")]);
        t.push_row(&[Value::I32(2), Value::F64(2.72), Value::Str("world")]);

        let view = t.view();
        let batches = view_to_recordbatches(&view);
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);

        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!(ids.value(0), 1);
        assert_eq!(ids.value(1), 2);

        let vals = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((vals.value(0) - 3.14).abs() < 1e-10);
        assert!((vals.value(1) - 2.72).abs() < 1e-10);

        let tags: &datafusion::arrow::array::StringArray = batch.column(2).as_string();
        assert_eq!(tags.value(0), "hello");
        assert_eq!(tags.value(1), "world");
    }

    #[test]
    fn recordbatches_multiple_chunks_in_logical_order() {
        let schema = MtSchema::new().col("v", DType::I64);
        // Small chunk so rows spill across chunks
        let mut t = MemTable::new(&schema, 128, 4);
        for i in 0..20 {
            t.push_row(&[Value::I64(i)]);
        }

        let view = t.view();
        let batches = view_to_recordbatches(&view);
        assert!(!batches.is_empty());

        // Concatenated in logical order, surviving values must be strictly
        // increasing — even though the ring may have wrapped.
        let values = concat_i64(&batches, 0);
        assert!(!values.is_empty());
        for w in values.windows(2) {
            assert!(w[1] > w[0], "values not in logical order: {values:?}");
        }
        // The most recent row always survives.
        assert_eq!(*values.last().unwrap(), 19);
    }

    #[test]
    fn recordbatches_logical_order_after_wrap() {
        let schema = MtSchema::new().col("v", DType::I64);
        let mut t = MemTable::new(&schema, 80, 2);
        t.push_row(&[Value::I64(10)]); // chunk 0, gen 1
        t.advance_chunk();
        t.push_row(&[Value::I64(20)]); // chunk 1, gen 1
        t.advance_chunk(); // wrap: chunk 0 → gen 2
        t.push_row(&[Value::I64(30)]); // chunk 0, gen 2

        let view = t.view();
        let batches = view_to_recordbatches(&view);
        // chunk 1 (older) first, then recycled chunk 0
        assert_eq!(concat_i64(&batches, 0), vec![20, 30]);
    }

    #[test]
    fn recordbatch_empty_table_keeps_schema() {
        let schema = MtSchema::new().col("x", DType::U8);
        let t = MemTable::new(&schema, 1024, 1);
        let view = t.view();
        let batches = view_to_recordbatches(&view);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 0);
        assert_eq!(batches[0].schema().field(0).name(), "x");
    }

    #[test]
    fn arrow_schema_matches_memtable_schema() {
        let schema = MtSchema::new()
            .col("ts", DType::I64)
            .col("cpu", DType::F64)
            .col("name", DType::Str);
        let t = MemTable::new(&schema, 1024, 1);
        let view = t.view();
        let arrow = view_to_arrow_schema(&view);

        assert_eq!(arrow.fields().len(), 3);
        assert_eq!(arrow.field(0).name(), "ts");
        assert_eq!(*arrow.field(0).data_type(), DataType::Int64);
        assert_eq!(arrow.field(1).name(), "cpu");
        assert_eq!(*arrow.field(1).data_type(), DataType::Float64);
        assert_eq!(arrow.field(2).name(), "name");
        assert_eq!(*arrow.field(2).data_type(), DataType::Utf8);
    }

    #[test]
    fn recordbatch_u8_column() {
        let schema = MtSchema::new().col("flag", DType::U8);
        let mut t = MemTable::new(&schema, 1024, 1);
        t.push_row(&[Value::U8(0)]);
        t.push_row(&[Value::U8(255)]);

        let view = t.view();
        let batches = view_to_recordbatches(&view);
        let col = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<UInt8Array>()
            .unwrap();
        assert_eq!(col.value(0), 0);
        assert_eq!(col.value(1), 255);
    }

    // ── time-range pruning ─────────────────────────────────────────────

    #[test]
    fn ts_bounds_extraction_from_filters() {
        use datafusion::prelude::{col, lit};

        // Conjunction across filter entries
        let b = ts_bounds_from_filters(
            &[col("ts").gt_eq(lit(100i64)), col("ts").lt(lit(200i64))],
            "ts",
        );
        assert_eq!(
            b,
            TsBounds {
                lower: Some(100),
                upper: Some(200)
            }
        );

        // AND inside one entry + tightening
        let f = col("ts").gt(lit(10i64)).and(col("ts").gt(lit(50i64)));
        assert_eq!(ts_bounds_from_filters(&[f], "ts").lower, Some(50));

        // Literal on the left mirrors the comparison: 300 <= ts
        let f = lit(300i64).lt_eq(col("ts"));
        assert_eq!(ts_bounds_from_filters(&[f], "ts").lower, Some(300));

        // BETWEEN
        let f = col("ts").between(lit(10i64), lit(20i64));
        let b = ts_bounds_from_filters(&[f], "ts");
        assert_eq!((b.lower, b.upper), (Some(10), Some(20)));

        // Equality pins both sides
        let f = col("ts").eq(lit(42i64));
        let b = ts_bounds_from_filters(&[f], "ts");
        assert_eq!((b.lower, b.upper), (Some(42), Some(42)));

        // OR cannot be folded → unbounded (conservative)
        let f = col("ts").gt(lit(5i64)).or(col("v").eq(lit(1i64)));
        let b = ts_bounds_from_filters(&[f], "ts");
        assert_eq!((b.lower, b.upper), (None, None));

        // Predicates on other columns are ignored
        let b = ts_bounds_from_filters(&[col("v").gt(lit(5i64))], "ts");
        assert_eq!((b.lower, b.upper), (None, None));
    }

    #[test]
    fn pruned_batches_skip_out_of_range_chunks() {
        let schema = MtSchema::new().col("ts", DType::I64);
        // ChunkHeader=40, I64 row=12 → 64-40=24 → 2 rows per chunk
        let mut t = MemTable::new(&schema, 64, 4);
        for ts in [10i64, 20, 30, 40, 50, 60] {
            t.push_row(&[Value::I64(ts)]);
        }
        let view = t.view();
        assert_eq!(view_to_recordbatches(&view).len(), 3);

        // lower bound falls inside chunk 1: chunk 0 (max 20) pruned
        let pruned = view_to_recordbatches_pruned(
            &view,
            &TsBounds {
                lower: Some(35),
                upper: None,
            },
        );
        assert_eq!(concat_i64(&pruned, 0), vec![30, 40, 50, 60]);

        // tight window: only the chunk containing [50, 60] survives
        let pruned = view_to_recordbatches_pruned(
            &view,
            &TsBounds {
                lower: Some(55),
                upper: Some(58),
            },
        );
        assert_eq!(concat_i64(&pruned, 0), vec![50, 60]);

        // window past all data: everything pruned, schema kept
        let pruned = view_to_recordbatches_pruned(
            &view,
            &TsBounds {
                lower: Some(1000),
                upper: None,
            },
        );
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].num_rows(), 0);
        assert_eq!(pruned[0].schema().field(0).name(), "ts");

        // unbounded: identical to the unpruned materialisation
        let unpruned = view_to_recordbatches_pruned(&view, &TsBounds::default());
        assert_eq!(concat_i64(&unpruned, 0), vec![10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn tables_without_ts_col_are_never_pruned() {
        let schema = MtSchema::new().col("v", DType::I64); // not a ts name
        let mut t = MemTable::new(&schema, 64, 4);
        for v in [1i64, 2, 3, 4] {
            t.push_row(&[Value::I64(v)]);
        }
        let view = t.view();
        assert_eq!(view.ts_col(), None);
        // Even with bounds set, chunks without ts metadata must survive.
        let batches = view_to_recordbatches_pruned(
            &view,
            &TsBounds {
                lower: Some(100),
                upper: None,
            },
        );
        assert_eq!(concat_i64(&batches, 0), vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn ring_mmap_table_sql_end_to_end() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
        use datafusion::prelude::SessionContext;
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        let schema = MtSchema::new()
            .col("timestamp", DType::I64)
            .col("v", DType::I32);
        // 2 rows per chunk → 12 rows spread over 8 chunks
        let mut table = ExposedTable::create("prune_demo", &schema, 80, 8).unwrap();
        for i in 1i64..=12 {
            table.push_row(&[Value::I64(i * 100), Value::I32(i as i32)]);
        }

        let path = self_dir().join("prune_demo");
        let mapped = MappedFile::open(&path).unwrap();
        let provider = mapped_file_to_table(mapped, "prune_demo");
        assert!(
            provider.as_any().downcast_ref::<RingMmapTable>().is_some(),
            "ring files must get the lazy pruning provider"
        );

        let ctx = SessionContext::new();
        ctx.register_table("prune_demo", provider).unwrap();
        let batches = ctx
            .sql("SELECT v FROM prune_demo WHERE timestamp >= 700 AND timestamp < 1100 ORDER BY v")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let got: Vec<i32> = batches
            .iter()
            .flat_map(|b| {
                let a = b
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .unwrap();
                (0..a.len()).map(|i| a.value(i)).collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(got, vec![7, 8, 9, 10]);

        drop(table);
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }

    #[tokio::test]
    async fn hot_cold_union_dedups_and_spans_time() {
        use datafusion::prelude::SessionContext;
        use probing_memtable::memc::{ColdStore, Compactor, CompactorConfig};

        let tmp = tempfile::tempdir().unwrap();
        let hot_path = tmp.path().join("hc_demo");
        let cold = tmp.path().join("cold");

        let schema = MtSchema::new()
            .col("timestamp", DType::I64)
            .col("v", DType::I32);
        // 2 rows per chunk, 4 chunks.
        let mut t = MemTable::file_at(&hot_path, &schema, 80, 4).unwrap();
        for i in 1i64..=6 {
            t.push_row(&[Value::I64(i * 100), Value::I32(i as i32)]);
        }
        // chunks 0,1 sealed (ts 100,200 / 300,400); chunk 2 full-but-writing (500,600).

        {
            let store = ColdStore::open(&cold).unwrap();
            let mut c = Compactor::new(
                store,
                CompactorConfig {
                    target_segment_bytes: 1 << 30,
                    ..Default::default()
                },
            );
            let drained = c.drain_view("hc_demo", &t.view()).unwrap();
            assert_eq!(drained, 4, "two sealed chunks → 4 rows compacted");
            c.flush().unwrap();
        }

        let mapped = MappedFile::open(&hot_path).unwrap();
        let ring = RingMmapTable::try_new(mapped).unwrap();
        let provider: Arc<dyn TableProvider> =
            Arc::new(HotColdTable::new(ring, cold.clone(), "hc_demo"));

        let ctx = SessionContext::new();
        ctx.register_table("hc_demo", provider).unwrap();

        // Full scan: cold (4) + hot tail (2), with the still-resident compacted
        // chunks deduped out of hot — exactly-once across tiers.
        let all = ctx
            .sql("SELECT v FROM hc_demo ORDER BY v")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(collect_i32(&all), vec![1, 2, 3, 4, 5, 6]);

        // One time predicate prunes both tiers and selects across the boundary.
        let span = ctx
            .sql("SELECT v FROM hc_demo WHERE timestamp >= 200 AND timestamp <= 500 ORDER BY v")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(collect_i32(&span), vec![2, 3, 4, 5]);

        drop(t);
    }

    #[tokio::test]
    async fn cold_compactor_runtime_drains_and_is_queryable() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
        use datafusion::prelude::SessionContext;
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        let schema = MtSchema::new()
            .col("timestamp", DType::I64)
            .col("v", DType::I32);
        let mut table = ExposedTable::create("rt_demo", &schema, 80, 8).unwrap();
        for i in 1i64..=6 {
            table.push_row(&[Value::I64(i * 100), Value::I32(i as i32)]);
        }
        // chunks 0,1 sealed; chunk 2 full-but-writing (stays hot-only).

        // The runtime owner discovers the ring on its own and drains it.
        ColdCompactor::instance().apply(ColdRuntimeConfig {
            enabled: true,
            poll: Duration::from_millis(50),
            ..Default::default()
        });

        let mut waited = 0;
        while ColdCompactor::instance()
            .stats()
            .map(|s| s.segment_count)
            .unwrap_or(0)
            == 0
            && waited < 5000
        {
            std::thread::sleep(Duration::from_millis(50));
            waited += 50;
        }
        ColdCompactor::instance().stop(); // final flush seals the open segment
        assert!(
            ColdCompactor::instance()
                .stats()
                .map(|s| s.segment_count)
                .unwrap_or(0)
                >= 1,
            "compactor should have produced a cold segment"
        );

        // Query through the same hot∪cold provider the catalog builds.
        let path = self_dir().join("rt_demo");
        let mapped = MappedFile::open(&path).unwrap();
        let ring = RingMmapTable::try_new(mapped).unwrap();
        let provider: Arc<dyn TableProvider> =
            Arc::new(HotColdTable::new(ring, cold_dir(), "rt_demo"));
        let ctx = SessionContext::new();
        ctx.register_table("rt_demo", provider).unwrap();
        let all = ctx
            .sql("SELECT v FROM rt_demo ORDER BY v")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(collect_i32(&all), vec![1, 2, 3, 4, 5, 6]);

        drop(table);
        ColdCompactor::instance().stop();
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }

    #[tokio::test]
    async fn engine_catalog_query_unions_cold_tier() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
        use datafusion::catalog::MemoryCatalogProvider;
        use datafusion::prelude::SessionContext;
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        let schema = MtSchema::new()
            .col("timestamp", DType::I64)
            .col("v", DType::I32);
        let mut table = ExposedTable::create("metrics", &schema, 80, 8).unwrap();
        for i in 1i64..=6 {
            table.push_row(&[Value::I64(i * 100), Value::I32(i as i32)]);
        }

        // Drain the sealed chunks to cold via the runtime owner.
        ColdCompactor::instance().apply(ColdRuntimeConfig {
            enabled: true,
            poll: Duration::from_millis(50),
            ..Default::default()
        });
        let mut waited = 0;
        while ColdCompactor::instance()
            .stats()
            .map(|s| s.segment_count)
            .unwrap_or(0)
            == 0
            && waited < 5000
        {
            std::thread::sleep(Duration::from_millis(50));
            waited += 50;
        }
        ColdCompactor::instance().stop();

        // Real query path: register the dynamic catalog and resolve the table
        // purely by name — DynamicMmapCatalog → MmapFileSchemaProvider →
        // HotColdTable, exactly as the engine does.
        let ctx = SessionContext::new();
        let catalog = Arc::new(DynamicMmapCatalog {
            inner: Arc::new(MemoryCatalogProvider::new()),
        });
        ctx.register_catalog("probe", catalog);

        let all = ctx
            .sql("SELECT v FROM probe.memtable.metrics ORDER BY v")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(collect_i32(&all), vec![1, 2, 3, 4, 5, 6], "hot∪cold once");

        // One time predicate prunes across both tiers through the catalog.
        let span = ctx
            .sql(
                "SELECT v FROM probe.memtable.metrics \
                 WHERE timestamp >= 200 AND timestamp <= 500 ORDER BY v",
            )
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(collect_i32(&span), vec![2, 3, 4, 5]);

        drop(table);
        ColdCompactor::instance().stop();
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }

    #[test]
    fn classify_and_mmap_roundtrip() {
        assert_eq!(
            classify_mmap_basename("pulsing.actors"),
            Some(("pulsing".into(), "actors".into()))
        );
        assert_eq!(
            classify_mmap_basename("foo.bar.baz"),
            Some(("foo".into(), "bar.baz".into()))
        );
        assert_eq!(
            classify_mmap_basename("metrics"),
            Some((DEFAULT_UNDOTTED_SCHEMA.into(), "metrics".into()))
        );
        assert_eq!(
            mmap_filename_for(DEFAULT_UNDOTTED_SCHEMA, "metrics"),
            "metrics"
        );
        assert_eq!(mmap_filename_for("pulsing", "actors"), "pulsing.actors");
        assert_eq!(mmap_filename_for("foo", "bar.baz"), "foo.bar.baz");
    }

    #[test]
    fn mmap_table_exists_rejects_path_traversal() {
        assert!(!mmap_table_exists("memtable", "../../etc/passwd"));
        assert!(!mmap_table_exists("memtable", "a/b"));
        assert!(!mmap_table_exists("memtable", ""));
    }

    fn read_pushdown_from_mmap(schema: &str, table: &str) -> Arc<dyn TableProvider> {
        let path = self_dir().join(mmap_filename_for(schema, table));
        let mapped = MappedFile::open(path).unwrap();
        bytes_to_pushdown_table(mapped.as_bytes(), table)
    }

    #[test]
    fn namespace_list_and_mmap_read_via_exposed_table() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        let schema = MtSchema::new().col("ts", DType::I64).col("msg", DType::Str);
        let mut table = ExposedTable::create("test_metrics", &schema, 4096, 2).unwrap();
        {
            let mut w = table.writer();
            w.push_row(&[Value::I64(100), Value::Str("alpha")]);
            w.push_row(&[Value::I64(200), Value::Str("beta")]);
        }

        let names = tables_in_schema(DEFAULT_UNDOTTED_SCHEMA);
        assert!(
            names.contains(&"test_metrics".to_string()),
            "got: {names:?}"
        );
        assert!(mmap_table_exists(DEFAULT_UNDOTTED_SCHEMA, "test_metrics"));

        let provider = read_pushdown_from_mmap(DEFAULT_UNDOTTED_SCHEMA, "test_metrics");
        assert!(provider
            .as_any()
            .downcast_ref::<PluginAdvancedTable>()
            .is_some());

        let path = self_dir().join(mmap_filename_for(DEFAULT_UNDOTTED_SCHEMA, "test_metrics"));
        let mapped = MappedFile::open(&path).unwrap();
        let view = MemTableView::new(mapped.as_bytes()).unwrap();
        let batches = view_to_recordbatches(&view);
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);

        let ts = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(ts.value(0), 100);
        assert_eq!(ts.value(1), 200);

        let msgs: &datafusion::arrow::array::StringArray = batch.column(1).as_string();
        assert_eq!(msgs.value(0), "alpha");
        assert_eq!(msgs.value(1), "beta");

        drop(table);
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }

    #[test]
    fn dotted_schema_isolated_from_memtable_list() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        let schema = MtSchema::new().col("ts", DType::I64).col("msg", DType::Str);
        let dotted = mmap_filename_for("acme", "metrics_demo");
        let mut ring = ExposedTable::create(&dotted, &schema, 4096, 2).unwrap();
        {
            let mut w = ring.writer();
            w.push_row(&[Value::I64(1), Value::Str("x")]);
        }

        let mem_names = tables_in_schema(DEFAULT_UNDOTTED_SCHEMA);
        assert!(
            !mem_names.contains(&"metrics_demo".to_string()),
            "dotted file must not appear as memtable table: {mem_names:?}"
        );

        let acme_names = tables_in_schema("acme");
        assert!(
            acme_names.contains(&"metrics_demo".to_string()),
            "got: {acme_names:?}"
        );

        let provider = read_pushdown_from_mmap("acme", "metrics_demo");
        assert!(provider
            .as_any()
            .downcast_ref::<PluginAdvancedTable>()
            .is_some());

        drop(ring);
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }

    #[tokio::test]
    async fn merged_schema_provider_does_not_shadow_inner() {
        let _lock = PROBING_DATA_DIR_LOCK.lock().unwrap();
        use datafusion::catalog::MemorySchemaProvider;
        use datafusion::datasource::MemTable as DfMemTable;
        use probing_memtable::discover::ExposedTable;

        let tmp = tempfile::tempdir().unwrap();
        let orig = std::env::var("PROBING_DATA_DIR").ok();
        std::env::set_var("PROBING_DATA_DIR", tmp.path());

        // Static (inner) provider with one table
        let inner = Arc::new(MemorySchemaProvider::new());
        let static_schema = Arc::new(Schema::new(vec![Field::new(
            "x",
            DataType::Int64,
            false,
        )]));
        let static_batch = RecordBatch::try_new(
            static_schema.clone(),
            vec![Arc::new(Int64Array::from(vec![42i64]))],
        )
        .unwrap();
        inner
            .register_table(
                "static_tbl".to_string(),
                Arc::new(DfMemTable::try_new(static_schema, vec![vec![static_batch]]).unwrap()),
            )
            .unwrap();

        // Mmap table in schema "python"
        let mt_schema = MtSchema::new().col("v", DType::I64);
        let mut ring =
            ExposedTable::create(&mmap_filename_for("python", "extern_tbl"), &mt_schema, 4096, 2)
                .unwrap();
        ring.push_row(&[Value::I64(7)]);

        let merged = MmapFileSchemaProvider::with_inner("python", Some(inner.clone() as _));

        // Both tables visible
        let names = merged.table_names();
        assert!(names.contains(&"extern_tbl".to_string()), "got {names:?}");
        assert!(names.contains(&"static_tbl".to_string()), "got {names:?}");

        // Static table still resolvable through the merged provider
        assert!(merged.table("static_tbl").await.unwrap().is_some());
        // Mmap table resolvable too
        assert!(merged.table("extern_tbl").await.unwrap().is_some());
        assert!(merged.table_exist("static_tbl"));
        assert!(merged.table_exist("extern_tbl"));

        drop(ring);
        match orig {
            Some(v) => std::env::set_var("PROBING_DATA_DIR", v),
            None => std::env::remove_var("PROBING_DATA_DIR"),
        }
    }
}
