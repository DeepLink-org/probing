use std::sync::Arc;

use anyhow::{Context, Result};
use probing_proto::prelude::*;

use crate::extensions as se;
use probing_cc::extensions as cc;
#[cfg(feature = "gpu")]
use probing_gpu::extensions as gpu;
use probing_python::extensions as py;

use probing_core::config;

use crate::server::error::{ApiError, ApiResult};

use probing_core::core::federation::{reset_fanout_stats, take_fanout_stats};
use probing_core::core::UnifiedMemtableProbeDataSource;
pub use probing_core::ENGINE;
use probing_python::extensions::python::PythonProbeDataSource;

pub async fn initialize_engine() -> Result<()> {
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
    // Opt-in background hot→cold compaction (PROBING_COLD=on / SET memtable.cold_compaction).
    crate::memtable_ext::start_cold_compaction_from_env();
    if result.is_ok() {
        cc::start_cpu_sampling_from_env();
        #[cfg(feature = "gpu")]
        gpu::start_gpu_sampling_from_env();
        crate::engine_lifecycle::mark_engine_ready();
    }
    result.map_err(anyhow::Error::new)
}

/// Parse `SET key = value` (value may be quoted).
fn parse_set_assignment(stmt: &str) -> Option<(&str, &str)> {
    let mut s = stmt.trim();
    if s.len() >= 3 && s.as_bytes()[..3].eq_ignore_ascii_case(b"set") {
        s = s[3..].trim_start();
    } else {
        return None;
    }
    let eq = s.find('=')?;
    let key = s[..eq].trim();
    if key.is_empty() {
        return None;
    }
    let mut value = s[eq + 1..].trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
        {
            value = &value[1..value.len() - 1];
        }
    }
    Some((key, value))
}

fn is_set_expr(expr: &str) -> bool {
    expr.split(';').any(|part| {
        let p = part.trim();
        p.len() >= 3 && p.as_bytes()[..3].eq_ignore_ascii_case(b"set")
    })
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

    if is_set_expr(&expr) {
        for q in expr.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            log::debug!("Executing SET statement: {q}");
            // NOTE: `config::write` acquires the engine write lock, so the
            // `engine.sql` branch must scope its read lock to that iteration only.
            let outcome = if let Some((key, value)) = parse_set_assignment(q) {
                execute_set_via_config(key, value).await
            } else {
                ENGINE
                    .read()
                    .await
                    .sql(q)
                    .await
                    .map(|_| ())
                    .map_err(Into::into)
            };
            outcome.with_context(|| format!("Failed SET query '{q}'"))?;
            log::debug!("Successfully executed SET statement: {q}");
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
}

// 处理Web API查询请求
pub async fn query(req: String) -> ApiResult<QueryHttpEnvelope> {
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
                message: err.to_string(),
                details: None,
            })
        }
    };

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
    Ok(QueryHttpEnvelope { body, partial })
}
