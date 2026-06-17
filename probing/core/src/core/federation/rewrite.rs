//! SQL rewrite helpers for the `probe` / `global` catalog split.

use super::convert::{PROBE_ADDR_COL, PROBE_HOST_COL, PROBE_RANK_COL};

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
/// Multi-table queries (JOIN, etc.) must still be broadcast to each node so joins
/// run locally per process; only single-relation scans can fan out via `global`.
pub fn can_fanout_via_global_catalog(sql: &str) -> bool {
    let lower = sql.to_lowercase();
    if lower.contains(" join ") {
        return false;
    }
    // Subqueries and unions need the legacy broadcast path for now.
    if lower.contains(" select ") && lower.matches("select").count() > 1 {
        return false;
    }
    if lower.contains(" union ") {
        return false;
    }
    true
}

fn references_global_catalog(sql: &str) -> bool {
    sql.to_lowercase().contains("global.")
}

/// Find the start of the top-level `FROM` clause (paren depth 0).
fn find_top_level_from(sql: &str) -> Option<usize> {
    let lower = sql.to_lowercase();
    let bytes = sql.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < sql.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && lower[i..].starts_with(" from ") => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn select_list_includes_wildcard(sql: &str) -> bool {
    let Some(from_idx) = find_top_level_from(sql) else {
        return false;
    };
    let lower = sql.to_lowercase();
    let Some(select_idx) = lower.find("select") else {
        return false;
    };
    let select_part = &sql[select_idx + "select".len()..from_idx];
    select_part.contains('*')
}

fn expand_global_select_star(sql: &str) -> String {
    let trimmed = sql.trim();
    let lower = trimmed.to_lowercase();
    if lower.contains(PROBE_HOST_COL)
        && lower.contains(PROBE_ADDR_COL)
        && lower.contains(PROBE_RANK_COL)
    {
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
        " EXCLUDE ({PROBE_HOST_COL}, {PROBE_ADDR_COL}, {PROBE_RANK_COL}), {PROBE_HOST_COL}, {PROBE_ADDR_COL}, {PROBE_RANK_COL}"
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
            "SELECT * EXCLUDE (_host, _addr, _rank), _host, _addr, _rank FROM global.process.envs"
        );
    }

    #[test]
    fn rewrites_qualified_select_star_with_probe_tags() {
        let sql = "SELECT e.* FROM global.process.envs e";
        assert_eq!(
            ensure_global_node_columns(sql),
            "SELECT e.* EXCLUDE (_host, _addr, _rank), _host, _addr, _rank FROM global.process.envs e"
        );
    }

    #[test]
    fn skips_select_star_wildcard_when_tags_already_present() {
        let sql = "SELECT * EXCLUDE (_host, _addr, _rank), _host, _addr, _rank FROM global.process.envs";
        assert_eq!(ensure_global_node_columns(sql), sql);
    }

    #[test]
    fn skips_qualified_select_star_when_already_expanded() {
        let sql = "SELECT e.* EXCLUDE (_host, _addr, _rank), _host, _addr, _rank FROM global.process.envs e";
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
