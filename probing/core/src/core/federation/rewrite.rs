//! SQL rewrite helpers for the `probe` / `global` catalog split.

use std::ops::ControlFlow;

use datafusion::sql::sqlparser::ast::{
    visit_relations_mut, Ident, ObjectName, ObjectNamePart, Query, SelectItem, SetExpr, Statement,
};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;

const KNOWN_SCHEMAS: &[&str] = &[
    "cluster", "process", "files", "python", "memtable", "gpu", "rdma",
];

/// Rewrite federated SQL so remote probing nodes execute against the local `probe` catalog.
pub fn rewrite_global_catalog_to_probe(sql: &str) -> String {
    rewrite_catalog_relations(sql, rewrite_relation_global_to_probe)
        .unwrap_or_else(|| rewrite_global_catalog_to_probe_legacy(sql))
}

/// Rewrite a user/cluster SQL string to reference the `global` catalog.
pub fn rewrite_sql_for_global_fanout(sql: &str) -> String {
    rewrite_catalog_relations(sql, rewrite_relation_to_global)
        .unwrap_or_else(|| rewrite_sql_for_global_fanout_legacy(sql))
}

fn rewrite_catalog_relations<F>(sql: &str, mut rewrite_fn: F) -> Option<String>
where
    F: FnMut(&mut ObjectName),
{
    let dialect = GenericDialect {};
    let mut stmts = Parser::parse_sql(&dialect, sql).ok()?;
    if stmts.is_empty() {
        return None;
    }
    for stmt in &mut stmts {
        let _ = visit_relations_mut(stmt, |name| {
            rewrite_fn(name);
            ControlFlow::<(), ()>::Continue(())
        });
    }
    Some(
        stmts
            .iter()
            .map(|stmt| stmt.to_string())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn relation_ident_parts(name: &ObjectName) -> Vec<String> {
    name.0
        .iter()
        .filter_map(|part| part.as_ident().map(|ident| ident.value.clone()))
        .collect()
}

fn set_relation_parts(name: &mut ObjectName, parts: &[&str]) {
    name.0 = parts
        .iter()
        .map(|part| ObjectNamePart::Identifier(Ident::new(*part)))
        .collect();
}

fn first_ident_eq(parts: &[String], expected: &str) -> bool {
    parts
        .first()
        .is_some_and(|part| part.eq_ignore_ascii_case(expected))
}

fn rewrite_relation_global_to_probe(name: &mut ObjectName) {
    let parts = relation_ident_parts(name);
    if !first_ident_eq(&parts, "global") {
        return;
    }
    let mut rewritten = vec!["probe"];
    rewritten.extend(parts.iter().skip(1).map(String::as_str));
    set_relation_parts(name, &rewritten);
}

fn rewrite_relation_to_global(name: &mut ObjectName) {
    let parts = relation_ident_parts(name);
    if parts.is_empty() || first_ident_eq(&parts, "global") {
        return;
    }
    if first_ident_eq(&parts, "probe") {
        let mut rewritten = vec!["global"];
        rewritten.extend(parts.iter().skip(1).map(String::as_str));
        set_relation_parts(name, &rewritten);
        return;
    }
    if parts.first().is_some_and(|part| {
        KNOWN_SCHEMAS
            .iter()
            .any(|schema| part.eq_ignore_ascii_case(schema))
    }) {
        let mut rewritten = vec!["global"];
        rewritten.extend(parts.iter().map(String::as_str));
        set_relation_parts(name, &rewritten);
    }
}

fn rewrite_global_catalog_to_probe_legacy(sql: &str) -> String {
    sql.replace("global.", "probe.")
        .replace("GLOBAL.", "probe.")
}

fn rewrite_sql_for_global_fanout_legacy(sql: &str) -> String {
    if sql.to_lowercase().contains("global.") {
        return sql.to_string();
    }

    let mut out = sql
        .replace("probe.", "global.")
        .replace("PROBE.", "global.");
    if out.to_lowercase().contains("global.") {
        return out;
    }

    for schema in KNOWN_SCHEMAS {
        for kw in ["FROM", "from", "JOIN", "join"] {
            let needle = format!("{kw} {schema}.");
            let replacement = format!("{kw} global.{schema}.");
            out = out.replace(&needle, &replacement);
        }
    }
    out
}

/// Whether the coordinator can execute this SQL via `global.*` table federation.
///
/// Multi-table queries (JOIN, comma joins, UNION, CTEs, subqueries) must still be
/// broadcast to each node so they run locally per process; only single-relation
/// scans can fan out via `global`. Detection is AST-based rather than substring
/// matching so SQL inside string literals or unusual whitespace cannot change the
/// routing decision. Anything that fails to parse (or is not a single `SELECT`)
/// conservatively falls back to the broadcast path, which is always correct.
pub fn can_fanout_via_global_catalog(sql: &str) -> bool {
    match parse_single_query(sql) {
        Some(query) => query_is_single_relation_scan(&query),
        None => false,
    }
}

fn parse_single_query(sql: &str) -> Option<Query> {
    let dialect = GenericDialect {};
    let mut stmts = Parser::parse_sql(&dialect, sql).ok()?;
    if stmts.len() != 1 {
        return None;
    }
    match stmts.remove(0) {
        Statement::Query(query) => Some(*query),
        _ => None,
    }
}

fn query_is_single_relation_scan(query: &Query) -> bool {
    // CTEs introduce additional relations that must be resolved per node.
    if query.with.is_some() {
        return false;
    }
    let SetExpr::Select(select) = query.body.as_ref() else {
        // UNION / EXCEPT / INTERSECT / VALUES / INSERT ...
        return false;
    };
    // Comma-separated relations (implicit joins) or explicit JOINs.
    if select.from.len() != 1 {
        return false;
    }
    if !select.from[0].joins.is_empty() {
        return false;
    }
    !query_contains_subquery(query)
}

/// Detect nested relations (scalar/IN/EXISTS subqueries) on the parsed AST.
///
/// Inspecting the debug rendering of the AST keeps this resilient to literals and
/// formatting: only actual `Expr::Subquery` / `Expr::Exists` / `Expr::InSubquery`
/// nodes render with these markers, whereas a string literal such as `'subquery'`
/// renders as a `Value` node and is unaffected.
fn query_contains_subquery(query: &Query) -> bool {
    let rendered = format!("{:?}", query.body);
    rendered.contains("Subquery") || rendered.contains("Exists")
}

fn references_global_catalog(sql: &str) -> bool {
    sql.to_lowercase().contains("global.")
}

fn select_list_includes_wildcard(query: &Query) -> bool {
    let SetExpr::Select(select) = query.body.as_ref() else {
        return false;
    };
    select.projection.iter().any(|item| {
        matches!(
            item,
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _)
        )
    })
}

fn expand_global_select_star(_sql: &str) -> String {
    // Federation tags are appended by `GlobalFederatedTable` / `FederatedScanExec`
    // (`federated_output_schema`). DataFusion does not support `SELECT * EXCLUDE (...)`.
    _sql.to_string()
}

/// Ensure `global.*` `SELECT *` queries expose which node each row came from.
///
/// Tag columns are injected at the table-provider layer; this pass intentionally
/// does not rewrite SQL (avoids breaking `count(*)` and unsupported EXCLUDE syntax).
pub fn ensure_global_node_columns(sql: &str) -> String {
    let trimmed = sql.trim();
    if !references_global_catalog(trimmed) {
        return sql.to_string();
    }
    if !trimmed.to_lowercase().starts_with("select") {
        return sql.to_string();
    }
    if let Some(query) = parse_single_query(trimmed) {
        if select_list_includes_wildcard(&query) {
            return expand_global_select_star(trimmed);
        }
    }
    sql.to_string()
}

/// Prepare a user SQL string for execution against the `global` catalog.
pub fn prepare_global_query(sql: &str) -> String {
    ensure_global_node_columns(sql)
}

#[cfg(test)]
mod tests {
    use super::super::convert::{PROBE_ADDR_COL, PROBE_RANK_COL};
    use super::*;

    #[test]
    fn rewrites_global_prefix_to_probe() {
        let sql = "SELECT * FROM global.cluster.nodes WHERE rank = 1";
        assert_eq!(
            rewrite_global_catalog_to_probe(sql),
            "SELECT * FROM probe.cluster.nodes WHERE rank = 1"
        );
    }

    #[test]
    fn rewrites_probe_prefix_to_global() {
        let sql = "SELECT * FROM probe.python.metrics LIMIT 5";
        assert_eq!(
            rewrite_sql_for_global_fanout(sql),
            "SELECT * FROM global.python.metrics LIMIT 5"
        );
    }

    #[test]
    fn rewrites_unqualified_schema_to_global() {
        let sql = "SELECT rank FROM python.comm_collective LIMIT 20";
        assert_eq!(
            rewrite_sql_for_global_fanout(sql),
            "SELECT rank FROM global.python.comm_collective LIMIT 20"
        );
    }

    #[test]
    fn join_queries_use_legacy_broadcast() {
        let sql = "SELECT a.x FROM python.a JOIN python.b ON a.id = b.id";
        assert!(!can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn single_table_queries_use_global_catalog() {
        let sql = "SELECT rank FROM python.comm_collective LIMIT 20";
        assert!(can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn newline_join_uses_legacy_broadcast() {
        // Substring matching on " join " would miss this; AST parsing does not.
        let sql = "SELECT a.x\nFROM python.a\nJOIN python.b ON a.id = b.id";
        assert!(!can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn comma_join_uses_legacy_broadcast() {
        let sql = "SELECT a.x FROM python.a, python.b WHERE a.id = b.id";
        assert!(!can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn union_uses_legacy_broadcast() {
        let sql = "SELECT rank FROM python.a UNION SELECT rank FROM python.b";
        assert!(!can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn cte_uses_legacy_broadcast() {
        let sql = "WITH t AS (SELECT rank FROM python.a) SELECT rank FROM t";
        assert!(!can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn subquery_uses_legacy_broadcast() {
        let sql = "SELECT rank FROM python.a WHERE rank > (SELECT max(rank) FROM python.a)";
        assert!(!can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn join_keyword_in_string_literal_still_fans_out() {
        // The literal contains "join" but the query is a genuine single-table scan.
        let sql = "SELECT name FROM python.metrics WHERE name = 'inner join demo'";
        assert!(can_fanout_via_global_catalog(sql));
    }

    #[test]
    fn unparseable_sql_falls_back_to_broadcast() {
        assert!(!can_fanout_via_global_catalog("this is not sql"));
    }

    #[test]
    fn ensure_global_node_columns_handles_non_ascii_without_panic() {
        let sql = "SELECT * FROM global.process.envs WHERE 名称 = '值'";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn count_star_aggregate_is_not_treated_as_select_star() {
        let sql = "SELECT _rank, op, avg(duration_ms) AS avg_ms, count(*) AS n \
                    FROM global.python.comm_collective GROUP BY _rank, op ORDER BY avg_ms DESC LIMIT 8";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn leaves_explicit_global_select_unchanged() {
        let sql = "SELECT rank FROM global.python.metrics WHERE step > 1 LIMIT 5";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn leaves_explicit_name_select_unchanged() {
        let sql = "SELECT name FROM global.process.envs";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn select_star_global_query_is_unchanged() {
        let sql = "SELECT * FROM global.process.envs";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn qualified_select_star_global_query_is_unchanged() {
        let sql = "SELECT e.* FROM global.process.envs e";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn skips_non_global_queries() {
        let sql = "SELECT rank FROM probe.python.metrics";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn prepare_global_query_pipeline() {
        let user = "SELECT rank FROM python.comm_collective WHERE rank > 0 LIMIT 10";
        let global_sql = rewrite_sql_for_global_fanout(user);
        let prepared = prepare_global_query(&global_sql);
        assert!(prepared.contains("global.python.comm_collective"));
        assert!(!prepared.contains(PROBE_ADDR_COL));
        assert!(!prepared.contains(PROBE_RANK_COL));
    }

    #[test]
    fn prepare_global_query_expands_select_star() {
        let user = "SELECT * FROM python.comm_collective WHERE rank > 0 LIMIT 10";
        let global_sql = rewrite_sql_for_global_fanout(user);
        let prepared = prepare_global_query(&global_sql);
        assert_eq!(
            prepared,
            "SELECT * FROM global.python.comm_collective WHERE rank > 0 LIMIT 10"
        );
    }

    #[test]
    fn global_fanout_does_not_rewrite_catalog_prefix_in_string_literals() {
        let sql = "SELECT 'probe.python.secret' AS note FROM python.metrics LIMIT 1";
        let out = rewrite_sql_for_global_fanout(sql);
        assert!(out.contains("'probe.python.secret'"));
        assert!(out.contains("global.python.metrics"));
        assert!(!out.contains("'global.python.secret'"));
    }

    #[test]
    fn global_to_probe_does_not_rewrite_catalog_prefix_in_string_literals() {
        let sql = "SELECT 'global.python.metrics' AS note FROM global.python.metrics LIMIT 1";
        let out = rewrite_global_catalog_to_probe(sql);
        assert!(out.contains("'global.python.metrics'"));
        assert!(out.contains("probe.python.metrics"));
        assert!(!out.contains("'probe.python.metrics'"));
    }

    #[test]
    fn global_fanout_still_rewrites_unqualified_table_when_literal_mentions_global() {
        let sql = "SELECT name FROM python.metrics WHERE name = 'see global.python.b'";
        let out = rewrite_sql_for_global_fanout(sql);
        assert_eq!(
            out,
            "SELECT name FROM global.python.metrics WHERE name = 'see global.python.b'"
        );
    }
}
