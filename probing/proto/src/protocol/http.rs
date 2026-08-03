//! Shared HTTP error envelope used by Server, Web, and CLI clients.

use serde::{Deserialize, Serialize};

/// Machine-readable problem details for a failed HTTP request.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ApiProblem {
    /// Stable symbolic code such as `NOT_FOUND` or `SERVICE_UNAVAILABLE`.
    pub code: String,
    /// Human-readable explanation. Clients should display this instead of the status phrase.
    pub message: String,
    /// Whether retrying the same request may succeed without changing its inputs.
    #[serde(default)]
    pub retryable: bool,
    /// Optional operator action that can make the request succeed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// Canonical non-success response body for Probing HTTP APIs.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ApiErrorResponse {
    pub error: ApiProblem,
}

impl ApiErrorResponse {
    pub fn new(error: ApiProblem) -> Self {
        Self { error }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_envelope_roundtrips() {
        let response = ApiErrorResponse::new(ApiProblem {
            code: "SERVICE_UNAVAILABLE".into(),
            message: "engine is starting".into(),
            retryable: true,
            action: Some("retry after /ready reports ready".into()),
        });
        let json = serde_json::to_string(&response).unwrap();
        let decoded: ApiErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, response);
    }
}
