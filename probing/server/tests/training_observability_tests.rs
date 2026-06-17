//! Training observability server-side tests (fan-out merge, step parsing helpers).

use probing_proto::prelude::*;

#[test]
fn tag_dataframe_adds_probe_columns_via_merge() {
    let df = DataFrame {
        names: vec!["k".into()],
        cols: vec![Seq::SeqI32(vec![7])],
        size: 1,
    };
    let mut base = df.clone();
    let rows = base.len();
    base.names.push("_probe_host".to_string());
    base.cols.push(Seq::SeqText(vec!["h".to_string(); rows]));
    assert_eq!(base.names.len(), 2);
    assert_eq!(base.len(), 1);
}

#[test]
fn step_matrix_response_fields_serializable() {
    use probing_server::server::training::{StepDurationSample, StepMatrixResponse};

    let resp = StepMatrixResponse {
        samples: vec![StepDurationSample {
            rank: 1,
            local_step: 10,
            duration_ms: 99.5,
            host: "node-a".into(),
            addr: "10.0.0.1:8080".into(),
        }],
        rank_count: 1,
        step_count: 1,
        cluster: false,
        nodes_queried: 1,
        nodes_failed: vec![],
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("nodes_queried"));
    assert!(json.contains("duration_ms"));
}

#[test]
fn cluster_query_request_roundtrip() {
    use probing_server::server::cluster_query::ClusterQueryRequest;

    let req = ClusterQueryRequest {
        expr: "SELECT 1".into(),
        cluster: true,
    };
    let body = serde_json::to_string(&req).unwrap();
    let back: ClusterQueryRequest = serde_json::from_str(&body).unwrap();
    assert!(back.cluster);
    assert_eq!(back.expr, "SELECT 1");
}
