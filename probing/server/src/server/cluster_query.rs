//! HTTP handler for on-demand cluster SQL fan-out.

use axum::Json;
use serde::{Deserialize, Serialize};

use super::cluster_fanout::{self, FanoutQueryResponse};
use super::error::ApiResult;

#[derive(Debug, Deserialize, Serialize)]
pub struct ClusterQueryRequest {
    pub expr: String,
    #[serde(default)]
    pub cluster: bool,
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
    let expr = body.expr.trim();
    if expr.is_empty() {
        return Err(super::error::ApiError::bad_request(
            "missing SQL expression",
        ));
    }
    let result = cluster_fanout::fanout_query(expr, body.cluster).await?;
    Ok(Json(result.into()))
}
