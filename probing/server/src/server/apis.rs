use axum::{routing::get, routing::post, Router};

use super::{cluster, extension_handler, file_api, profiling, system};

/// Main router for all API endpoints
pub fn apis_route() -> Router {
    Router::new()
        .route("/overview", get(system::get_overview_json))
        .route("/files", get(file_api::read_file))
        .route("/nodes", get(cluster::get_nodes).put(cluster::put_node))
        .route("/nodes/sync", post(cluster::post_sync_nodes))
        .route("/flamegraph/torch", get(profiling::get_torch_flamegraph))
        .route("/flamegraph/pprof", get(profiling::get_pprof_flamegraph))
        .fallback(extension_handler::handle_extension_call)
}
