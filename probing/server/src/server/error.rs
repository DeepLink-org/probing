use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use probing_core::core::EngineError;
use probing_proto::prelude::{ApiErrorResponse, ApiProblem};

/// HTTP API error with an explicit status code.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    problem: ApiProblem,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            problem: ApiProblem {
                code: code_for_status(status).to_string(),
                message: message.into(),
                retryable: retryable_status(status),
                action: None,
            },
        }
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.problem.action = Some(action.into());
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        Self::new(StatusCode::METHOD_NOT_ALLOWED, message)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, message)
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }

    pub fn gateway_timeout(message: impl Into<String>) -> Self {
        Self::new(StatusCode::GATEWAY_TIMEOUT, message)
    }

    pub fn from_engine(err: EngineError) -> Self {
        match err {
            EngineError::CallError(msg) | EngineError::PluginNotFound(msg) => Self::not_found(msg),
            EngineError::UnsupportedCall => Self::not_found("Unsupported API call"),
            EngineError::InvalidCallParameter(name, value) => {
                Self::bad_request(format!("Invalid API parameter: {name}={value}"))
            }
            EngineError::PluginError(msg) => Self::new(StatusCode::BAD_GATEWAY, msg),
            EngineError::QueryError(msg)
            | EngineError::InternalError(msg)
            | EngineError::ConfigError(msg) => Self::internal(msg),
            other => Self::internal(other.to_string()),
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn message(&self) -> &str {
        &self.problem.message
    }

    pub fn problem(&self) -> &ApiProblem {
        &self.problem
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.problem.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ApiErrorResponse::new(self.problem))).into_response()
    }
}

fn code_for_status(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "BAD_REQUEST",
        StatusCode::UNAUTHORIZED => "UNAUTHORIZED",
        StatusCode::FORBIDDEN => "FORBIDDEN",
        StatusCode::NOT_FOUND => "NOT_FOUND",
        StatusCode::METHOD_NOT_ALLOWED => "METHOD_NOT_ALLOWED",
        StatusCode::PAYLOAD_TOO_LARGE => "PAYLOAD_TOO_LARGE",
        StatusCode::TOO_MANY_REQUESTS => "TOO_MANY_REQUESTS",
        StatusCode::BAD_GATEWAY => "BAD_GATEWAY",
        StatusCode::SERVICE_UNAVAILABLE => "SERVICE_UNAVAILABLE",
        StatusCode::GATEWAY_TIMEOUT => "GATEWAY_TIMEOUT",
        _ => "INTERNAL_ERROR",
    }
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        let err = err.into();
        Self::internal(format!("{err:#}"))
    }
}

/// Alias for convenience
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_call_error_maps_to_not_found() {
        let err = ApiError::from_engine(EngineError::CallError("missing".into()));
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn engine_plugin_error_maps_to_bad_gateway() {
        let err = ApiError::from_engine(EngineError::PluginError("boom".into()));
        assert_eq!(err.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn invalid_call_parameter_maps_to_bad_request() {
        let err = ApiError::from_engine(EngineError::InvalidCallParameter(
            "cluster".into(),
            "sometimes".into(),
        ));
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn response_is_structured_json() {
        let response = ApiError::service_unavailable("engine is starting")
            .with_action("retry after /ready reports ready")
            .into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let parsed: ApiErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.error.code, "SERVICE_UNAVAILABLE");
        assert_eq!(parsed.error.message, "engine is starting");
        assert!(parsed.error.retryable);
        assert_eq!(
            parsed.error.action.as_deref(),
            Some("retry after /ready reports ready")
        );
    }
}
