//! Distributed torch flamegraph regression (SPMD merge at one ``local_step``).

use probing_proto::prelude::*;
use probing_python::features::torch::distributed_flamegraph_json_from_df;
use serde_json::Value;

fn spmd_duration_df(step: i64) -> DataFrame {
    DataFrame::new(
        vec![
            "rank".into(),
            "module".into(),
            "stage".into(),
            "duration".into(),
            "local_step".into(),
        ],
        vec![
            Seq::SeqI64(vec![0, 1, 0, 1]),
            Seq::SeqText(vec![
                "encoder".into(),
                "encoder".into(),
                "decoder".into(),
                "decoder.head".into(),
            ]),
            Seq::SeqText(vec![
                "post forward".into(),
                "post forward".into(),
                "post forward".into(),
                "post forward".into(),
            ]),
            Seq::SeqF64(vec![0.01, 0.01, 0.02, 0.008]),
            Seq::SeqI64(vec![step, step, step, step]),
        ],
    )
}

fn parse_json(json: &str) -> Value {
    serde_json::from_str(json).expect("valid flamegraph json")
}

#[test]
fn distributed_flamegraph_merges_shared_encoder_across_ranks() {
    let json = distributed_flamegraph_json_from_df(&spmd_duration_df(6), None);
    let payload = parse_json(&json);
    assert_eq!(payload["profile"], "torch-distributed");
    // encoder merged (2 × 10ms) + per-rank decoder paths
    assert_eq!(payload["total"], 48_000_000);
    let frames = payload["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty());
}

#[test]
fn distributed_flamegraph_peak_mb_metric() {
    let df = DataFrame::new(
        vec![
            "rank".into(),
            "module".into(),
            "stage".into(),
            "max_allocated_delta".into(),
            "local_step".into(),
        ],
        vec![
            Seq::SeqI64(vec![0, 1]),
            Seq::SeqText(vec!["layer".into(), "layer".into()]),
            Seq::SeqText(vec!["post forward".into(), "post forward".into()]),
            Seq::SeqF64(vec![0.5, 0.5]),
            Seq::SeqI64(vec![3, 3]),
        ],
    );

    let payload = parse_json(&distributed_flamegraph_json_from_df(&df, Some("peak_mb")));
    assert_eq!(payload["metric"], "peak_mb");
    assert_eq!(payload["countName"], "MB");
    assert_eq!(payload["total"], 1_000_000);
}

#[test]
fn distributed_flamegraph_empty_step_has_actionable_message() {
    let df = DataFrame::new(
        vec![
            "rank".into(),
            "module".into(),
            "stage".into(),
            "local_step".into(),
        ],
        vec![
            Seq::SeqI64(vec![0]),
            Seq::SeqText(vec!["m".into()]),
            Seq::SeqText(vec!["pre forward".into()]),
            Seq::SeqI64(vec![1]),
        ],
    );

    let payload = parse_json(&distributed_flamegraph_json_from_df(&df, None));
    assert_eq!(payload["total"], 0);
    assert!(payload["emptyMessage"]
        .as_str()
        .unwrap_or("")
        .contains("torch_trace"));
}
