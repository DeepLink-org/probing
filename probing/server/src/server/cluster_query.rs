//! HTTP handler for on-demand cluster SQL fan-out.

use axum::Json;
use serde::{Deserialize, Serialize};

use super::cluster_fanout::{self, ClusterFanoutScope, FanoutQueryResponse};
use super::error::ApiResult;
use probing_core::core::cluster::is_hierarchical_metadata_unavailable;
use probing_core::core::federation::validate_global_query;

#[derive(Debug, Deserialize, Serialize)]
pub struct ClusterQueryRequest {
    pub expr: String,
    #[serde(default)]
    pub cluster: bool,
    /// Hierarchical fan-out: coordinator → node local0 → on-node leaf ranks.
    #[serde(default = "default_true")]
    pub hierarchical: bool,
    /// Override fan-out tier (`auto` picks coordinator on local0, local on leaf ranks).
    #[serde(default)]
    pub scope: ClusterFanoutScope,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct ClusterQueryResponse {
    pub dataframe: probing_proto::prelude::DataFrame,
    pub meta: cluster_fanout::FanoutMeta,
}

impl From<FanoutQueryResponse> for ClusterQueryResponse {
    fn from(value: FanoutQueryResponse) -> Self {
        Self {
            dataframe: value.dataframe,
            meta: value.meta,
        }
    }
}

pub async fn post_cluster_query(
    Json(body): Json<ClusterQueryRequest>,
) -> ApiResult<Json<ClusterQueryResponse>> {
    if let Some(msg) = crate::engine_lifecycle::engine_not_ready_message() {
        return Err(super::error::ApiError::service_unavailable(msg));
    }
    let expr = body.expr.trim();
    if expr.is_empty() {
        return Err(super::error::ApiError::bad_request(
            "missing SQL expression",
        ));
    }
    validate_global_query(expr).map_err(|e| super::error::ApiError::bad_request(e.to_string()))?;
    match cluster_fanout::fanout_query(expr, body.cluster, body.hierarchical, body.scope).await {
        Ok(result) => Ok(Json(result.into())),
        Err(err) if is_hierarchical_metadata_unavailable(&err) => {
            Err(super::error::ApiError::service_unavailable(err.to_string()))
        }
        Err(err) => Err(err.into()),
    }
}
