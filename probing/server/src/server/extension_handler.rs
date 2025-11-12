use std::collections::HashMap;

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;

use probing_core::core::EngineExtensionManager;

use super::error::ApiResult;
use crate::engine::ENGINE;

/// Handle extension API calls
#[axum::debug_handler]
pub async fn handle_extension_call(req: axum::extract::Request) -> ApiResult<Response> {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path();
    let method = parts.method.clone();
    
    // Handle CORS preflight requests (OPTIONS)
    // Perfetto UI may send OPTIONS requests before the actual GET request
    // This is required for cross-origin requests from https://ui.perfetto.dev
    if method == axum::http::Method::OPTIONS {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"), // Allows https://ui.perfetto.dev
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
        headers.insert(
            axum::http::header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("86400"), // Cache preflight response for 24 hours
        );
        return Ok((StatusCode::OK, headers, "").into_response());
    }
    
    let params_str = parts.uri.query().unwrap_or_default();
    let params: HashMap<String, String> =
        serde_urlencoded::from_str(params_str).unwrap_or_default();

    // Body size is already limited by middleware, so we can safely collect it
    let body_bytes = body.collect().await?.to_bytes();

    // Only log request details in debug mode to avoid log spam
    log::debug!(
        "Extension API Call[{}]: params = {:?}, body_size = {} bytes",
        path,
        params,
        body_bytes.len()
    );

    let eem = {
        let engine = ENGINE.write().await;
        let state = engine.context.state();
        state
            .config()
            .options()
            .extensions
            .get::<EngineExtensionManager>()
            .cloned()
    };

    if let Some(eem) = eem {
            match eem.call(path, &params, &body_bytes).await {
            Ok(response) => {
                // Determine content type based on path
                let content_type = if path.contains("timeline") || path.contains("chrome-tracing") {
                    "application/json"
                } else {
                    "text/plain"
                };
                
                // Create response with headers
                let mut headers = HeaderMap::new();
                headers.insert(
                    axum::http::header::CONTENT_TYPE,
                    HeaderValue::from_static(content_type),
                );
                
                // Add CORS headers for trace data endpoints
                // This allows Perfetto UI (https://ui.perfetto.dev) to fetch trace data
                // Using "*" allows all origins, including https://ui.perfetto.dev
                if path.contains("timeline") || path.contains("chrome-tracing") {
                    headers.insert(
                        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                        HeaderValue::from_static("*"), // Allows https://ui.perfetto.dev and all other origins
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
                    // Allow credentials if needed (currently not used, but can be enabled)
                    // headers.insert(
                    //     axum::http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                    //     HeaderValue::from_static("true"),
                    // );
                }
                
                return Ok((StatusCode::OK, headers, response).into_response());
            }
            Err(e) => {
                log::warn!("Extension call failed for path '{path}': {e}");
                return Err(anyhow::anyhow!("Extension call failed: {}", e).into());
            }
        }
    }

    // Return 404 if no extension manager is available
    Ok((StatusCode::NOT_FOUND, "Extension not found").into_response())
}
