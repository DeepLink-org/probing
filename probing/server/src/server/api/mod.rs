//! `/apis` routing: public endpoints first, extension fallback for everything else.
//!
//! See `probing/server/API.md` for the routing policy.

pub mod extension;
pub mod response;

use axum::{
    routing::{get, post},
    Router,
};

use super::{cluster, cluster_query, file_api, system, training};

/// Canonical public `/apis` routes (method, path suffix under `/apis`).
/// Keep in sync with `tests/regression/spec/api_spec.json` — verified by `spec_tests`.
pub const PUBLIC_API_ROUTES: &[(&str, &str)] = &[
    ("GET", "/overview"),
    ("GET", "/files"),
    ("GET", "/nodes"),
    ("PUT", "/nodes"),
    ("GET", "/training/step_matrix"),
    ("GET", "/training/distributed_flamegraph/json"),
    ("GET", "/training/distributed_stack_flamegraph/json"),
    ("POST", "/cluster/query"),
];

/// Build the `/apis` router mounted by the root application.
pub fn router() -> Router {
    public_routes().fallback(extension::handle)
}

/// Stable platform endpoints with explicit Axum handlers.
fn public_routes() -> Router {
    Router::new()
        .route("/overview", get(system::get_overview_json))
        .route("/files", get(file_api::read_file))
        .route("/nodes", get(cluster::get_nodes).put(cluster::put_node))
        .route("/training/step_matrix", get(training::get_step_matrix))
        .route(
            "/training/distributed_flamegraph/json",
            get(training::get_distributed_flamegraph_json),
        )
        .route(
            "/training/distributed_stack_flamegraph/json",
            get(training::get_distributed_stack_flamegraph_json),
        )
        .route("/cluster/query", post(cluster_query::post_cluster_query))
}

#[cfg(test)]
mod spec_tests {
    use super::PUBLIC_API_ROUTES;

    fn load_spec() -> serde_json::Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/regression/spec/api_spec.json");
        let text = std::fs::read_to_string(path).expect("read api_spec.json");
        serde_json::from_str(&text).expect("parse api_spec.json")
    }

    #[test]
    fn public_routes_match_api_spec() {
        let spec = load_spec();
        let expected: Vec<(String, String)> = spec["server_public"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                let method = entry["method"].as_str().unwrap().to_string();
                let full = entry["path"].as_str().unwrap();
                let suffix = full.strip_prefix("/apis").unwrap();
                (method, suffix.to_string())
            })
            .collect();

        let actual: Vec<(String, String)> = PUBLIC_API_ROUTES
            .iter()
            .map(|(m, p)| (m.to_string(), p.to_string()))
            .collect();

        assert_eq!(actual, expected);
    }
}
