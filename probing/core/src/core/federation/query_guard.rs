//! Guardrails for federated / global-catalog queries (row and byte budgets).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use datafusion::error::{DataFusionError, Result};
use datafusion::sql::sqlparser::ast::{Expr, LimitClause, Query, Statement};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;
use probing_proto::prelude::{DataFrame, Seq};

use super::fanout_scope::{current_federation_limits, FederationLimits};
use super::rewrite::{prepare_global_query, rewrite_sql_for_global_fanout};
use super::route::{classify_federated_sql, FederatedQueryPath};

pub const FEDERATION_RESPONSE_BUDGET_HEADER: &str = "x-probing-response-max-bytes";
pub const FEDERATION_MEMORY_BUDGET_HEADER: &str = "x-probing-memory-max-bytes";

const GLOBAL_SCAN_MAX_ROWS_ENV: &str = "PROBING_GLOBAL_SCAN_MAX_ROWS";
const GLOBAL_RESPONSE_MAX_BYTES_ENV: &str = "PROBING_GLOBAL_RESPONSE_MAX_BYTES";
const GLOBAL_MEMORY_MAX_BYTES_ENV: &str = "PROBING_GLOBAL_MEMORY_MAX_BYTES";
const REQUIRE_BROADCAST_LIMIT_ENV: &str = "PROBING_REQUIRE_BROADCAST_LIMIT";
const DEFAULT_GLOBAL_SCAN_MAX_ROWS: usize = 10_000;
const DEFAULT_GLOBAL_RESPONSE_MAX_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_GLOBAL_MEMORY_MAX_BYTES: usize = 128 * 1024 * 1024;

fn positive_env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(default)
}

/// Max rows materialized for a federated query without an explicit LIMIT.
pub fn global_scan_max_rows() -> usize {
    positive_env_usize(GLOBAL_SCAN_MAX_ROWS_ENV, DEFAULT_GLOBAL_SCAN_MAX_ROWS)
}

/// Maximum serialized body accepted from one federation peer.
pub fn global_response_max_bytes() -> usize {
    if let Some(limits) = current_federation_limits() {
        return limits.response_max_bytes.min(limits.memory_max_bytes);
    }
    configured_global_response_max_bytes().min(configured_global_memory_max_bytes())
}

fn configured_global_response_max_bytes() -> usize {
    positive_env_usize(
        GLOBAL_RESPONSE_MAX_BYTES_ENV,
        DEFAULT_GLOBAL_RESPONSE_MAX_BYTES,
    )
}

/// Maximum cumulative bytes materialized by one federated query.
pub fn global_memory_max_bytes() -> usize {
    current_federation_limits()
        .map(|limits| limits.memory_max_bytes)
        .unwrap_or_else(configured_global_memory_max_bytes)
}

fn configured_global_memory_max_bytes() -> usize {
    positive_env_usize(GLOBAL_MEMORY_MAX_BYTES_ENV, DEFAULT_GLOBAL_MEMORY_MAX_BYTES)
}

/// Clamp caller-provided limits to this node's configured maxima.
pub fn federation_limits_from_request(
    response_max_bytes: Option<usize>,
    memory_max_bytes: Option<usize>,
) -> FederationLimits {
    let configured_memory = configured_global_memory_max_bytes();
    let memory_max_bytes = memory_max_bytes
        .filter(|&value| value > 0)
        .unwrap_or(configured_memory)
        .min(configured_memory);
    let configured_response = configured_global_response_max_bytes().min(configured_memory);
    let response_max_bytes = response_max_bytes
        .filter(|&value| value > 0)
        .unwrap_or(configured_response)
        .min(configured_response)
        .min(memory_max_bytes);
    FederationLimits {
        response_max_bytes,
        memory_max_bytes,
    }
}

/// Per-query cumulative materialization budget shared by fan-out workers.
#[derive(Debug, Clone)]
pub struct FederationMemoryBudget {
    max: usize,
    consumed: Arc<AtomicUsize>,
}

impl Default for FederationMemoryBudget {
    fn default() -> Self {
        Self::new(global_memory_max_bytes())
    }
}

impl FederationMemoryBudget {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            consumed: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Reserve bytes permanently for the lifetime of this query.
    ///
    /// Cumulative accounting is intentionally conservative: streaming may
    /// release an earlier batch, but total work still cannot exceed the cap.
    pub fn try_consume(&self, bytes: usize, context: &str) -> Result<()> {
        let update = self
            .consumed
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(bytes).filter(|&next| next <= self.max)
            });
        match update {
            Ok(_) => Ok(()),
            Err(current) => Err(DataFusionError::ResourcesExhausted(format!(
                "federated query memory budget exceeded while {context}: consumed {current} + {bytes} bytes (max {}, set {GLOBAL_MEMORY_MAX_BYTES_ENV})",
                self.max
            ))),
        }
    }

    #[cfg(test)]
    fn consumed(&self) -> usize {
        self.consumed.load(Ordering::Acquire)
    }
}

/// Reject a peer body before JSON deserialization or coordinator merge.
pub fn cap_federated_response_bytes(bytes: usize, context: &str) -> Result<()> {
    cap_federated_response_bytes_with_limit(bytes, global_response_max_bytes(), context)
}

fn cap_federated_response_bytes_with_limit(bytes: usize, max: usize, context: &str) -> Result<()> {
    if bytes > max {
        Err(DataFusionError::ResourcesExhausted(format!(
            "federated response exceeded byte budget while {context}: {bytes} bytes (max {max}, set {GLOBAL_RESPONSE_MAX_BYTES_ENV})"
        )))
    } else {
        Ok(())
    }
}

/// Conservative heap footprint for a protocol DataFrame.
pub fn proto_dataframe_memory_bytes(df: &DataFrame) -> usize {
    let names = df.names.iter().fold(
        df.names
            .capacity()
            .saturating_mul(std::mem::size_of::<String>()),
        |total, name| total.saturating_add(name.capacity()),
    );
    let columns = df.cols.iter().fold(
        df.cols
            .capacity()
            .saturating_mul(std::mem::size_of::<Seq>()),
        |total, column| {
            let bytes = match column {
                Seq::SeqText(values) => values.iter().fold(
                    values
                        .capacity()
                        .saturating_mul(std::mem::size_of::<String>()),
                    |text_total, value| text_total.saturating_add(value.capacity()),
                ),
                Seq::SeqBOOL(values) => values.capacity(),
                Seq::SeqI32(values) => values.capacity().saturating_mul(std::mem::size_of::<i32>()),
                Seq::SeqI64(values) => values.capacity().saturating_mul(std::mem::size_of::<i64>()),
                Seq::SeqF32(values) => values.capacity().saturating_mul(std::mem::size_of::<f32>()),
                Seq::SeqF64(values) => values.capacity().saturating_mul(std::mem::size_of::<f64>()),
                Seq::SeqDateTime(values) => {
                    values.capacity().saturating_mul(std::mem::size_of::<u64>())
                }
                Seq::Nil => 0,
            };
            total.saturating_add(bytes)
        },
    );
    std::mem::size_of::<DataFrame>()
        .saturating_add(names)
        .saturating_add(columns)
}

/// Safety clamp for concurrent peer bodies under the query memory budget.
pub fn budgeted_fanout_concurrency(configured: usize) -> usize {
    budgeted_fanout_concurrency_for(
        configured,
        global_memory_max_bytes(),
        global_response_max_bytes(),
    )
}

fn budgeted_fanout_concurrency_for(
    configured: usize,
    memory_max: usize,
    response_max: usize,
) -> usize {
    let by_memory = memory_max.checked_div(response_max).unwrap_or(0).max(1);
    configured.max(1).min(by_memory)
}

/// When true (default), broadcast paths (JOIN / CTE / UNION) require LIMIT.
pub fn require_broadcast_limit() -> bool {
    !matches!(
        std::env::var(REQUIRE_BROADCAST_LIMIT_ENV)
            .ok()
            .as_deref()
            .map(str::trim),
        Some("0") | Some("false") | Some("FALSE") | Some("off") | Some("OFF")
    )
}

fn parse_single_query(sql: &str) -> Option<Box<Query>> {
    let dialect = GenericDialect {};
    let mut stmts = Parser::parse_sql(&dialect, sql).ok()?;
    if stmts.len() != 1 {
        return None;
    }
    match stmts.pop()? {
        Statement::Query(query) => Some(query),
        _ => None,
    }
}

fn expr_as_usize(expr: &Expr) -> Option<usize> {
    expr.to_string().parse().ok()
}

fn limit_clause_value(clause: &LimitClause) -> Option<usize> {
    match clause {
        LimitClause::LimitOffset {
            limit: Some(limit), ..
        }
        | LimitClause::OffsetCommaLimit { limit, .. } => expr_as_usize(limit),
        LimitClause::LimitOffset { limit: None, .. } => None,
    }
}

fn fetch_value(query: &Query) -> Option<usize> {
    let fetch = query.fetch.as_ref()?;
    if fetch.percent || fetch.with_ties {
        return None;
    }
    fetch.quantity.as_ref().map_or(Some(1), expr_as_usize)
}

/// Whether one SQL statement has a statically bounded top-level LIMIT/FETCH.
pub fn sql_has_limit(sql: &str) -> bool {
    let Some(query) = parse_single_query(sql) else {
        return false;
    };
    query
        .limit_clause
        .as_ref()
        .and_then(limit_clause_value)
        .is_some()
        || fetch_value(&query).is_some()
}

fn federated_path(sql: &str) -> FederatedQueryPath {
    let global_sql = prepare_global_query(&rewrite_sql_for_global_fanout(sql));
    classify_federated_sql(&global_sql)
}

/// Reject cross-node broadcast fan-out SQL without LIMIT.
///
/// Only call before cluster fan-out (`cluster=true`); local probe-catalog queries
/// (including JOINs on `python.*`) are allowed without LIMIT and skip row cap.
pub fn validate_global_query(sql: &str) -> Result<()> {
    let path = federated_path(sql);
    if path != FederatedQueryPath::Local && parse_single_query(sql).is_none() {
        return Err(DataFusionError::Plan(
            "federated queries must contain exactly one SELECT statement".into(),
        ));
    }
    match path {
        FederatedQueryPath::Local => Ok(()),
        FederatedQueryPath::Broadcast if require_broadcast_limit() && !sql_has_limit(sql) => {
            Err(DataFusionError::Plan(
                "broadcast federated query (JOIN/CTE/UNION) requires an explicit LIMIT clause \
                 — unbounded cross-node materialization is disabled"
                    .into(),
            ))
        }
        _ => Ok(()),
    }
}

fn max_limit_clause(max: usize) -> Option<LimitClause> {
    parse_single_query(&format!("SELECT 1 LIMIT {max}"))?.limit_clause
}

fn clamp_top_level_limit(sql: &str, max: usize) -> String {
    let Some(mut query) = parse_single_query(sql) else {
        return sql.to_string();
    };
    let limit_ok = query
        .limit_clause
        .as_ref()
        .and_then(limit_clause_value)
        .is_some_and(|limit| limit <= max);
    let fetch_ok = query.fetch.is_some() && fetch_value(&query).is_some_and(|limit| limit <= max);
    if limit_ok || fetch_ok {
        return sql.to_string();
    }

    query.fetch = None;
    if let Some(clause) = max_limit_clause(max) {
        query.limit_clause = Some(match (query.limit_clause.take(), clause) {
            (
                Some(LimitClause::LimitOffset {
                    offset, limit_by, ..
                }),
                LimitClause::LimitOffset { limit, .. },
            ) => LimitClause::LimitOffset {
                limit,
                offset,
                limit_by,
            },
            (
                Some(LimitClause::OffsetCommaLimit { offset, .. }),
                LimitClause::LimitOffset {
                    limit: Some(limit), ..
                },
            ) => LimitClause::OffsetCommaLimit { offset, limit },
            (_, clause) => clause,
        });
        query.to_string()
    } else {
        format!(
            "SELECT * FROM ({}) AS __probing_limited LIMIT {max}",
            sql.trim_end_matches(';')
        )
    }
}

/// Enforce a coordinator-side maximum before federated materialization.
pub fn ensure_global_scan_limit(sql: &str) -> String {
    match federated_path(sql) {
        FederatedQueryPath::Local => sql.to_string(),
        FederatedQueryPath::Broadcast if !sql_has_limit(sql) => sql.to_string(),
        _ => clamp_top_level_limit(sql, global_scan_max_rows()),
    }
}

/// Fail when a federated query materializes more rows than allowed.
pub fn cap_materialized_rows(sql: &str, row_count: usize) -> Result<()> {
    // Local probe-catalog queries (no global.*) stay on-node; skip federated row cap.
    if !sql.to_lowercase().contains("global.") {
        return Ok(());
    }
    if federated_path(sql) == FederatedQueryPath::Local {
        return Ok(());
    }
    let max = global_scan_max_rows();
    if row_count > max {
        Err(DataFusionError::ResourcesExhausted(format!(
            "federated query materialized {row_count} rows (max {max}, set PROBING_GLOBAL_SCAN_MAX_ROWS)"
        )))
    } else {
        Ok(())
    }
}

/// Fail when a federated query's retained result exceeds its memory budget.
pub fn cap_materialized_memory(sql: &str, bytes: usize) -> Result<()> {
    if !sql.to_lowercase().contains("global.") || federated_path(sql) == FederatedQueryPath::Local {
        return Ok(());
    }
    let max = global_memory_max_bytes();
    if bytes > max {
        Err(DataFusionError::ResourcesExhausted(format!(
            "federated query materialized {bytes} bytes (max {max}, set {GLOBAL_MEMORY_MAX_BYTES_ENV})"
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_limit_clause() {
        assert!(sql_has_limit("SELECT 1 LIMIT 5"));
        assert!(!sql_has_limit("SELECT 1"));
    }

    #[test]
    fn only_accepts_limit_on_single_top_level_query() {
        assert!(!sql_has_limit(
            "(SELECT * FROM python.a LIMIT 1) UNION SELECT * FROM python.b"
        ));
        assert!(!sql_has_limit("SELECT 1; SELECT 2 LIMIT 1"));
    }

    #[test]
    fn broadcast_without_limit_rejected() {
        let sql = "SELECT a.x FROM python.a JOIN python.b ON a.id = b.id";
        assert!(validate_global_query(sql).is_err());
    }

    #[test]
    fn broadcast_nested_limit_does_not_bypass_guard() {
        let sql = "(SELECT * FROM python.a LIMIT 1) UNION SELECT * FROM python.b";
        assert!(validate_global_query(sql).is_err());
        assert!(
            validate_global_query("SELECT * FROM python.a; SELECT * FROM python.b LIMIT 1")
                .is_err()
        );
    }

    #[test]
    fn scan_without_limit_gets_cap() {
        let sql = "SELECT rank FROM python.comm_collective";
        let capped = ensure_global_scan_limit(sql);
        assert!(capped.contains("LIMIT"));
    }

    #[test]
    fn oversized_explicit_limit_is_clamped_before_execution() {
        let max = global_scan_max_rows();
        let capped = ensure_global_scan_limit(&format!(
            "SELECT rank FROM python.comm_collective LIMIT {}",
            max + 1
        ));
        assert_eq!(
            capped,
            format!("SELECT rank FROM python.comm_collective LIMIT {max}")
        );

        let small = ensure_global_scan_limit("SELECT rank FROM python.comm_collective LIMIT 7");
        assert_eq!(small, "SELECT rank FROM python.comm_collective LIMIT 7");
    }

    #[test]
    fn local_python_scan_skips_row_cap() {
        let sql = "SELECT module, stage FROM python.torch_trace WHERE stage LIKE 'post %'";
        assert!(cap_materialized_rows(sql, 1_000_000).is_ok());
    }

    #[test]
    fn local_python_join_skips_row_cap() {
        let sql = "SELECT post.module FROM python.torch_trace pre \
                   INNER JOIN python.torch_trace post ON pre.local_step = post.local_step";
        assert!(cap_materialized_rows(sql, 1_000_000).is_ok());
    }

    #[test]
    fn global_federated_scan_row_cap_enforced() {
        let sql = "SELECT rank FROM global.comm_collective";
        let over = global_scan_max_rows() + 1;
        assert!(cap_materialized_rows(sql, over).is_err());
        assert!(cap_materialized_rows(sql, 1).is_ok());
    }

    #[test]
    fn memory_budget_is_atomic_and_never_overcommits() {
        let budget = FederationMemoryBudget::new(10);
        let first = budget.clone();
        let second = budget.clone();
        let a = std::thread::spawn(move || first.try_consume(6, "first peer"));
        let b = std::thread::spawn(move || second.try_consume(6, "second peer"));
        let accepted =
            usize::from(a.join().unwrap().is_ok()) + usize::from(b.join().unwrap().is_ok());
        assert_eq!(accepted, 1);
        assert_eq!(budget.consumed(), 6);
    }

    #[test]
    fn response_bytes_and_concurrency_are_bounded_independently_of_rows() {
        assert!(cap_federated_response_bytes_with_limit(1024, 1024, "peer").is_ok());
        assert!(cap_federated_response_bytes_with_limit(1025, 1024, "peer").is_err());
        assert_eq!(budgeted_fanout_concurrency_for(128, 64, 16), 4);
        assert_eq!(budgeted_fanout_concurrency_for(128, 8, 16), 1);
        assert_eq!(budgeted_fanout_concurrency_for(2, 64, 16), 2);

        let narrowed = federation_limits_from_request(Some(8), Some(16));
        assert_eq!(narrowed.response_max_bytes, 8);
        assert_eq!(narrowed.memory_max_bytes, 16);
        let attempted_increase = federation_limits_from_request(Some(usize::MAX), Some(usize::MAX));
        assert!(attempted_increase.response_max_bytes <= configured_global_response_max_bytes());
        assert!(attempted_increase.memory_max_bytes <= configured_global_memory_max_bytes());
    }

    #[tokio::test]
    async fn request_limits_survive_nested_hierarchical_scope() {
        let limits = FederationLimits {
            response_max_bytes: 8,
            memory_max_bytes: 16,
        };
        super::super::fanout_scope::with_federation_limits_async(limits, async {
            super::super::fanout_scope::with_fanout_scope_async(
                super::super::fanout_scope::FanoutScope::Node,
                async {
                    assert_eq!(global_response_max_bytes(), 8);
                    assert_eq!(global_memory_max_bytes(), 16);
                },
            )
            .await;
        })
        .await;
    }

    #[test]
    fn wide_text_dataframe_is_charged_by_bytes_not_rows() {
        let df = DataFrame::new(
            vec!["payload".into()],
            vec![Seq::SeqText(vec!["x".repeat(4096)])],
        );
        assert_eq!(df.len(), 1);
        assert!(proto_dataframe_memory_bytes(&df) >= 4096);

        let budget = FederationMemoryBudget::new(1024);
        assert!(budget
            .try_consume(proto_dataframe_memory_bytes(&df), "wide peer row")
            .is_err());
        assert!(cap_materialized_memory(
            "SELECT payload FROM global.python.events",
            global_memory_max_bytes() + 1,
        )
        .is_err());
        assert!(cap_materialized_memory("SELECT payload FROM python.events", usize::MAX,).is_ok());
    }
}
