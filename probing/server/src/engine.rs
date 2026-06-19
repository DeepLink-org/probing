use std::sync::Arc;

use anyhow::{self, Result};
use probing_proto::prelude::*;

use crate::extensions as se;
use probing_cc::extensions as cc;
#[cfg(feature = "gpu")]
use probing_gpu::extensions as gpu;
use probing_python::extensions as py;

use probing_core::config;

use crate::server::error::{ApiError, ApiResult};

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
    }
    result
}

/// Parse `SET key = value` (value may be quoted).
fn parse_set_assignment(stmt: &str) -> Option<(&str, &str)> {
    let mut s = stmt.trim();
    if s.len() >= 3
        && s.as_bytes()[0].to_ascii_lowercase() == b's'
        && s.as_bytes()[1].to_ascii_lowercase() == b'e'
        && s.as_bytes()[2].to_ascii_lowercase() == b't'
    {
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
        p.len() >= 3
            && p.as_bytes()[0].to_ascii_lowercase() == b's'
            && p.as_bytes()[1].to_ascii_lowercase() == b'e'
            && p.as_bytes()[2].to_ascii_lowercase() == b't'
    })
}

/// Route extension SET knobs through `config::write` (`probing.<namespace>.*`).
async fn execute_set_via_config(key: &str, value: &str) -> Result<()> {
    let probe_key = if key.starts_with("probing.") {
        key.to_string()
    } else {
        format!("probing.{key}")
    };
    config::write(&probe_key, value)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

pub async fn handle_query(request: Query) -> Result<QueryDataFormat> {
    let Query { expr, opts: _ } = request;

    // We are already running within the Axum/Tokio runtime.

    if is_set_expr(&expr) {
        for q in expr.split(';').filter(|s| !s.trim().is_empty()) {
            let trimmed_q = q.trim();
            if trimmed_q.is_empty() {
                continue;
            }
            log::debug!("Executing SET statement: {trimmed_q}");
            if let Some((key, value)) = parse_set_assignment(trimmed_q) {
                match execute_set_via_config(key, value).await {
                    Ok(()) => log::debug!("Successfully configured: {key}={value}"),
                    Err(e) => {
                        log::error!("Error executing SET statement '{trimmed_q}': {e}");
                        return Err(anyhow::anyhow!("Failed SET query '{trimmed_q}': {e}"));
                    }
                }
            } else {
                let engine = ENGINE.read().await;
                match engine.sql(trimmed_q).await {
                    Ok(_) => log::debug!("Successfully executed: {trimmed_q}"),
                    Err(e) => {
                        log::error!("Error executing SET statement '{trimmed_q}': {e}");
                        return Err(anyhow::anyhow!("Failed SET query '{trimmed_q}': {e}"));
                    }
                }
            }
        }
        return Ok(QueryDataFormat::Nil);
    }

    let engine = ENGINE.read().await;
    log::debug!("Executing SELECT query: {expr}");
    match engine.async_query(&expr).await {
        Ok(Some(dataframe)) => Ok(QueryDataFormat::DataFrame(dataframe)),
        Ok(None) => Ok(QueryDataFormat::Nil),
        Err(e) => {
            log::error!("Error executing SELECT query '{expr}': {e}");
            Err(e.into())
        }
    }
}

// 处理Web API查询请求
pub async fn query(req: String) -> ApiResult<String> {
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
    let reply_message = Message::new(reply_payload);

    // Serialize the response message
    serde_json::to_string(&reply_message).map_err(|e| {
        log::error!("Failed to serialize query response: {e}");
        anyhow::anyhow!("Failed to create response: {}", e).into() // Convert to ApiError
    })
}
