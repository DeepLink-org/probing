//! Dioxus SPA shell: serve `index.html` for client-side routes, static files otherwise.

use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Router};

use crate::asset::{contains, index, static_files};

/// Static files that must not fall back to the SPA shell (avoids serving HTML as CSS/JS).
fn is_static_asset_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name.rsplit_once('.').map(|(_, ext)| ext),
        Some(
            "css" | "js" | "wasm" | "svg" | "png" | "jpg" | "jpeg" | "gif" | "ico" | "json" | "br"
        )
    )
}

/// True for backend/API endpoints that must not return the SPA shell.
pub fn is_api_path(path: &str) -> bool {
    path == "/query"
        || path == "/query/dto"
        || path.starts_with("/apis/")
        || path.starts_with("/config/")
        || path.starts_with("/mcp")
        || path == "/ws"
}

/// Explicit shell routes (everything else falls through to [`fallback`]).
pub fn routes() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
}

/// SPA fallback: static asset if it exists, otherwise `index.html` for client routing.
pub async fn fallback(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path();

    if is_api_path(path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    if is_static_asset_path(path) && !contains(path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    if contains(path) {
        if let Ok(resp) = static_files(uri, headers).await {
            return resp.into_response();
        }
    }

    index().await.into_response()
}

#[cfg(test)]
mod tests {
    use super::{is_api_path, is_static_asset_path};

    #[test]
    fn api_paths_are_not_spa() {
        assert!(is_api_path("/query"));
        assert!(is_api_path("/query/dto"));
        assert!(is_api_path("/apis/nodes"));
        assert!(is_api_path("/config/server.address"));
        assert!(is_api_path("/ws"));
        assert!(is_api_path("/mcp"));
    }

    #[test]
    fn profiling_subpaths_are_spa() {
        assert!(!is_api_path("/profiling"));
        assert!(!is_api_path("/profiling/pprof"));
        assert!(!is_api_path("/profiling/torch"));
        assert!(!is_api_path("/profiling/trace"));
        assert!(!is_api_path("/profiling/pytorch"));
        assert!(!is_api_path("/profiling/ray"));
        assert!(!is_api_path("/stacks/12345"));
        assert!(!is_api_path("/spans"));
        assert!(!is_api_path("/traces"));
    }

    #[test]
    fn static_asset_extensions_are_not_spa() {
        assert!(is_static_asset_path("/assets/tailwind.css"));
        assert!(is_static_asset_path("/./assets/web-dxhabc.js"));
        assert!(!is_static_asset_path("/profiling"));
    }
}
