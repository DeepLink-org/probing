//! Axum application construction and top-level HTTP handlers.

use axum::http::StatusCode;
use axum::response::IntoResponse;

use super::error::ApiError;
use super::middleware::{
    connection_limit_middleware, request_logging_middleware, request_size_limit_middleware,
    request_timeout_middleware,
};
use super::repl::ws_handler;

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

pub(super) fn build_app(auth: bool) -> axum::Router {
    let mut app = super::spa::routes()
        .route("/health", axum::routing::get(super::health::liveness))
        .route("/ready", axum::routing::get(super::health::readiness))
        .route("/query", axum::routing::post(query))
        .route(
            "/query/dto",
            axum::routing::post(super::query_dto::query_dto),
        )
        .route(
            "/config/{config_key}",
            axum::routing::get(get_config_value_handler),
        )
        .nest("/apis", super::api::router())
        .route("/ws", axum::routing::get(ws_handler))
        .fallback(super::spa::fallback);

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
        .layer(axum::middleware::from_fn(request_timeout_middleware))
        .layer(axum::middleware::from_fn(connection_limit_middleware))
}

async fn query(body: String) -> impl IntoResponse {
    if let Some(message) = crate::engine_lifecycle::engine_not_ready_message() {
        return ApiError::service_unavailable(message).into_response();
    }
    match crate::engine::query(body).await {
        Ok(envelope) => {
            let status = if envelope.error {
                StatusCode::INTERNAL_SERVER_ERROR
            } else if envelope.partial {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            };
            (status, envelope.body).into_response()
        }
        Err(api_error) => api_error.into_response(),
    }
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
            .map(|(method, path)| (method.to_string(), path.to_string()))
            .collect();

        assert_eq!(actual, expected);
    }
}
