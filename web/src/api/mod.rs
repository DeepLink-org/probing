//! HTTP client and typed endpoints.
//!
//! Naming: `trace` = Python live variable tracing (`/python` page);
//! `traces` = distributed span trees and Chrome/Ray timelines.

use crate::utils::error::{AppError, Result};
use probing_proto::prelude::ApiErrorResponse;

fn fallback_http_code(status: reqwest::StatusCode) -> String {
    status
        .canonical_reason()
        .unwrap_or("HTTP_ERROR")
        .to_ascii_uppercase()
        .replace([' ', '-'], "_")
}

fn legacy_json_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("error")
        .and_then(|error| error.as_str().map(str::to_string))
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.as_str().map(str::to_string))
        })
        .or_else(|| {
            value
                .pointer("/payload/Error/message")
                .and_then(|message| message.as_str().map(str::to_string))
        })
        .or_else(|| {
            value
                .pointer("/payload/value/message")
                .and_then(|message| message.as_str().map(str::to_string))
        })
}

fn http_response_error(status: reqwest::StatusCode, body: &str) -> AppError {
    if let Ok(response) = serde_json::from_str::<ApiErrorResponse>(body) {
        let problem = response.error;
        return AppError::http(
            status.as_u16(),
            problem.code,
            problem.message,
            problem.retryable,
            problem.action,
        );
    }

    let legacy_message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| legacy_json_message(&value));
    let plain_message =
        (!body.trim().is_empty()).then(|| body.trim().chars().take(512).collect::<String>());
    let message = legacy_message.or(plain_message).unwrap_or_else(|| {
        status
            .canonical_reason()
            .unwrap_or("HTTP request failed")
            .to_string()
    });

    AppError::http(
        status.as_u16(),
        fallback_http_code(status),
        message,
        matches!(status.as_u16(), 429 | 502 | 503 | 504),
        None,
    )
}

fn is_accepted_partial_response(status: reqwest::StatusCode, body: &str, accepted: &[u16]) -> bool {
    accepted.contains(&status.as_u16()) && serde_json::from_str::<ApiErrorResponse>(body).is_err()
}

/// Base API client
pub struct ApiClient;

impl ApiClient {
    pub fn new() -> Self {
        Self
    }

    /// Get current page origin
    fn get_origin() -> Result<String> {
        web_sys::window()
            .ok_or_else(|| AppError::Api("No window object".to_string()))?
            .location()
            .origin()
            .map_err(|_| AppError::Api("Failed to get origin".to_string()))
    }

    /// Build API URL
    fn build_url(path: &str) -> Result<String> {
        Ok(format!(
            "{}{}",
            Self::get_origin()?,
            crate::utils::base_path::with_base(path)
        ))
    }

    /// Send GET request
    async fn get_request(&self, path: &str) -> Result<String> {
        self.get_request_accepting(path, &[]).await
    }

    /// Send GET request while preserving a response body for explicitly accepted statuses.
    async fn get_request_accepting(&self, path: &str, accepted: &[u16]) -> Result<String> {
        self.get_request_accepting_with_timeout(path, accepted, None)
            .await
    }

    /// Send GET request with an endpoint-specific deadline.
    async fn get_request_accepting_with_timeout(
        &self,
        path: &str,
        accepted: &[u16],
        timeout: Option<std::time::Duration>,
    ) -> Result<String> {
        let url = Self::build_url(path)?;
        let mut request = reqwest::Client::new().get(&url);
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                let seconds = timeout.map(|value| value.as_secs()).unwrap_or_default();
                AppError::http(
                    504,
                    "CLIENT_REQUEST_TIMEOUT",
                    format!("No response was received within {seconds} seconds"),
                    true,
                    Some("retry after checking peer availability".to_string()),
                )
            } else {
                error.into()
            }
        })?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() && !is_accepted_partial_response(status, &body, accepted) {
            return Err(http_response_error(status, &body));
        }

        Ok(body)
    }

    /// Send POST request (custom Content-Type)
    async fn post_request_with_body(&self, path: &str, body: String) -> Result<String> {
        self.post_request_with_body_accepting(path, body, &[]).await
    }

    /// Send POST request while preserving a response body for explicitly accepted statuses.
    async fn post_request_with_body_accepting(
        &self,
        path: &str,
        body: String,
        accepted: &[u16],
    ) -> Result<String> {
        let url = Self::build_url(path)?;
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .body(body)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() && !is_accepted_partial_response(status, &body, accepted) {
            return Err(http_response_error(status, &body));
        }

        Ok(body)
    }

    /// Send GET request (public wrapper for agent / extensions).
    pub async fn get_raw(&self, path: &str) -> Result<String> {
        self.get_request(path).await
    }

    /// Parse JSON response
    pub fn parse_json<T: serde::de::DeserializeOwned>(response: &str) -> Result<T> {
        serde_json::from_str(response)
            .map_err(|e| AppError::Api(format!("JSON parse error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_error_preserves_problem_details() {
        let error = http_response_error(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            r#"{"error":{"code":"ENGINE_STARTING","message":"engine is starting","retryable":true,"action":"wait for /ready"}}"#,
        );
        assert_eq!(
            error.display_message(),
            "engine is starting · wait for /ready"
        );
        assert!(matches!(
            error,
            AppError::Http {
                status: 503,
                retryable: true,
                ..
            }
        ));
    }

    #[test]
    fn plain_text_error_is_not_replaced_by_status_phrase() {
        let error = http_response_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "python.trace_event is not available",
        );
        assert_eq!(
            error.display_message(),
            "python.trace_event is not available"
        );
    }

    #[test]
    fn legacy_json_error_remains_readable() {
        let error = http_response_error(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"missing SQL expression"}"#,
        );
        assert_eq!(error.display_message(), "missing SQL expression");
    }

    #[test]
    fn accepted_partial_status_does_not_hide_canonical_error() {
        let body = r#"{"error":{"code":"SERVICE_UNAVAILABLE","message":"engine is starting","retryable":true}}"#;
        assert!(!is_accepted_partial_response(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            body,
            &[503]
        ));
        assert!(is_accepted_partial_response(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            r#"{"dataframe":{},"meta":{"partial":true}}"#,
            &[503]
        ));
    }
}

// Export all API modules
mod analytics;
mod cluster;
mod cpu;
mod dashboard;
mod files;
mod gpu;
mod overhead;
mod profiling;
mod pulsing;
mod pytorch;
mod repl;
mod rl;
mod skills;
mod stack;
mod trace;
mod traces;
mod training;

#[allow(unused_imports)]
pub use analytics::*;
#[allow(unused_imports)]
pub use cluster::*;
#[allow(unused_imports)]
pub use cpu::*;
#[allow(unused_imports)]
pub use dashboard::*;
#[allow(unused_imports)]
pub use gpu::*;
#[allow(unused_imports)]
pub use overhead::*;
#[allow(unused_imports)]
pub use profiling::*;
#[allow(unused_imports)]
pub use pulsing::*;
#[allow(unused_imports)]
pub use pytorch::*;
#[allow(unused_imports)]
pub use repl::*;
#[allow(unused_imports)]
pub use rl::*;
#[allow(unused_imports)]
pub use skills::*;
#[allow(unused_imports)]
pub use stack::*;
#[allow(unused_imports)]
pub use trace::*;
#[allow(unused_imports)]
pub use traces::*;
#[allow(unused_imports)]
pub use training::*;
