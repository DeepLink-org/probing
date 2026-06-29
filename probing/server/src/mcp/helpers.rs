//! Shared helpers for MCP tools (query shaping, write guard, audit).

use probing_cli::cli::ctrl::ProbeEndpoint;
use probing_proto::prelude::{Ele, Query, QueryDataFormat};
use rmcp::ErrorData;

use crate::engine::handle_query;

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

pub fn local_ctrl() -> ProbeEndpoint {
    ProbeEndpoint::Local {
        pid: std::process::id() as i32,
    }
}

pub fn tool_error(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}

pub fn ensure_read_only_sql(sql: &str) -> Result<(), String> {
    let upper = sql.trim().to_uppercase();
    if upper.starts_with("SELECT")
        || upper.starts_with("WITH")
        || upper.starts_with("SHOW")
        || upper.starts_with("DESCRIBE")
    {
        Ok(())
    } else {
        Err("Only read-only SQL is allowed (SELECT/WITH/SHOW/DESCRIBE)".to_string())
    }
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

pub async fn engine_query_json(sql: String, limit: usize) -> Result<serde_json::Value, ErrorData> {
    ensure_read_only_sql(&sql).map_err(tool_error)?;
    let reply = handle_query(Query {
        expr: sql,
        opts: None,
    })
    .await
    .map_err(|e| tool_error(e.to_string()))?;
    dataframe_reply_to_json(reply, limit)
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
}
