use axum::{
    extract::Query,
    response::{IntoResponse, Response},
};

use super::error::ApiResult;

#[derive(Debug, serde::Deserialize)]
pub struct FlamegraphFormatQuery {
    format: Option<String>,
    metric: Option<String>,
}

const FLAMEGRAPH_HTML_HEADERS: [(&str, &str); 2] = [
    ("Content-Type", "text/html; charset=utf-8"),
    ("Content-Disposition", "inline; filename=flamegraph.html"),
];

const FLAMEGRAPH_JSON_HEADERS: [(&str, &str); 1] =
    [("Content-Type", "application/json; charset=utf-8")];

fn wants_json(format: &Option<String>) -> bool {
    format.as_deref() == Some("json")
}

/// Generate interactive HTML or JSON flamegraph for torch profiler data.
pub async fn get_torch_flamegraph(
    Query(query): Query<FlamegraphFormatQuery>,
) -> ApiResult<Response> {
    if wants_json(&query.format) {
        let json = probing_python::features::torch::flamegraph_json(query.metric.as_deref());
        Ok((FLAMEGRAPH_JSON_HEADERS, json).into_response())
    } else {
        let graph = probing_python::features::torch::flamegraph();
        Ok((FLAMEGRAPH_HTML_HEADERS, graph).into_response())
    }
}

/// Generate interactive HTML or JSON flamegraph for CPU sampling (pprof).
pub async fn get_pprof_flamegraph(
    Query(query): Query<FlamegraphFormatQuery>,
) -> ApiResult<Response> {
    if wants_json(&query.format) {
        let json = probing_python::features::pprof::flamegraph_json();
        Ok((FLAMEGRAPH_JSON_HEADERS, json).into_response())
    } else {
        match probing_python::features::pprof::flamegraph() {
            Ok(graph) => Ok((FLAMEGRAPH_HTML_HEADERS, graph).into_response()),
            Err(err) => Err(anyhow::anyhow!(err).into()),
        }
    }
}
