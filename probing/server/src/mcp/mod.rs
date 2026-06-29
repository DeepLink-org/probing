//! Model Context Protocol (MCP) surface mounted on the probing HTTP server.
//!
//! Phase 1: read-only tools (`query`, skills).
//! Phase 2: schema grounding (`describe_tables`, resources), cluster tools, gated write tools.

mod helpers;

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use probing_cli::cli::skill::{
    build_context, default_parameters, expand_template, list_skill_ids, load_skill, run_skill_json,
    SkillStep,
};
use probing_proto::prelude::{Query, QueryDataFormat};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ListResourcesResult, PaginatedRequestParams, RawResource,
    ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerInfo,
};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::RequestContext;
use rmcp::tool;
use rmcp::tool_router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::RoleServer;
use rmcp::ServerHandler;
use serde::Deserialize;

use crate::engine::handle_query;
use helpers::{
    allow_mcp_write, audit_mcp_write, engine_query_json, ensure_read_only_sql, escape_sql_string,
    local_ctrl, require_write_permission, tool_error, validate_config_key, DEFAULT_ROW_LIMIT,
    MAX_ROW_LIMIT,
};

const SCHEMA_RESOURCE_PREFIX: &str = "probing://schema/";

/// Nest the MCP Streamable HTTP service under `/mcp`.
pub fn router() -> Router {
    let service = StreamableHttpService::new(
        || Ok(ProbingMcp::new()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    Router::new().nest_service("/mcp", service)
}

#[derive(Debug, Clone)]
pub struct ProbingMcp {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl ProbingMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for ProbingMcp {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryArgs {
    sql: String,
    #[serde(default = "default_row_limit")]
    limit: usize,
}

fn default_row_limit() -> usize {
    DEFAULT_ROW_LIMIT
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DescribeTablesArgs {
    /// Optional `table_schema` filter (e.g. `python`, `gpu`, `nccl`).
    schema: Option<String>,
    /// Optional `table_name` filter.
    table: Option<String>,
    #[serde(default = "default_row_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PlanSkillArgs {
    skill_id: String,
    #[serde(default)]
    params: HashMap<String, String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunSkillArgs {
    skill_id: String,
    #[serde(default)]
    params: HashMap<String, String>,
    #[serde(default)]
    use_global: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClusterQueryArgs {
    sql: String,
    #[serde(default = "default_true")]
    cluster: bool,
    #[serde(default = "default_row_limit")]
    limit: usize,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetConfigArgs {
    /// Config key (`probing.sample_rate` or `sample_rate`).
    key: String,
    value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EvalPythonArgs {
    /// Python source executed in the target training process.
    code: String,
}

#[tool_router]
impl ProbingMcp {
    #[tool(
        description = "Run a read-only SQL query against the in-process probing engine. Returns JSON rows bounded by `limit`."
    )]
    async fn query(
        &self,
        Parameters(QueryArgs { sql, limit }): Parameters<QueryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let limit = limit.clamp(1, MAX_ROW_LIMIT);
        let json = engine_query_json(sql, limit).await?;
        Ok(text_result(json.to_string()))
    }

    #[tool(
        description = "List semantic table documentation from `probe.probing.table_docs` and `column_docs` for agent grounding."
    )]
    async fn describe_tables(
        &self,
        Parameters(DescribeTablesArgs {
            schema,
            table,
            limit,
        }): Parameters<DescribeTablesArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let limit = limit.clamp(1, MAX_ROW_LIMIT);
        let mut table_sql = String::from(
            "SELECT table_schema, table_name, description, synonyms \
             FROM probe.probing.table_docs",
        );
        let mut col_sql = String::from(
            "SELECT table_schema, table_name, column_name, data_type, description \
             FROM probe.probing.column_docs",
        );
        let mut clauses = Vec::new();
        if let Some(schema) = schema.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            clauses.push(format!("table_schema = '{}'", escape_sql_string(schema)));
        }
        if let Some(table) = table.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            clauses.push(format!("table_name = '{}'", escape_sql_string(table)));
        }
        if !clauses.is_empty() {
            let where_clause = format!(" WHERE {}", clauses.join(" AND "));
            table_sql.push_str(&where_clause);
            col_sql.push_str(&where_clause);
        }
        table_sql.push_str(" ORDER BY table_schema, table_name");
        col_sql.push_str(" ORDER BY table_schema, table_name, column_name");

        let tables = engine_query_json(table_sql, limit).await?;
        let columns = engine_query_json(col_sql, limit.saturating_mul(4)).await?;
        let payload = serde_json::json!({
            "tables": tables,
            "columns": columns,
        });
        Ok(text_result(payload.to_string()))
    }

    #[tool(description = "List bundled diagnostic skills (id, title, category).")]
    fn list_skills(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut skills = Vec::new();
        for id in list_skill_ids() {
            match load_skill(&id) {
                Ok(skill) => skills.push(serde_json::json!({
                    "id": skill.id,
                    "title": skill.title,
                    "category": skill.category,
                    "description": skill.docs.lines().next().unwrap_or("").trim(),
                })),
                Err(err) => {
                    log::warn!("MCP list_skills: skip {id}: {err}");
                }
            }
        }
        Ok(text_result(
            serde_json::to_string_pretty(&skills).unwrap_or_else(|_| "[]".to_string()),
        ))
    }

    #[tool(description = "Expand a diagnostic skill into SQL/API steps without executing them.")]
    fn plan_skill(
        &self,
        Parameters(PlanSkillArgs { skill_id, params }): Parameters<PlanSkillArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let skill = load_skill(&skill_id).map_err(|e| tool_error(e.to_string()))?;
        let mut merged = default_parameters(&skill);
        merged.extend(params);
        let ctx = build_context(&skill, &merged);
        let steps: Vec<serde_json::Value> = skill
            .steps
            .iter()
            .map(|step| step_to_plan_json(step, &ctx))
            .collect();
        let payload = serde_json::json!({
            "skill_id": skill.id,
            "title": skill.title,
            "parameters": merged,
            "steps": steps,
            "next_steps": skill.next_steps,
        });
        Ok(text_result(payload.to_string()))
    }

    #[tool(description = "Run a diagnostic skill against the local probing process.")]
    async fn run_skill(
        &self,
        Parameters(RunSkillArgs {
            skill_id,
            mut params,
            use_global,
        }): Parameters<RunSkillArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if let Some(global) = use_global {
            params.insert("use_global".to_string(), global.to_string());
        }
        match run_skill_json(local_ctrl(), &skill_id, params).await {
            Ok(value) => Ok(text_result(value.to_string())),
            Err(err) => Err(tool_error(err.to_string())),
        }
    }

    #[tool(description = "List registered cluster nodes from `GET /apis/nodes`.")]
    async fn list_cluster_nodes(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let ctrl = local_ctrl();
        let body = ctrl
            .get("/apis/nodes")
            .await
            .map_err(|e| tool_error(e.to_string()))?;
        Ok(text_result(body))
    }

    #[tool(
        description = "Run read-only SQL with optional cluster fan-out via `POST /apis/cluster/query`."
    )]
    async fn cluster_query(
        &self,
        Parameters(ClusterQueryArgs {
            sql,
            cluster,
            limit,
        }): Parameters<ClusterQueryArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let limit = limit.clamp(1, MAX_ROW_LIMIT);
        ensure_read_only_sql(&sql).map_err(tool_error)?;
        let body = serde_json::json!({
            "expr": sql,
            "cluster": cluster,
        });
        let reply = local_ctrl()
            .post_json("/apis/cluster/query", &body.to_string())
            .await
            .map_err(|e| tool_error(e.to_string()))?;
        let value: serde_json::Value =
            serde_json::from_str(&reply).map_err(|e| tool_error(e.to_string()))?;
        if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
            return Err(tool_error(err.to_string()));
        }
        let df = value
            .get("dataframe")
            .ok_or_else(|| tool_error("missing dataframe in cluster response"))?;
        let dataframe: probing_proto::prelude::DataFrame =
            serde_json::from_value(df.clone()).map_err(|e| tool_error(e.to_string()))?;
        let rows = helpers::truncate_dataframe_json(&dataframe, limit);
        let payload = serde_json::json!({
            "dataframe": rows,
            "meta": value.get("meta"),
        });
        Ok(text_result(payload.to_string()))
    }

    #[tool(
        description = "Update a probing config key via `SET probing.* = …`. Disabled unless PROBING_MCP_ALLOW_WRITE=1."
    )]
    async fn set_config(
        &self,
        Parameters(SetConfigArgs { key, value }): Parameters<SetConfigArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        require_write_permission("set_config")?;
        validate_config_key(&key).map_err(tool_error)?;
        let probe_key = if key.starts_with("probing.") {
            key
        } else {
            format!("probing.{key}")
        };
        let stmt = format!("set {}='{}'", probe_key, escape_sql_string(&value));
        audit_mcp_write("set_config", &stmt);
        handle_query(Query {
            expr: stmt,
            opts: None,
        })
        .await
        .map_err(|e| tool_error(e.to_string()))?;
        Ok(text_result(format!(
            "{{\"status\":\"ok\",\"key\":\"{probe_key}\",\"write_enabled\":true}}"
        )))
    }

    #[tool(
        description = "Execute Python in the target training process (`POST /apis/pythonext/eval`). Disabled unless PROBING_MCP_ALLOW_WRITE=1."
    )]
    async fn eval_python(
        &self,
        Parameters(EvalPythonArgs { code }): Parameters<EvalPythonArgs>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        require_write_permission("eval_python")?;
        audit_mcp_write("eval_python", &format!("code_len={}", code.len()));
        let body = local_ctrl()
            .eval_json(code)
            .await
            .map_err(|e| tool_error(e.to_string()))?;
        Ok(text_result(body))
    }
}

impl ServerHandler for ProbingMcp {
    fn get_info(&self) -> ServerInfo {
        let write_note = if allow_mcp_write() {
            "Write tools (`set_config`, `eval_python`) are enabled."
        } else {
            "Write tools are disabled; set PROBING_MCP_ALLOW_WRITE=1 to enable intervention."
        };
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            "probing",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(format!(
            "Agent-native distributed training diagnostics. \
             Read: `list_skills`, `plan_skill`, `run_skill`, `query`, `describe_tables`, \
             `list_cluster_nodes`, `cluster_query`. \
             Schema resources: `{SCHEMA_RESOURCE_PREFIX}{{schema}}/{{table}}`. {write_note}"
        ))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        let sql = "SELECT table_schema, table_name, description \
                   FROM probe.probing.table_docs \
                   ORDER BY table_schema, table_name \
                   LIMIT 500";
        let reply = handle_query(Query {
            expr: sql.to_string(),
            opts: None,
        })
        .await
        .map_err(|e| tool_error(e.to_string()))?;
        let QueryDataFormat::DataFrame(df) = reply else {
            return Ok(ListResourcesResult {
                resources: vec![catalog_resource()],
                meta: None,
                next_cursor: None,
            });
        };

        let mut resources = vec![catalog_resource()];
        for row in df.iter() {
            if row.len() < 2 {
                continue;
            }
            let schema = cell_as_str(&row[0]);
            let table = cell_as_str(&row[1]);
            let description = row.get(2).map(cell_as_str).unwrap_or_default();
            if schema.is_empty() || table.is_empty() {
                continue;
            }
            let uri = format!("{SCHEMA_RESOURCE_PREFIX}{schema}/{table}");
            resources.push(Resource::new(
                RawResource {
                    uri,
                    name: format!("{schema}.{table}"),
                    title: Some(format!("{schema}.{table}")),
                    description: if description.is_empty() {
                        None
                    } else {
                        Some(description)
                    },
                    mime_type: Some("application/json".to_string()),
                    size: None,
                    icons: None,
                    meta: None,
                },
                None,
            ));
        }
        Ok(ListResourcesResult {
            resources,
            meta: None,
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        let uri = request.uri.clone();
        let text = if uri == format!("{SCHEMA_RESOURCE_PREFIX}catalog") {
            catalog_resource_body().await?
        } else if let Some(rest) = uri.strip_prefix(SCHEMA_RESOURCE_PREFIX) {
            let (schema, table) = rest
                .split_once('/')
                .ok_or_else(|| tool_error(format!("invalid schema resource URI: {uri}")))?;
            table_resource_body(schema, table).await?
        } else {
            return Err(tool_error(format!("unknown resource URI: {uri}")));
        };
        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: request.uri,
                mime_type: Some("application/json".to_string()),
                text,
                meta: None,
            },
        ]))
    }
}

fn catalog_resource() -> Resource {
    Resource::new(
        RawResource {
            uri: format!("{SCHEMA_RESOURCE_PREFIX}catalog"),
            name: "schema-catalog".to_string(),
            title: Some("Probing schema catalog".to_string()),
            description: Some("Summary of all documented tables".to_string()),
            mime_type: Some("application/json".to_string()),
            size: None,
            icons: None,
            meta: None,
        },
        None,
    )
}

async fn catalog_resource_body() -> Result<String, rmcp::ErrorData> {
    let json = engine_query_json(
        "SELECT table_schema, table_name, description, synonyms \
         FROM probe.probing.table_docs ORDER BY table_schema, table_name LIMIT 500"
            .to_string(),
        500,
    )
    .await?;
    Ok(json.to_string())
}

async fn table_resource_body(schema: &str, table: &str) -> Result<String, rmcp::ErrorData> {
    let table_sql = format!(
        "SELECT table_schema, table_name, description, synonyms \
         FROM probe.probing.table_docs \
         WHERE table_schema = '{}' AND table_name = '{}'",
        escape_sql_string(schema),
        escape_sql_string(table)
    );
    let col_sql = format!(
        "SELECT column_name, data_type, description \
         FROM probe.probing.column_docs \
         WHERE table_schema = '{}' AND table_name = '{}' \
         ORDER BY column_name",
        escape_sql_string(schema),
        escape_sql_string(table)
    );
    let tables = engine_query_json(table_sql, 1).await?;
    let columns = engine_query_json(col_sql, 500).await?;
    Ok(serde_json::json!({ "table": tables, "columns": columns }).to_string())
}

fn cell_as_str(ele: &probing_proto::prelude::Ele) -> String {
    match ele {
        probing_proto::prelude::Ele::Text(s) | probing_proto::prelude::Ele::Url(s) => s.clone(),
        other => other.to_string(),
    }
}

fn step_to_plan_json(step: &SkillStep, ctx: &HashMap<String, String>) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": step.id,
        "title": step.title,
        "type": step.step_type,
        "on_empty": step.on_empty,
    });
    if let Some(sql) = &step.sql {
        value["sql"] = serde_json::Value::String(expand_template(sql, ctx));
    }
    if let Some(path) = &step.path {
        value["path"] = serde_json::Value::String(path.clone());
    }
    if let Some(when) = &step.when {
        value["when"] = serde_json::Value::String(when.clone());
    }
    value
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text)])
}

#[cfg(test)]
mod tests {
    use super::helpers::ensure_read_only_sql;

    #[test]
    fn ensure_read_only_sql_accepts_select() {
        assert!(ensure_read_only_sql("SELECT 1").is_ok());
    }

    #[test]
    fn ensure_read_only_sql_rejects_set() {
        assert!(ensure_read_only_sql("SET probing.sample_rate=0.1").is_err());
    }
}
