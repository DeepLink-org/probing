use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use probing_core::core::EngineError;

/// HTTP API error with an explicit status code.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
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

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub fn from_engine(err: EngineError) -> Self {
        match err {
            EngineError::CallError(msg) | EngineError::PluginNotFound(msg) => Self::not_found(msg),
            EngineError::UnsupportedCall => Self::not_found("Unsupported API call"),
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
        &self.message
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::internal(err.into().to_string())
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
}
