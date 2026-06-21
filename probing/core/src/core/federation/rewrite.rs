//! SQL rewrite helpers for the `probe` / `global` catalog split.

use datafusion::sql::sqlparser::ast::{Query, SetExpr, Statement};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;

use super::convert::FEDERATION_TAG_COLUMNS;

const KNOWN_SCHEMAS: &[&str] = &[
    "cluster", "process", "files", "python", "memtable", "gpu", "rdma",
];

/// Rewrite federated SQL so remote probing nodes execute against the local `probe` catalog.
pub fn rewrite_global_catalog_to_probe(sql: &str) -> String {
    sql.replace("global.", "probe.")
        .replace("GLOBAL.", "probe.")
}

/// Rewrite a user/cluster SQL string to reference the `global` catalog.
pub fn rewrite_sql_for_global_fanout(sql: &str) -> String {
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

/// Find the start of the top-level ` FROM ` clause (paren depth 0).
///
/// Iterates by `char` so all indexing stays on UTF-8 boundaries (SQL may contain
/// multi-byte identifiers or string literals), and matches the keyword
/// case-insensitively on raw bytes to avoid `to_lowercase()` length skew.
fn find_top_level_from(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut depth = 0i32;
    for (i, ch) in sql.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && byte_window_eq_ci(bytes, i, b" from ") => return Some(i),
            _ => {}
        }
    }
    None
}

/// Case-insensitive comparison of `needle` against the byte window starting at `at`.
fn byte_window_eq_ci(bytes: &[u8], at: usize, needle: &[u8]) -> bool {
    bytes
        .get(at..at + needle.len())
        .is_some_and(|window| window.eq_ignore_ascii_case(needle))
}

fn select_list_includes_wildcard(sql: &str) -> bool {
    match find_top_level_from(sql) {
        // `from_idx` is a char boundary, so slicing the select list is panic-safe.
        Some(from_idx) => sql[..from_idx].contains('*'),
        None => false,
    }
}

fn federation_tags_already_expanded(lower: &str) -> bool {
    FEDERATION_TAG_COLUMNS.iter().all(|col| lower.contains(col))
}

fn federation_tag_exclude_list() -> String {
    FEDERATION_TAG_COLUMNS
        .iter()
        .map(|col| col.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn expand_global_select_star(sql: &str) -> String {
    let trimmed = sql.trim();
    let lower = trimmed.to_lowercase();
    if federation_tags_already_expanded(&lower) {
        return sql.to_string();
    }
    let Some(from_idx) = find_top_level_from(trimmed) else {
        return sql.to_string();
    };
    let select_part = &trimmed[..from_idx];
    let from_part = &trimmed[from_idx..];
    if !select_part.contains('*') {
        return sql.to_string();
    }

    let exclude = format!(
        " EXCLUDE ({tags}), {tags}",
        tags = federation_tag_exclude_list()
    );
    let new_select = if let Some(dot_star) = select_part.rfind(".*") {
        let before = &select_part[..dot_star + 2];
        let after = &select_part[dot_star + 2..];
        format!("{before}{exclude}{after}")
    } else {
        select_part.replacen('*', &format!("*{exclude}"), 1)
    };
    format!("{new_select}{from_part}")
}

/// Ensure `global.*` `SELECT *` queries expose which node each row came from.
///
/// Rewrites `SELECT *` so the logical projection always includes `_host`,
/// `_addr` and `_rank`. Explicit column lists are left unchanged.
pub fn ensure_global_node_columns(sql: &str) -> String {
    let trimmed = sql.trim();
    let lower = trimmed.to_lowercase();
    if !references_global_catalog(trimmed) {
        return sql.to_string();
    }
    if !lower.starts_with("select") {
        return sql.to_string();
    }
    if select_list_includes_wildcard(trimmed) {
        return expand_global_select_star(trimmed);
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
        // Multi-byte identifiers/literals must not cause byte-boundary panics.
        let sql = "SELECT * FROM global.process.envs WHERE 名称 = '值'";
        let rewritten = ensure_global_node_columns(sql);
        assert!(rewritten.contains("EXCLUDE"));
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
    fn rewrites_select_star_with_exclude_and_probe_tags() {
        let sql = "SELECT * FROM global.process.envs";
        assert_eq!(
            ensure_global_node_columns(sql),
            "SELECT * EXCLUDE (_host, _addr, _rank, _node_rank, _local_rank, _role), _host, _addr, _rank, _node_rank, _local_rank, _role FROM global.process.envs"
        );
    }

    #[test]
    fn rewrites_qualified_select_star_with_probe_tags() {
        let sql = "SELECT e.* FROM global.process.envs e";
        assert_eq!(
            ensure_global_node_columns(sql),
            "SELECT e.* EXCLUDE (_host, _addr, _rank, _node_rank, _local_rank, _role), _host, _addr, _rank, _node_rank, _local_rank, _role FROM global.process.envs e"
        );
    }

    #[test]
    fn skips_select_star_wildcard_when_tags_already_present() {
        let sql = "SELECT * EXCLUDE (_host, _addr, _rank, _node_rank, _local_rank, _role), _host, _addr, _rank, _node_rank, _local_rank, _role FROM global.process.envs";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn skips_qualified_select_star_when_already_expanded() {
        let sql = "SELECT e.* EXCLUDE (_host, _addr, _rank, _node_rank, _local_rank, _role), _host, _addr, _rank, _node_rank, _local_rank, _role FROM global.process.envs e";
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
        assert!(prepared.contains("EXCLUDE"));
        assert!(prepared.contains(PROBE_ADDR_COL));
        assert!(prepared.contains(PROBE_RANK_COL));
    }
}
