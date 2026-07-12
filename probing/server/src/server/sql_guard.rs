//! Read-only SQL validation for HTTP and MCP query surfaces.

use datafusion::sql::sqlparser::ast::{Query, SetExpr, Statement};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;

/// Reject write statements and multi-statement batches containing writes.
pub fn ensure_read_only_sql(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("SQL must not be empty".to_string());
    }

    let dialect = GenericDialect {};
    let statements =
        Parser::parse_sql(&dialect, trimmed).map_err(|e| format!("invalid SQL: {e}"))?;
    if statements.is_empty() {
        return Err("SQL must not be empty".to_string());
    }

    for stmt in &statements {
        if !statement_is_read_only(stmt) {
            return Err(
                "Only read-only SQL is allowed (SELECT/WITH/SHOW/DESCRIBE/EXPLAIN)".to_string(),
            );
        }
    }
    Ok(())
}

fn statement_is_read_only(stmt: &Statement) -> bool {
    match stmt {
        Statement::Query(query) => query_is_read_only(query),
        Statement::Explain { .. } | Statement::ExplainTable { .. } => true,
        Statement::ShowTables { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowStatus { .. } => true,
        _ => false,
    }
}

fn query_is_read_only(query: &Query) -> bool {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            if !query_is_read_only(&cte.query) {
                return false;
            }
        }
    }
    set_expr_is_read_only(query.body.as_ref())
}

fn set_expr_is_read_only(expr: &SetExpr) -> bool {
    match expr {
        SetExpr::Select(_) | SetExpr::Values(_) | SetExpr::Table(_) => true,
        SetExpr::Query(query) => query_is_read_only(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_is_read_only(left) && set_expr_is_read_only(right)
        }
        SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) => false,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_select_and_with() {
        assert!(ensure_read_only_sql("SELECT 1").is_ok());
        assert!(ensure_read_only_sql("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
        assert!(ensure_read_only_sql("SELECT 1; SELECT 2").is_ok());
    }

    #[test]
    fn rejects_trailing_write() {
        assert!(ensure_read_only_sql("SELECT 1; DELETE FROM t").is_err());
        assert!(ensure_read_only_sql("SET probing.sample_rate=0.1").is_err());
    }

    #[test]
    fn rejects_write_inside_with_cte() {
        assert!(ensure_read_only_sql("WITH x AS (DELETE FROM python.t) SELECT 1").is_err());
    }
}
