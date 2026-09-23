//! `/apis` routing: public endpoints first, extension fallback for everything else.
//!
//! See `probing/server/API.md` for the routing policy.

pub mod extension;
pub mod response;

use axum::{
    routing::{get, post},
    Router,
};

use super::{cluster, cluster_query, file_api, local_query, rl, system, training};

/// Canonical public `/apis` routes (method, path suffix under `/apis`).
/// Keep in sync with `tests/regression/spec/api_spec.json` — verified by `spec_tests`.
pub const PUBLIC_API_ROUTES: &[(&str, &str)] = &[
    ("GET", "/overview"),
    ("GET", "/files"),
    ("GET", "/nodes"),
    ("PUT", "/nodes"),
    ("GET", "/training/step_matrix"),
    ("GET", "/rl/runs"),
    ("GET", "/rl/status"),
    ("GET", "/rl/tags"),
    ("GET", "/rl/series"),
    ("GET", "/rl/samples"),
    ("GET", "/rl/sampler"),
    ("GET", "/rl/composition"),
    ("GET", "/rl/datasets"),
    ("GET", "/rl/pass_histogram"),
    ("GET", "/rl/staleness"),
    ("GET", "/rl/benchmarks"),
    ("GET", "/rl/events"),
    ("GET", "/rl/about"),
    ("POST", "/cluster/query"),
    ("GET", "/processes/local"),
    ("POST", "/query/local-pid"),
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
        .route("/rl/runs", get(rl::get_runs))
        .route("/rl/status", get(rl::get_status))
        .route("/rl/tags", get(rl::get_tags))
        .route("/rl/series", get(rl::get_series))
        .route("/rl/samples", get(rl::get_samples))
        .route("/rl/sampler", get(rl::get_sampler))
        .route("/rl/composition", get(rl::get_composition))
        .route("/rl/datasets", get(rl::get_datasets))
        .route("/rl/pass_histogram", get(rl::get_pass_histogram))
        .route("/rl/staleness", get(rl::get_staleness))
        .route("/rl/benchmarks", get(rl::get_benchmarks))
        .route("/rl/events", get(rl::get_events))
        .route("/rl/about", get(rl::get_about))
        .route("/cluster/query", post(cluster_query::post_cluster_query))
        .route("/processes/local", get(system::get_local_processes_json))
        .route("/query/local-pid", post(local_query::query_local_pid))
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
