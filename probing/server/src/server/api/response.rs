//! Extension HTTP routing metadata driven by `tests/regression/spec/api_spec.json`.

use std::collections::HashMap;

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use once_cell::sync::Lazy;

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

/// Method + response metadata for a spec-defined extension route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtensionRouteSpec {
    pub method: &'static str,
    pub response: ResponseMeta,
}

static ROUTE_MAP: Lazy<HashMap<String, ExtensionRouteSpec>> = Lazy::new(|| {
    build_route_map().unwrap_or_else(|e| {
        log::error!("failed to build extension route map: {e}; using empty map");
        HashMap::new()
    })
});

fn build_route_map() -> Result<HashMap<String, ExtensionRouteSpec>, String> {
    let spec: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/regression/spec/api_spec.json"
    ))
    .map_err(|e| format!("parse api_spec.json: {e}"))?;

    let defaults = spec
        .get("extension_response_defaults")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    let mut map = HashMap::new();

    let ext = spec["routing"]["python_http_extension_name"]
        .as_str()
        .ok_or("missing python_http_extension_name")?;
    let handlers = spec["pythonext_handlers"]
        .as_array()
        .ok_or("missing pythonext_handlers")?;
    for handler in handlers {
        let local = handler["local_path"]
            .as_str()
            .ok_or("missing local_path in pythonext_handlers entry")?;
        let method = handler["method"]
            .as_str()
            .ok_or("missing method in pythonext_handlers entry")?;
        let response = handler.get("response").unwrap_or(&defaults);
        map.insert(
            format!("{ext}/{local}"),
            ExtensionRouteSpec {
                method: parse_method(method),
                response: parse_response_meta(response, &defaults),
            },
        );
    }

    let other_extensions = spec["other_extensions"]
        .as_array()
        .ok_or("missing other_extensions")?;
    for entry in other_extensions {
        let name = entry["extension_name"]
            .as_str()
            .ok_or("missing extension_name in other_extensions entry")?;
        let local = entry["local_path"].as_str().unwrap_or("");
        let method = entry["method"]
            .as_str()
            .ok_or("missing method in other_extensions entry")?;
        let response = entry.get("response").unwrap_or(&defaults);
        let key = if local.is_empty() {
            name.to_string()
        } else {
            format!("{name}/{local}")
        };
        map.insert(
            key,
            ExtensionRouteSpec {
                method: parse_method(method),
                response: parse_response_meta(response, &defaults),
            },
        );
    }

    Ok(map)
}

fn parse_method(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        other => {
            log::error!("unsupported HTTP method in api_spec.json: {other}; defaulting to GET");
            "GET"
        }
    }
}

fn parse_response_meta(response: &serde_json::Value, defaults: &serde_json::Value) -> ResponseMeta {
    let content_type = response
        .get("content_type")
        .or_else(|| defaults.get("content_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("text/plain");
    let cors = response
        .get("cors")
        .or_else(|| defaults.get("cors"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    ResponseMeta {
        content_type: match content_type {
            "application/json" => "application/json",
            "text/plain" => "text/plain",
            "text/html" => "text/html",
            other => {
                log::error!(
                    "unsupported extension response content_type in api_spec.json: {other}; \
                     defaulting to text/plain"
                );
                "text/plain"
            }
        },
        cors,
    }
}

fn route_key(path: &str) -> String {
    path.trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

/// Look up spec metadata for an extension path (e.g. `/pythonext/trace/list`).
pub fn route_spec(path: &str) -> Option<&'static ExtensionRouteSpec> {
    let key = route_key(path);
    ROUTE_MAP.get(&key)
}

/// Look up response metadata; unknown paths use defaults.
pub fn lookup(path: &str) -> ResponseMeta {
    route_spec(path)
        .map(|spec| spec.response)
        .unwrap_or_default()
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

    fn load_spec() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/regression/spec/api_spec.json");
        let text = std::fs::read_to_string(path).expect("read api_spec.json");
        serde_json::from_str(&text).expect("parse api_spec.json")
    }

    #[test]
    fn spec_paths_have_route_entries() {
        let spec = load_spec();
        let ext = spec["routing"]["python_http_extension_name"]
            .as_str()
            .unwrap();

        for handler in spec["pythonext_handlers"].as_array().unwrap() {
            let local = handler["local_path"].as_str().unwrap();
            let path = format!("/{ext}/{local}");
            let route = route_spec(&path).expect("route spec");
            assert_eq!(route.method, handler["method"].as_str().unwrap());
            assert_eq!(
                route.response.content_type,
                handler["response"]["content_type"].as_str().unwrap()
            );
        }
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

    #[test]
    fn lookup_unknown_path_uses_defaults_without_panic() {
        let meta = lookup("/pythonext/does-not-exist");
        assert_eq!(meta.content_type, "text/plain");
        assert!(!meta.cors);
    }
}
