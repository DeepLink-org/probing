//! Guardrails for federated / global-catalog queries (LIMIT, row caps).

use datafusion::error::{DataFusionError, Result};
use datafusion::sql::sqlparser::ast::{Query, SetExpr, Statement};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;

use super::rewrite::{prepare_global_query, rewrite_sql_for_global_fanout};
use super::route::{classify_federated_sql, FederatedQueryPath};

const GLOBAL_SCAN_MAX_ROWS_ENV: &str = "PROBING_GLOBAL_SCAN_MAX_ROWS";
const REQUIRE_BROADCAST_LIMIT_ENV: &str = "PROBING_REQUIRE_BROADCAST_LIMIT";
const DEFAULT_GLOBAL_SCAN_MAX_ROWS: usize = 10_000;

/// Max rows materialized for a federated query without an explicit LIMIT.
pub fn global_scan_max_rows() -> usize {
    std::env::var(GLOBAL_SCAN_MAX_ROWS_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_GLOBAL_SCAN_MAX_ROWS)
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

/// Whether the SQL contains a top-level LIMIT (or OFFSET) on any SELECT.
pub fn sql_has_limit(sql: &str) -> bool {
    let dialect = GenericDialect {};
    let Ok(stmts) = Parser::parse_sql(&dialect, sql) else {
        return false;
    };
    stmts.iter().any(stmt_has_limit)
}

fn stmt_has_limit(stmt: &Statement) -> bool {
    match stmt {
        Statement::Query(q) => query_has_limit(q),
        _ => false,
    }
}

fn query_has_limit(q: &Query) -> bool {
    if q.limit_clause.is_some() || q.fetch.is_some() {
        return true;
    }
    match q.body.as_ref() {
        SetExpr::Select(_) => false,
        SetExpr::Query(inner) => query_has_limit(inner),
        SetExpr::SetOperation { left, right, .. } => {
            matches!(left.as_ref(), SetExpr::Query(q) if query_has_limit(q))
                || matches!(right.as_ref(), SetExpr::Query(q) if query_has_limit(q))
        }
        _ => false,
    }
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
    match federated_path(sql) {
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

/// Append a coordinator-side LIMIT for single-table federated scans missing LIMIT.
pub fn ensure_global_scan_limit(sql: &str) -> String {
    if sql_has_limit(sql) {
        return sql.to_string();
    }
    match federated_path(sql) {
        FederatedQueryPath::Local | FederatedQueryPath::Broadcast => sql.to_string(),
        _ => format!(
            "{} LIMIT {}",
            sql.trim_end_matches(';'),
            global_scan_max_rows()
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_limit_clause() {
        assert!(sql_has_limit("SELECT 1 LIMIT 5"));
        assert!(!sql_has_limit("SELECT 1"));
    }

    #[test]
    fn broadcast_without_limit_rejected() {
        let sql = "SELECT a.x FROM python.a JOIN python.b ON a.id = b.id";
        assert!(validate_global_query(sql).is_err());
    }

    #[test]
    fn scan_without_limit_gets_cap() {
        let sql = "SELECT rank FROM python.comm_collective";
        let capped = ensure_global_scan_limit(sql);
        assert!(capped.contains("LIMIT"));
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
}
