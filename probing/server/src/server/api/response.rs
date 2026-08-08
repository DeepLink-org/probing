//! HTTP rendering for type-checked extension route contracts.

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use probing_core::core::ExtensionRoute;

/// Per-endpoint response metadata from the API spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponseMeta {
    pub content_type: &'static str,
    pub cors: bool,
}

impl Default for ResponseMeta {
    fn default() -> Self {
        Self {
            content_type: "text/plain",
            cors: false,
        }
    }
}

pub fn response_meta(route: ExtensionRoute) -> ResponseMeta {
    ResponseMeta {
        content_type: route.content_type.as_str(),
        cors: route.cors,
    }
}

/// HTTP status for an extension response body (Python router JSON errors → 4xx).
pub fn status_for_extension_body(content_type: &str, body: &[u8]) -> StatusCode {
    if content_type != "application/json" {
        return StatusCode::OK;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return StatusCode::OK;
    };
    let Some(error) = value.get("error").and_then(|v| v.as_str()) else {
        return StatusCode::OK;
    };
    if error.contains("No handler found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    }
}

pub fn apply_response_headers(meta: ResponseMeta, headers: &mut HeaderMap) {
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static(meta.content_type),
    );
    if meta.cors {
        append_cors(headers);
    }
}

pub fn append_cors(headers: &mut HeaderMap) {
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Accept"),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Type, Content-Length"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_core::core::{ExtensionContentType, ExtensionHttpMethod};

    #[test]
    fn response_meta_comes_from_typed_route() {
        let route = ExtensionRoute::new(
            "trace/chrome-tracing",
            ExtensionHttpMethod::Get,
            ExtensionContentType::Json,
        )
        .with_cors();
        assert_eq!(
            response_meta(route),
            ResponseMeta {
                content_type: "application/json",
                cors: true,
            }
        );
    }

    #[test]
    fn json_handler_error_returns_bad_request() {
        let body = br#"{"error":"Missing required parameter: function"}"#;
        assert_eq!(
            status_for_extension_body("application/json", body),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn json_no_handler_returns_not_found() {
        let body = br#"{"error":"No handler found for path: foo"}"#;
        assert_eq!(
            status_for_extension_body("application/json", body),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn plain_text_body_stays_ok() {
        assert_eq!(
            status_for_extension_body("text/plain", b"hello"),
            StatusCode::OK
        );
    }
}
