use std::collections::HashMap;

use axum::{
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;

use probing_core::core::{ExtensionRoute, ProbeExtensionManager, ProbeExtensionResponse};

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

    let Some(route) = eem.route(path).await else {
        return Err(ApiError::not_found(format!(
            "No extension route registered for {path}"
        )));
    };
    if route.method.as_str() != method.as_str() {
        return Err(ApiError::method_not_allowed(format!(
            "Method {method} not allowed for {path}; expected {}",
            route.method.as_str()
        )));
    }
    if route.requires_engine_ready {
        if let Some(message) = crate::engine_lifecycle::engine_not_ready_message() {
            return Err(ApiError::service_unavailable(message));
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

    match eem.call_response(path, &params, &body_bytes).await {
        Ok(extension) => Ok(extension_response(route, extension).into_response()),
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

fn extension_response(
    route: ExtensionRoute,
    response: ProbeExtensionResponse,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let meta = response::response_meta(route);
    let status = if response.partial {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        response::status_for_extension_body(meta.content_type, &response.body)
    };
    let mut headers = HeaderMap::new();
    response::apply_response_headers(meta, &mut headers);
    (status, headers, response.body)
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
    use super::{api_path, extension_response};
    use crate::server::api::response::status_for_extension_body;
    use axum::http::StatusCode;
    use probing_core::core::{
        ExtensionContentType, ExtensionHttpMethod, ExtensionRoute, ProbeExtensionCall,
        ProbeExtensionResponse,
    };

    fn load_spec() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/regression/spec/api_spec.json");
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
    fn registered_route_contracts_match_api_spec() {
        let spec = load_spec();
        #[allow(unused_mut)]
        let mut contracts = vec![
            (
                "pythonext",
                probing_python::extensions::PythonExt::default().routes(),
            ),
            (
                "torchextension",
                probing_python::extensions::TorchProbeExtension::default().routes(),
            ),
            (
                "pprofextension",
                probing_python::extensions::PprofProbeExtension::default().routes(),
            ),
        ];
        #[cfg(target_os = "linux")]
        contracts.push((
            "rdmaextension",
            probing_cc::extensions::RdmaProbeExtension::default().routes(),
        ));

        let expected = spec["pythonext_handlers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| ("pythonext", entry))
            .chain(
                spec["other_extensions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|entry| {
                        cfg!(target_os = "linux") || entry["extension_name"] != "rdmaextension"
                    })
                    .map(|entry| (entry["extension_name"].as_str().unwrap(), entry)),
            );

        for (extension_name, entry) in expected {
            let local_path = entry["local_path"].as_str().unwrap();
            let route = contracts
                .iter()
                .find(|(name, _)| *name == extension_name)
                .and_then(|(_, routes)| routes.iter().find(|route| route.path == local_path))
                .unwrap_or_else(|| panic!("missing typed route {extension_name}/{local_path}"));
            assert_eq!(route.method.as_str(), entry["method"].as_str().unwrap());
            assert_eq!(
                route.content_type.as_str(),
                entry["response"]["content_type"].as_str().unwrap()
            );
            assert_eq!(
                route.cors,
                entry["response"]["cors"].as_bool().unwrap_or(false)
            );
            assert_eq!(
                route.requires_engine_ready,
                entry["requires_engine_ready"].as_bool().unwrap_or(false)
            );
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

    #[test]
    fn partial_extension_response_maps_to_service_unavailable() {
        let response = ProbeExtensionResponse {
            body: br#"{"frames":[]}"#.to_vec(),
            partial: true,
        };

        assert_eq!(
            extension_response(
                ExtensionRoute::new(
                    "flamegraph/distributed/json",
                    ExtensionHttpMethod::Get,
                    ExtensionContentType::Json,
                ),
                response,
            )
            .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
