use std::sync::Arc;

use anyhow::{Context, Result};
use datafusion::sql::sqlparser::ast::{Expr, Set, Statement, Value};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;
use probing_proto::prelude::*;

use crate::extensions as se;
use probing_cc::extensions as cc;
#[cfg(feature = "gpu")]
use probing_gpu::extensions as gpu;
use probing_python::extensions as py;

use probing_core::config;

use crate::server::error::{ApiError, ApiResult};

use probing_core::core::federation::{
    reset_fanout_stats, take_fanout_stats, with_fanout_scope_async, FanoutScope,
};
use probing_core::core::UnifiedMemtableProbeDataSource;
pub use probing_core::ENGINE;
use probing_python::extensions::python::PythonProbeDataSource;

/// Composition root: wires L2 collectors/extensions into the engine.
/// NCCL/HCCL schema docs are registered here (not via `probing-core` default features).
pub async fn initialize_engine() -> Result<()> {
    probing_hccl_shim::register_docs();
    probing_nccl_profiler::register_docs();

    let builder = probing_core::create_engine()
        .with_data_source(cc::ClusterProbeDataSource::create("cluster", "nodes"))
        .with_data_source(cc::EnvProbeDataSource::create("process", "envs"))
        .with_data_source(cc::FilesProbeDataSource::create("files"))
        .with_extension(py::PprofProbeExtension::default())
        .with_extension(py::TorchProbeExtension::default())
        .with_extension(se::ServerProbeExtension::default())
        .with_extension(py::PythonExt::default())
        .with_data_source(PythonProbeDataSource::create("python"))
        .with_extension(crate::memtable_ext::MemTableProbeExtension::default())
        .with_data_source(Arc::new(UnifiedMemtableProbeDataSource))
        .with_extension(cc::CpuProbeExtension::default());

    #[cfg(feature = "gpu")]
    let builder = builder
        .with_data_source(gpu::GpuDevicesProbeDataSource::create("gpu", "devices"))
        .with_extension(gpu::GpuProbeExtension::default());

    #[cfg(target_os = "linux")]
    let builder = builder
        .with_extension(cc::RdmaProbeExtension::default())
        .with_data_source(cc::RdmaProbeDataSource::create("rdma", "mlx_hca"));

    // Kernel ring buffer (dmesg) — Linux only, requires the `kmsg` feature.
    #[cfg(all(target_os = "linux", feature = "kmsg"))]
    let builder = builder.with_data_source(cc::KMsgProbeDataSource::create("process", "kmsg"));

    let result = probing_core::initialize_engine(builder).await;
    if result.is_ok() {
        // Opt-in background hot→cold compaction (PROBING_COLD=on / SET memtable.cold_compaction).
        crate::memtable_ext::start_cold_compaction_from_env();
        cc::start_cpu_sampling_from_env();
        #[cfg(feature = "gpu")]
        gpu::start_gpu_sampling_from_env();
        crate::engine_lifecycle::mark_engine_ready();
    }
    result.map_err(anyhow::Error::new)
}

fn config_value_from_expr(expr: &Expr) -> String {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::SingleQuotedString(value)
            | Value::TripleSingleQuotedString(value)
            | Value::TripleDoubleQuotedString(value)
            | Value::EscapedStringLiteral(value)
            | Value::UnicodeStringLiteral(value) => value.clone(),
            _ => expr.to_string(),
        },
        _ => expr.to_string(),
    }
}

/// Parse an all-SET request into config assignments. Non-SET SQL returns `None`.
fn parse_config_assignments(expr: &str) -> Result<Option<Vec<(String, String)>>> {
    let statements = match Parser::parse_sql(&GenericDialect {}, expr) {
        Ok(statements) => statements,
        Err(_) => return Ok(None),
    };
    if !statements
        .iter()
        .any(|statement| matches!(statement, Statement::Set(_)))
    {
        return Ok(None);
    }
    if statements
        .iter()
        .any(|statement| !matches!(statement, Statement::Set(_)))
    {
        anyhow::bail!("SET statements cannot be mixed with query statements");
    }

    statements
        .into_iter()
        .map(|statement| match statement {
            Statement::Set(Set::SingleAssignment {
                variable, values, ..
            }) if values.len() == 1 => {
                Ok((variable.to_string(), config_value_from_expr(&values[0])))
            }
            _ => anyhow::bail!("only single-variable SET assignments are supported"),
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

/// Route extension SET knobs through `config::write` (`probing.<namespace>.*`).
async fn execute_set_via_config(key: &str, value: &str) -> Result<()> {
    let probe_key = if key.starts_with("probing.") {
        key.to_string()
    } else {
        format!("probing.{key}")
    };
    config::write(&probe_key, value).await?;
    Ok(())
}

pub async fn handle_query(request: Query) -> Result<QueryDataFormat> {
    if let Some(msg) = crate::engine_lifecycle::engine_not_ready_message() {
        return Err(anyhow::anyhow!(msg));
    }
    let Query { expr, opts: _ } = request;

    // We are already running within the Axum/Tokio runtime.

    if let Some(assignments) = parse_config_assignments(&expr)? {
        for (key, value) in assignments {
            log::debug!("Executing SET for config key: {key}");
            execute_set_via_config(&key, &value)
                .await
                .with_context(|| format!("Failed SET for config key '{key}'"))?;
            log::debug!("Successfully executed SET for config key: {key}");
        }
        return Ok(QueryDataFormat::Nil);
    }

    reset_fanout_stats();
    let engine = ENGINE.read().await;
    log::debug!("Executing SELECT query: {expr}");
    match engine.async_query(&expr).await {
        Ok(Some(dataframe)) => Ok(QueryDataFormat::DataFrame(dataframe)),
        Ok(None) => Ok(QueryDataFormat::Nil),
        Err(e) => {
            if is_missing_table_error(&e) {
                log::debug!("Optional table missing for SELECT '{expr}': {e}");
            } else {
                log::error!("Error executing SELECT query '{expr}': {e}");
            }
            Err(e.into())
        }
    }
}

/// Extension tables (NCCL profiler, optional GPU, etc.) may be absent on single-process jobs.
fn is_missing_table_error(err: &impl std::fmt::Display) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("not found") && msg.contains("table")
}

fn fanout_meta_from_stats(
    stats: probing_core::core::federation::FanoutStats,
) -> Option<serde_json::Value> {
    if stats.nodes_failed.is_empty() && stats.peer_batches_dropped == 0 {
        return None;
    }
    Some(serde_json::json!({
        "fanout": {
            "partial": true,
            "nodes_succeeded": stats.nodes_succeeded,
            "nodes_failed": stats.nodes_failed,
            "peer_batches_dropped": stats.peer_batches_dropped,
        }
    }))
}

fn query_response_partial(stats: &probing_core::core::federation::FanoutStats) -> bool {
    !stats.nodes_failed.is_empty() || stats.peer_batches_dropped > 0
}

/// Serialized `/query` body plus whether federated fan-out was partial.
pub struct QueryHttpEnvelope {
    pub body: String,
    pub partial: bool,
    pub error: bool,
}

// 处理Web API查询请求
pub async fn query(req: String) -> ApiResult<QueryHttpEnvelope> {
    with_fanout_scope_async(FanoutScope::Auto, query_in_fanout_context(req)).await
}

async fn query_in_fanout_context(req: String) -> ApiResult<QueryHttpEnvelope> {
    let request = serde_json::from_str::<Message<Query>>(&req);
    let request = match request {
        Ok(request) => request.payload,
        Err(err) => {
            log::error!("Failed to deserialize query request: {err}");
            return Err(ApiError::bad_request(format!(
                "Invalid request format: {err}"
            )));
        }
    };

    // Await the async handle_query function
    let reply_payload = match handle_query(request).await {
        Ok(reply) => reply,
        Err(err) => {
            // Error already logged in handle_query if it originated there
            QueryDataFormat::Error(QueryError {
                code: ErrorCode::Internal,
                message: format!("{err:#}"),
                details: None,
            })
        }
    };

    let error = matches!(&reply_payload, QueryDataFormat::Error(_));

    // Wrap the payload in a Message
    let stats = take_fanout_stats();
    let partial = query_response_partial(&stats);
    if partial {
        log::warn!(
            "query fan-out partial: nodes_succeeded={} nodes_failed={} peer_batches_dropped={}",
            stats.nodes_succeeded,
            stats.nodes_failed.len(),
            stats.peer_batches_dropped,
        );
    }
    let mut reply_message = Message::new(reply_payload);
    reply_message.meta = fanout_meta_from_stats(stats);

    // Serialize the response message
    let body = serde_json::to_string(&reply_message)
        .inspect_err(|e| log::error!("Failed to serialize query response: {e}"))
        .map_err(|e| ApiError::internal(format!("Failed to create response: {e}")))?;
    Ok(QueryHttpEnvelope {
        body,
        partial,
        error,
    })
}
