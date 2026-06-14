use std::collections::HashMap;

use axum::{
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;

use probing_core::core::ProbeExtensionManager;

use crate::engine::ENGINE;
use crate::server::api::response;
use crate::server::error::{ApiError, ApiResult};

/// Fallback handler: dispatch `/apis/*` to registered engine extensions.
#[axum::debug_handler]
pub async fn handle(req: axum::extract::Request) -> ApiResult<Response> {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let path = api_path(parts.uri.path());

    if method == Method::OPTIONS {
        return Ok(cors_preflight().into_response());
    }

    if let Some(route) = response::route_spec(path) {
        if route.method != method.as_str() {
            return Err(ApiError::method_not_allowed(format!(
                "Method {method} not allowed for {path}; expected {}",
                route.method
            )));
        }
    }

    let params: HashMap<String, String> = match parts.uri.query() {
        Some(q) => serde_urlencoded::from_str(q)
            .map_err(|e| ApiError::bad_request(format!("Invalid query string: {e}")))?,
        None => HashMap::new(),
    };

    let body_bytes = body.collect().await?.to_bytes();

    log::debug!(
        "Extension API [{method} {path}]: params = {params:?}, body_size = {} bytes",
        body_bytes.len()
    );

    let eem = {
        let engine = ENGINE.read().await;
        engine
            .context
            .state()
            .config()
            .options()
            .extensions
            .get::<ProbeExtensionManager>()
            .cloned()
    };

    let Some(eem) = eem else {
        return Ok((StatusCode::NOT_FOUND, "Extension manager not available").into_response());
    };

    match eem.call(path, &params, &body_bytes).await {
        Ok(response_bytes) => Ok(extension_response(path, response_bytes).into_response()),
        Err(e) => {
            log::error!("Extension call failed for path '{path}': {e}");
            Err(ApiError::from_engine(e))
        }
    }
}

/// Strip the `/apis` mount prefix so extensions match on `/{name}/…`.
pub fn api_path(full_path: &str) -> &str {
    full_path.strip_prefix("/apis").unwrap_or(full_path)
}

fn extension_response(path: &str, body: Vec<u8>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let meta = response::lookup(path);
    let status = response::status_for_extension_body(meta.content_type, &body);
    let mut headers = HeaderMap::new();
    response::apply_response_headers(meta, &mut headers);
    (status, headers, body)
}

fn cors_preflight() -> (StatusCode, HeaderMap, &'static str) {
    let mut headers = HeaderMap::new();
    response::append_cors(&mut headers);
    headers.insert(
        axum::http::header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
    (StatusCode::OK, headers, "")
}

#[cfg(test)]
mod tests {
    use super::api_path;
    use crate::server::api::response::{
        lookup, route_spec, status_for_extension_body, ResponseMeta,
    };
    use axum::http::StatusCode;

    fn load_spec() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/spec/api_spec.json");
        let text = std::fs::read_to_string(path).expect("read api_spec.json");
        serde_json::from_str(&text).expect("parse api_spec.json")
    }

    #[test]
    fn strips_apis_mount_prefix() {
        assert_eq!(
            api_path("/apis/pythonext/callstack"),
            "/pythonext/callstack"
        );
    }

    #[test]
    fn eval_route_is_post_in_spec() {
        let route = route_spec("/pythonext/eval").expect("eval route");
        assert_eq!(route.method, "POST");
    }

    #[test]
    fn response_lookup_follows_spec_not_path_heuristics() {
        assert_eq!(
            lookup("/pythonext/callstack"),
            ResponseMeta {
                content_type: "application/json",
                cors: false,
            }
        );
    }

    #[test]
    fn api_path_matches_pythonext_spec_urls() {
        let spec = load_spec();
        let ext = spec["routing"]["python_http_extension_name"]
            .as_str()
            .unwrap();
        for handler in spec["pythonext_handlers"].as_array().unwrap() {
            let local = handler["local_path"].as_str().unwrap();
            let full = format!("/apis/{ext}/{local}");
            assert_eq!(api_path(&full), format!("/{ext}/{local}"));
        }
    }

    #[test]
    fn handler_errors_map_to_http_status() {
        assert_eq!(
            status_for_extension_body(
                "application/json",
                br#"{"error":"No handler found for path: x"}"#
            ),
            StatusCode::NOT_FOUND
        );
    }
}
