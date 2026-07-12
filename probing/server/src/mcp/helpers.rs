//! Shared helpers for MCP tools (query shaping, write guard, audit).

use std::collections::HashMap;

use probing_core::core::federation::{fanout_strict_enabled, take_fanout_stats};
use probing_core::core::ProbeExtensionManager;
use probing_proto::prelude::{Ele, Query, QueryDataFormat};
use rmcp::ErrorData;

use crate::engine::{handle_query, ENGINE};

pub const DEFAULT_ROW_LIMIT: usize = 200;
pub const MAX_ROW_LIMIT: usize = 1000;

pub fn allow_mcp_write() -> bool {
    matches!(
        std::env::var("PROBING_MCP_ALLOW_WRITE")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1")
            | Some("true")
            | Some("TRUE")
            | Some("on")
            | Some("ON")
            | Some("yes")
            | Some("YES")
    )
}

pub fn require_write_permission(tool: &str) -> Result<(), ErrorData> {
    if allow_mcp_write() {
        Ok(())
    } else {
        Err(tool_error(format!(
            "tool `{tool}` is disabled by default; set PROBING_MCP_ALLOW_WRITE=1 to enable intervention tools"
        )))
    }
}

pub fn audit_mcp_write(tool: &str, detail: &str) {
    log::info!("MCP write audit: tool={tool} detail={detail}");
}

pub fn tool_error(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}

/// Map a fallible error into MCP `ErrorData` while preserving anyhow cause chains (`{err:#}`).
pub fn tool_error_from(err: impl std::fmt::Display) -> ErrorData {
    tool_error(format!("{err:#}"))
}

pub fn validate_config_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("config key must not be empty".to_string());
    }
    if key.contains('=') || key.contains(';') || key.contains('\'') || key.contains('"') {
        return Err("config key contains invalid characters".to_string());
    }
    Ok(())
}

pub fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

pub(crate) fn split_api_path(path: &str) -> (&str, HashMap<String, String>) {
    let path = path.strip_prefix("/apis").unwrap_or(path);
    if let Some((p, q)) = path.split_once('?') {
        (p, serde_urlencoded::from_str(q).unwrap_or_default())
    } else {
        (path, HashMap::new())
    }
}

/// Call a registered probe extension in-process (no HTTP loopback).
pub(crate) async fn extension_request(path: &str, body: &[u8]) -> Result<Vec<u8>, ErrorData> {
    let (api_path, params) = split_api_path(path);
    let eem = {
        let engine = ENGINE.read().await;
        engine
            .context
            .state()
            .config()
            .options()
            .extensions
            .get::<ProbeExtensionManager>()
            .cloned()
    };
    let Some(eem) = eem else {
        return Err(tool_error("Extension manager not available"));
    };
    eem.call(api_path, &params, body)
        .await
        .map_err(tool_error_from)
}

pub async fn engine_query_json(sql: String, limit: usize) -> Result<serde_json::Value, ErrorData> {
    crate::server::sql_guard::ensure_read_only_sql(&sql).map_err(tool_error)?;
    let reply = handle_query(Query {
        expr: sql,
        opts: None,
    })
    .await
    .map_err(tool_error_from)?;
    let mut payload = dataframe_reply_to_json(reply, limit)?;
    attach_fanout_quality(&mut payload)?;
    Ok(payload)
}

/// Surface federated / global-table fan-out completeness when peers were dropped.
fn attach_fanout_quality(payload: &mut serde_json::Value) -> Result<(), ErrorData> {
    let stats = take_fanout_stats();
    if stats.nodes_failed.is_empty() && stats.peer_batches_dropped == 0 {
        return Ok(());
    }
    if fanout_strict_enabled() {
        return Err(tool_error(format!(
            "federated fan-out strict mode: {} node(s) failed, {} peer batch(es) dropped",
            stats.nodes_failed.len(),
            stats.peer_batches_dropped
        )));
    }
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "fanout".to_string(),
            serde_json::json!({
                "partial": true,
                "nodes_succeeded": stats.nodes_succeeded,
                "nodes_failed": stats.nodes_failed,
                "peer_batches_dropped": stats.peer_batches_dropped,
            }),
        );
    }
    Ok(())
}

pub fn dataframe_reply_to_json(
    reply: QueryDataFormat,
    limit: usize,
) -> Result<serde_json::Value, ErrorData> {
    match reply {
        QueryDataFormat::DataFrame(df) => Ok(truncate_dataframe_json(&df, limit)),
        QueryDataFormat::Nil => Ok(serde_json::json!({
            "columns": [],
            "rows": [],
            "row_count": 0,
            "truncated": false,
        })),
        QueryDataFormat::TimeSeries(_) => Err(tool_error(
            "TimeSeries responses are not supported by MCP query tools",
        )),
        QueryDataFormat::Error(err) => Err(tool_error(format!("{:?}: {}", err.code, err.message))),
    }
}

pub fn truncate_dataframe_json(
    df: &probing_proto::prelude::DataFrame,
    limit: usize,
) -> serde_json::Value {
    let total = df.len();
    let take = total.min(limit);
    let mut rows = Vec::with_capacity(take);
    for row in df.iter().take(take) {
        let mut obj = serde_json::Map::new();
        for (name, cell) in df.names.iter().zip(row.iter()) {
            obj.insert(name.clone(), ele_to_json(cell));
        }
        rows.push(serde_json::Value::Object(obj));
    }
    serde_json::json!({
        "columns": df.names,
        "rows": rows,
        "row_count": take,
        "total_rows": total,
        "truncated": total > take,
    })
}

pub fn ele_to_json(ele: &Ele) -> serde_json::Value {
    match ele {
        Ele::Nil => serde_json::Value::Null,
        Ele::BOOL(v) => serde_json::Value::Bool(*v),
        Ele::I32(v) => serde_json::json!(v),
        Ele::I64(v) => serde_json::json!(v),
        Ele::F32(v) => serde_json::json!(v),
        Ele::F64(v) => serde_json::json!(v),
        Ele::Text(v) | Ele::Url(v) => serde_json::Value::String(v.clone()),
        Ele::DataTime(v) => serde_json::json!(v),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_disabled_by_default() {
        std::env::remove_var("PROBING_MCP_ALLOW_WRITE");
        assert!(!allow_mcp_write());
    }

    #[test]
    fn write_enabled_with_env() {
        std::env::set_var("PROBING_MCP_ALLOW_WRITE", "1");
        assert!(allow_mcp_write());
        std::env::remove_var("PROBING_MCP_ALLOW_WRITE");
    }

    #[test]
    fn ensure_read_only_sql_rejects_trailing_write() {
        use crate::server::sql_guard::ensure_read_only_sql;
        assert!(ensure_read_only_sql("SELECT 1; DELETE FROM t").is_err());
    }

    #[test]
    fn ensure_read_only_sql_allows_multiple_selects() {
        use crate::server::sql_guard::ensure_read_only_sql;
        assert!(ensure_read_only_sql("SELECT 1; SELECT 2").is_ok());
    }

    #[test]
    fn tool_error_from_preserves_anyhow_chain() {
        let err = anyhow::anyhow!("root cause")
            .context("middle")
            .context("outer");
        let data = tool_error_from(err);
        let msg = data.message;
        assert!(msg.contains("outer"));
        assert!(msg.contains("root cause"));
    }
}
