//! AST-based read-only SQL validation shared by skill loading and execution.

use sqlparser::ast::{Query, SetExpr, Statement};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

pub(crate) fn ensure_read_only_sql(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("SQL must not be empty".to_string());
    }

    let statements = Parser::parse_sql(&GenericDialect {}, trimmed)
        .map_err(|error| format!("invalid SQL: {error}"))?;
    if statements.is_empty() {
        return Err("SQL must not be empty".to_string());
    }
    if statements.iter().all(statement_is_read_only) {
        Ok(())
    } else {
        Err("Only read-only SQL is allowed (SELECT/WITH/SHOW/DESCRIBE/EXPLAIN)".to_string())
    }
}

fn statement_is_read_only(statement: &Statement) -> bool {
    match statement {
        Statement::Query(query) => query_is_read_only(query),
        Statement::Explain { .. } | Statement::ExplainTable { .. } => true,
        Statement::ShowFunctions { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCatalogs { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowProcessList { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowObjects(_)
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowCollation { .. } => true,
        _ => false,
    }
}

fn query_is_read_only(query: &Query) -> bool {
    query.with.as_ref().is_none_or(|with| {
        with.cte_tables
            .iter()
            .all(|cte| query_is_read_only(&cte.query))
    }) && set_expr_is_read_only(query.body.as_ref())
}

fn set_expr_is_read_only(expression: &SetExpr) -> bool {
    match expression {
        SetExpr::Select(_) | SetExpr::Values(_) | SetExpr::Table(_) => true,
        SetExpr::Query(query) => query_is_read_only(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_is_read_only(left) && set_expr_is_read_only(right)
        }
        SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_read_only_statements() {
        assert!(ensure_read_only_sql("SELECT 1").is_ok());
        assert!(ensure_read_only_sql("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
        assert!(ensure_read_only_sql("SHOW TABLES; DESCRIBE python.t").is_ok());
    }

    #[test]
    fn rejects_write_hidden_after_read() {
        assert!(ensure_read_only_sql("SELECT 1; DELETE FROM python.t").is_err());
        assert!(ensure_read_only_sql("SELECT 1; SET probing.x=1").is_err());
    }

    #[test]
    fn rejects_write_cte() {
        assert!(ensure_read_only_sql("WITH x AS (DELETE FROM python.t) SELECT 1").is_err());
    }
}
