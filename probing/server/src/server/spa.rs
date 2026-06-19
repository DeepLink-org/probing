//! Dioxus SPA shell: serve `index.html` for client-side routes, static files otherwise.

use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Router};

use crate::asset::{contains, index, static_files};

/// True for backend/API endpoints that must not return the SPA shell.
pub fn is_api_path(path: &str) -> bool {
    path == "/query"
        || path == "/query/dto"
        || path.starts_with("/apis/")
        || path.starts_with("/config/")
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

    if contains(path) {
        if let Ok(resp) = static_files(uri, headers).await {
            return resp.into_response();
        }
    }

    index().await.into_response()
}

#[cfg(test)]
mod tests {
    use super::is_api_path;

    #[test]
    fn api_paths_are_not_spa() {
        assert!(is_api_path("/query"));
        assert!(is_api_path("/query/dto"));
        assert!(is_api_path("/apis/nodes"));
        assert!(is_api_path("/config/server.address"));
        assert!(is_api_path("/ws"));
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
}
