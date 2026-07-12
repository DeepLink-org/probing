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
    base.names.push("_host".to_string());
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
            coord_step: 10,
            duration_ms: 99.5,
            host: "node-a".into(),
            addr: "10.0.0.1:8080".into(),
        }],
        rank_count: 1,
        step_count: 1,
        cluster: false,
        partial: false,
        nodes_queried: 1,
        nodes_failed: vec![],
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("nodes_queried"));
    assert!(json.contains("duration_ms"));
    assert!(json.contains("partial"));
}

#[test]
fn cluster_query_request_roundtrip() {
    use probing_server::server::cluster_fanout::ClusterFanoutScope;
    use probing_server::server::cluster_query::ClusterQueryRequest;

    let req = ClusterQueryRequest {
        expr: "SELECT 1".into(),
        cluster: true,
        hierarchical: true,
        scope: ClusterFanoutScope::Auto,
    };
    let body = serde_json::to_string(&req).unwrap();
    let back: ClusterQueryRequest = serde_json::from_str(&body).unwrap();
    assert!(back.cluster);
    assert!(back.hierarchical);
    assert_eq!(back.expr, "SELECT 1");
    assert_eq!(back.scope, ClusterFanoutScope::Auto);
}

#[test]
fn distributed_torch_sql_valid_for_global_fanout() {
    use probing_core::core::federation::validate_global_query;
    use probing_server::server::training::distributed_torch_trace_sql;

    let sql = distributed_torch_trace_sql("global.python.torch_trace", Some(42));
    assert!(sql.contains("local_step = 42"));
    assert!(sql.contains("COALESCE(rank, 0)"));
    assert!(validate_global_query(&sql).is_ok());
    let latest = distributed_torch_trace_sql("global.python.torch_trace", None);
    assert!(latest.contains("max(local_step)"));
}

#[test]
fn distributed_flamegraph_json_contract_from_dataframe() {
    use probing_proto::types::{DataFrame, Seq};
    use probing_python::features::torch::distributed_flamegraph_json_from_df;

    let df = DataFrame::new(
        vec![
            "rank".into(),
            "module".into(),
            "stage".into(),
            "duration".into(),
            "local_step".into(),
        ],
        vec![
            Seq::SeqI64(vec![0, 1]),
            Seq::SeqText(vec!["block".into(), "block".into()]),
            Seq::SeqText(vec!["post forward".into(), "post forward".into()]),
            Seq::SeqF64(vec![0.002, 0.002]),
            Seq::SeqI64(vec![11, 11]),
        ],
    );

    let json = distributed_flamegraph_json_from_df(&df, None);
    let payload: serde_json::Value = serde_json::from_str(&json).expect("parse json");
    assert_eq!(payload["profile"], "torch-distributed");
    assert_eq!(payload["metric"], "duration");
    assert_eq!(payload["countName"], "ns");
    assert!(payload["subtitle"]
        .as_str()
        .unwrap_or("")
        .contains("local_step 11"));
    assert!(payload["subtitle"]
        .as_str()
        .unwrap_or("")
        .contains("2 ranks"));
    assert!(payload["frames"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false));
    assert_eq!(payload["total"], 4_000_000);
}

#[test]
fn distributed_flamegraph_params_roundtrip() {
    use probing_server::server::training::DistributedFlamegraphParams;

    let params: DistributedFlamegraphParams =
        serde_json::from_str(r#"{"step":5,"metric":"peak_mb","cluster":false}"#)
            .expect("deserialize");
    assert_eq!(params.step, Some(5));
    assert_eq!(params.metric.as_deref(), Some("peak_mb"));
    assert_eq!(params.cluster, Some(false));
}
