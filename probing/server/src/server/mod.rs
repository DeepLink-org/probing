pub mod api;
mod query_dto;
mod repl;
mod runtime;
mod spa;
pub mod sql_guard;

pub use runtime::SERVER_RUNTIME;

pub mod cluster;
pub mod cluster_fanout;
pub mod cluster_query;
pub mod config;
pub mod error;
pub mod file_api;
pub mod health;
pub mod middleware;
pub mod system;
pub mod training;

use crate::server::error::ApiError;
use anyhow::Result;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use log::error;

use crate::engine::{handle_query, initialize_engine};
use crate::server::middleware::{
    connection_limit_middleware, request_logging_middleware, request_size_limit_middleware,
};
use crate::server::repl::ws_handler;
use probing_proto::prelude::Query;

/// Top-level routes outside `/apis`. Keep in sync with `tests/regression/spec/api_spec.json`.
pub const TOP_LEVEL_ROUTES: &[(&str, &str)] = &[
    ("GET", "/health"),
    ("GET", "/ready"),
    ("POST", "/query"),
    ("POST", "/query/dto"),
    ("GET", "/config/{config_key}"),
    ("GET", "/ws"),
    ("POST", "/mcp"),
];

async fn get_config_value_handler(
    axum::extract::Path(config_key): axum::extract::Path<String>,
) -> impl IntoResponse {
    match probing_core::config::get_str(&config_key).await {
        Some(value) => (StatusCode::OK, value).into_response(),
        None => ApiError::not_found(format!("Config key '{config_key}' not found")).into_response(),
    }
}

fn build_app(auth: bool) -> axum::Router {
    let mut app = spa::routes()
        .route("/health", axum::routing::get(health::liveness))
        .route("/ready", axum::routing::get(health::readiness))
        .route("/query", axum::routing::post(query))
        .route("/query/dto", axum::routing::post(query_dto::query_dto))
        .route(
            "/config/{config_key}",
            axum::routing::get(get_config_value_handler),
        )
        .nest("/apis", api::router())
        .route("/ws", axum::routing::get(ws_handler))
        .fallback(spa::fallback);

    #[cfg(feature = "rmcp")]
    {
        app = app.merge(crate::mcp::router());
    }

    if auth {
        app = app.layer(axum::middleware::from_fn(
            crate::auth::selective_auth_middleware,
        ));
    }

    app.layer(axum::middleware::from_fn(request_size_limit_middleware))
        .layer(axum::middleware::from_fn(request_logging_middleware))
        .layer(axum::middleware::from_fn(connection_limit_middleware))
}

async fn query(body: String) -> impl IntoResponse {
    if let Some(msg) = crate::engine_lifecycle::engine_not_ready_message() {
        return ApiError::service_unavailable(msg).into_response();
    }
    match crate::engine::query(body).await {
        Ok(envelope) => {
            let status = if envelope.partial {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            };
            (status, envelope.body).into_response()
        }
        Err(api_error) => api_error.into_response(),
    }
}

pub async fn local_server() -> Result<()> {
    #[cfg(target_os = "linux")]
    let socket_path = format!("\0probing-{}", std::process::id());
    #[cfg(not(target_os = "linux"))]
    let socket_path = {
        let pid = std::process::id();
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("probing-{}.sock", pid));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        path.to_string_lossy().to_string()
    };

    log::info!(
        "Starting local server at {}",
        socket_path.replace('\0', "@")
    );

    let app = build_app(false);
    axum::serve(tokio::net::UnixListener::bind(socket_path)?, app).await?;
    Ok(())
}

async fn run_local_server() {
    if let Err(err) = local_server().await {
        error!("local HTTP server exited: {err:#}");
        crate::engine_lifecycle::mark_engine_failed(format!("local server: {err:#}"));
        if crate::engine_lifecycle::engine_fail_fast_enabled() {
            error!("PROBING_ENGINE_FAIL_FAST=1 — exiting after local server failure");
            std::process::exit(1);
        }
    }
}

pub fn start_local() {
    if let Err(err) = probing_core::runtime::block_on(async move {
        match initialize_engine().await {
            Ok(()) => {
                log::info!("probing engine initialized");
            }
            Err(err) => {
                let msg = err.to_string();
                error!("Failed to initialize engine: {msg}");
                crate::engine_lifecycle::mark_engine_failed(msg.clone());
                if crate::engine_lifecycle::engine_fail_fast_enabled() {
                    error!("PROBING_ENGINE_FAIL_FAST=1 — exiting");
                    std::process::exit(1);
                }
            }
        }
    }) {
        error!("probing runtime unavailable during engine init: {err}");
        crate::engine_lifecycle::mark_engine_failed(err.to_string());
        if crate::engine_lifecycle::engine_fail_fast_enabled() {
            error!("PROBING_ENGINE_FAIL_FAST=1 — exiting");
            std::process::exit(1);
        }
    }
    SERVER_RUNTIME.spawn(async move {
        run_local_server().await;
    });
}

pub async fn remote_server(addr: Option<String>) -> Result<()> {
    let addr = addr.unwrap_or_else(|| "0.0.0.0:0".to_string());
    log::info!("Starting probe server at {addr}");

    crate::auth::bootstrap_auth_from_env().await;

    let app = build_app(true);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    match listener.local_addr() {
        Ok(addr) => {
            {
                let mut probing_address = crate::vars::write_probing_address();
                *probing_address = addr.to_string();
            }
            probing_core::core::cluster::set_local_listen_addrs(vec![addr.to_string()]);
            log::info!("probing server is available on: {addr}");
            probing_core::config::write("server.address", &addr.to_string()).await?;
        }
        Err(err) => {
            log::error!("error getting server address: {err}");
        }
    }
    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_remote_server(addr: Option<String>) {
    if let Err(err) = remote_server(addr).await {
        error!("remote HTTP server exited: {err:#}");
        crate::engine_lifecycle::mark_engine_failed(format!("remote server: {err:#}"));
        if crate::engine_lifecycle::engine_fail_fast_enabled() {
            error!("PROBING_ENGINE_FAIL_FAST=1 — exiting after remote server failure");
            std::process::exit(1);
        }
    }
}

pub fn start_remote(addr: Option<String>) {
    SERVER_RUNTIME.spawn(async move {
        run_remote_server(addr).await;
    });
}

pub fn sync_env_settings() {
    let env_vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| {
            k.starts_with("PROBING_")
                && ![
                    "PROBING_PORT",
                    "PROBING_LOGLEVEL",
                    "PROBING_ASSETS_ROOT",
                    "PROBING_SERVER_ADDRPATTERN",
                    "PROBING_AUTH_TOKEN",
                    "PROBING_BASE_PATH",
                ]
                .contains(&k.as_str())
        })
        .collect();

    SERVER_RUNTIME.spawn(async move {
        for (k, v) in env_vars {
            let k = k.replace("_", ".").to_lowercase();
            let setting = format!("set {k}={v}");
            match handle_query(Query {
                expr: setting,
                opts: None,
            })
            .await
            {
                Ok(_) => log::debug!("Synced env setting: {k}"),
                Err(err) => error!("Failed to sync env settings: set {k}={v}, {err}"),
            };
        }
    });
}

#[cfg(test)]
mod spec_tests {
    use super::TOP_LEVEL_ROUTES;

    fn load_spec() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/regression/spec/api_spec.json");
        let text = std::fs::read_to_string(path).expect("read api_spec.json");
        serde_json::from_str(&text).expect("parse api_spec.json")
    }

    #[test]
    fn top_level_routes_match_api_spec() {
        let spec = load_spec();
        let expected: Vec<(String, String)> = spec["top_level"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["method"].as_str().unwrap().to_string(),
                    entry["path"].as_str().unwrap().to_string(),
                )
            })
            .collect();

        let actual: Vec<(String, String)> = TOP_LEVEL_ROUTES
            .iter()
            .map(|(m, p)| (m.to_string(), p.to_string()))
            .collect();

        assert_eq!(actual, expected);
    }
}
