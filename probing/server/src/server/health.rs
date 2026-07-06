//! Liveness / readiness probes for orchestrators and load balancers.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::engine_lifecycle::{engine_init_state, EngineInitState};

#[derive(Serialize)]
struct LivenessResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct ReadinessResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// Process is up and the HTTP server is accepting connections.
pub async fn liveness() -> impl IntoResponse {
    (StatusCode::OK, Json(LivenessResponse { status: "ok" }))
}

/// Engine finished initialization and can serve SQL queries.
pub async fn readiness() -> impl IntoResponse {
    match engine_init_state() {
        EngineInitState::Ready => (
            StatusCode::OK,
            Json(ReadinessResponse {
                status: "ready",
                reason: None,
            }),
        )
            .into_response(),
        EngineInitState::Uninitialized => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessResponse {
                status: "starting",
                reason: Some("engine not initialized yet".into()),
            }),
        )
            .into_response(),
        EngineInitState::Failed(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessResponse {
                status: "failed",
                reason: Some(reason),
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_lifecycle::{mark_engine_failed, mark_engine_ready};

    #[tokio::test]
    async fn readiness_reflects_engine_state() {
        mark_engine_ready();
        let resp = readiness().await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);

        mark_engine_failed("boom");
        let resp = readiness().await.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
