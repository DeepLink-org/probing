use axum::{routing::get, Router};

use crate::asset::index;

const PAGE_PATHS: &[&str] = &[
    "/",
    "/overview",
    "/cluster",
    "/stacks",
    "/profiling",
    "/analytics",
    "/python",
    "/traces",
    "/chrome-tracing",
    "/pulsing",
    "/index.html",
];

/// Dioxus SPA shell: every listed path serves the same `index.html`.
pub fn routes() -> Router {
    let mut router = Router::new();
    for path in PAGE_PATHS {
        router = router.route(path, get(index));
    }
    router
}
