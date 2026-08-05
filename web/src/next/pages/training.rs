use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Node};

use crate::api::ApiClient;
use crate::components::dataframe_view::DataFrameView;
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::page_context::publish_page_evidence;
use crate::state::training::{
    placement_availability, TRAINING_CLUSTER_SCOPE, TRAINING_PLACEMENT_AVAILABILITY,
    TRAINING_REFRESH,
};
use crate::utils::error::Result;

use super::super::capabilities::{capability_status, CapabilityStatus};
use super::super::components::{
    EvidenceMetric, EvidenceSection, EvidenceSurface, InlineNotice, LoadingPanel, NoticeTone,
    UnavailablePanel, WorkspacePage,
};
use super::super::evidence::{
    cluster_dataframe_payload, dataframe_preview, step_matrix_payload, EvidenceBundle,
    EvidencePayload, EvidenceReceipt, EvidenceRequest, EvidenceScope,
};
use super::super::model::{format_duration, StepHealth};
use super::dashboard::StepTrendChart;
use super::training_placement::TrainingPlacement;

const POLL_MS: u32 = 5_000;
const STEP_LIMIT: usize = 120;

const MODULE_HOTSPOTS_SQL: &str = "SELECT module, stage, count(DISTINCT local_step) AS steps, \
     count(*) AS hooks, round(avg(duration), 4) AS avg_sec, round(sum(duration), 4) AS total_sec \
     FROM python.torch_trace \
     WHERE local_step >= GREATEST(COALESCE((SELECT max(local_step) FROM python.torch_trace), 0) - 9, 1) \
       AND stage LIKE 'post %' AND duration > 0 \
       AND module IS NOT NULL AND module != '' AND module != 'None' \
     GROUP BY module, stage ORDER BY total_sec DESC LIMIT 12";

const COMM_SUMMARY_SQL: &str = "SELECT op, count(*) AS calls, \
     round(avg(duration_ms), 2) AS avg_ms, round(max(duration_ms), 2) AS max_ms, \
     sum(bytes) AS total_bytes \
     FROM python.comm_collective GROUP BY op ORDER BY avg_ms DESC LIMIT 12";

const GROUP_COMMUNICATION_SQL: &str = "SELECT rank, op, group_size, participate_ranks, \
     count(*) AS calls, round(avg(duration_ms), 3) AS avg_ms, \
     round(max(duration_ms), 3) AS max_ms, sum(bytes) AS total_bytes \
     FROM python.comm_collective \
     WHERE participate_ranks IS NOT NULL AND participate_ranks != '' \
       AND global_step >= GREATEST(COALESCE((SELECT max(global_step) \
         FROM python.comm_collective), 0) - 19, 0) \
     GROUP BY rank, op, group_size, participate_ranks \
     ORDER BY max_ms DESC LIMIT 768";

const RANK_MEMORY_SQL: &str = "SELECT rank, local_step, allocated, max_allocated, cached \
     FROM python.torch_trace \
     WHERE rank >= 0 AND allocated >= 0 AND stage LIKE 'post %' \
     ORDER BY local_step DESC, seq DESC LIMIT 1";

const DEVICE_MEMORY_SQL: &str = "WITH samples AS ( \
       SELECT device_id, used_bytes, total_bytes, ts, \
         CAST(COALESCE((SELECT value FROM process.envs WHERE name = 'LOCAL_RANK' LIMIT 1), '-1') AS INT) AS local_rank, \
         MAX(used_bytes) OVER (PARTITION BY device_id) AS peak_used_bytes, \
         COUNT(*) OVER (PARTITION BY device_id) AS sample_count, \
         ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY ts DESC) AS recency \
       FROM gpu.utilization \
       WHERE ts >= GREATEST(COALESCE((SELECT MAX(ts) FROM gpu.utilization), 0) - 300000000, 0) \
     ) SELECT CAST(COALESCE((SELECT value FROM process.envs WHERE name = 'RANK' LIMIT 1), '-1') AS INT) AS rank, \
         device_id, used_bytes AS current_used_bytes, peak_used_bytes, \
         total_bytes, sample_count FROM samples WHERE recency = 1 \
         AND (local_rank < 0 OR device_id = local_rank) \
       ORDER BY device_id LIMIT 16";

#[component]
pub fn TrainingPage() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let refresh_key = use_memo(move || poll().wrapping_add(*TRAINING_REFRESH.read()));
    let evidence_request = use_memo(move || {
        EvidenceRequest::new(
            u64::from(refresh_key()),
            if *TRAINING_CLUSTER_SCOPE.read() {
                EvidenceScope::ClusterFanout
            } else {
                EvidenceScope::LocalProcess
            },
            None,
            INVESTIGATION_CONTEXT.read().clone(),
        )
    });

    let steps = use_resource(move || {
        let request = evidence_request();
        let cluster = request.scope == EvidenceScope::ClusterFanout;
        let available = capability_status(
            "python",
            "trace_event",
            &["record_type", "span_id", "name", "time"],
        );
        async move {
            let matrix = if available.allows_query() {
                ApiClient::new()
                    .fetch_step_matrix(STEP_LIMIT, cluster)
                    .await
            } else {
                Ok(empty_step_matrix(cluster))
            }?;
            Ok(step_matrix_payload(matrix, &request))
        }
    });
    let nodes = use_resource(move || {
        let request = evidence_request().for_scope(EvidenceScope::ClusterRegistry, None);
        async move {
            let nodes = ApiClient::new().get_nodes().await?;
            let receipt = EvidenceReceipt::local("cluster.nodes", &request, nodes.len());
            Ok(EvidencePayload::new(nodes, receipt))
        }
    });
    let modules = use_resource(move || {
        let request = evidence_request();
        let available =
            capability_status("python", "torch_trace", &["local_step", "module", "stage"]);
        async move {
            if available.allows_query() {
                training_query("python.torch_trace modules", MODULE_HOTSPOTS_SQL, &request).await
            } else {
                Ok(empty_dataframe_payload(
                    "python.torch_trace modules",
                    &request,
                ))
            }
        }
    });
    let collectives = use_resource(move || {
        let request = evidence_request();
        let available =
            capability_status("python", "comm_collective", &["op", "duration_ms", "bytes"]);
        async move {
            if available.allows_query() {
                training_query("python.comm_collective summary", COMM_SUMMARY_SQL, &request).await
            } else {
                Ok(empty_dataframe_payload(
                    "python.comm_collective summary",
                    &request,
                ))
            }
        }
    });
    let group_communication = use_resource(move || {
        let request = evidence_request();
        let available = capability_status(
            "python",
            "comm_collective",
            &["rank", "group_size", "participate_ranks", "duration_ms"],
        );
        async move {
            if available.allows_query() {
                training_query(
                    "python.comm_collective groups",
                    GROUP_COMMUNICATION_SQL,
                    &request,
                )
                .await
            } else {
                Ok(empty_dataframe_payload(
                    "python.comm_collective groups",
                    &request,
                ))
            }
        }
    });
    let rank_memory = use_resource(move || {
        let request = evidence_request();
        let available = capability_status(
            "python",
            "torch_trace",
            &["rank", "local_step", "allocated", "max_allocated", "cached"],
        );
        async move {
            if available.allows_query() {
                training_query("python.torch_trace allocator", RANK_MEMORY_SQL, &request).await
            } else {
                Ok(empty_dataframe_payload(
                    "python.torch_trace allocator",
                    &request,
                ))
            }
        }
    });
    let device_memory = use_resource(move || {
        let request = evidence_request();
        async move { training_query("gpu.utilization memory", DEVICE_MEMORY_SQL, &request).await }
    });

    let step_state = steps.read().clone();
    let node_state = nodes.read().clone();
    let module_state = modules.read().clone();
    let collective_state = collectives.read().clone();
    let group_communication_state = group_communication.read().clone();
    let rank_memory_state = rank_memory.read().clone();
    let device_memory_state = device_memory.read().clone();
    let step_health = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|payload| StepHealth::from_matrix(&payload.value));
    let latest_step = step_health.as_ref().and_then(|health| health.latest_step);
    let placement_state = payload_value_state(&node_state);
    use_effect(move || {
        *TRAINING_PLACEMENT_AVAILABILITY.write() = placement_availability(placement_state.as_ref());
    });
    let page_request = evidence_request();
    let bundle_step_state = step_state.clone();
    let bundle_node_state = node_state.clone();
    let bundle_module_state = module_state.clone();
    let bundle_collective_state = collective_state.clone();
    let bundle_group_state = group_communication_state.clone();
    let bundle_rank_memory_state = rank_memory_state.clone();
    let bundle_device_memory_state = device_memory_state.clone();
    use_effect(move || {
        if let Some(snapshot) = training_evidence_bundle(
            &page_request,
            bundle_step_state.as_ref(),
            bundle_node_state.as_ref(),
            bundle_module_state.as_ref(),
            bundle_collective_state.as_ref(),
            bundle_group_state.as_ref(),
            bundle_rank_memory_state.as_ref(),
            bundle_device_memory_state.as_ref(),
        ) {
            publish_page_evidence(
                "training",
                &crate::state::investigation::investigation_context_key(&page_request.context),
                page_request.requested_at_ms,
                snapshot,
            );
        }
    });
    let step_view_state = payload_value_state(&step_state);
    let group_view_state = payload_value_state(&group_communication_state);
    let rank_memory_view_state = payload_value_state(&rank_memory_state);
    let device_memory_view_state = payload_value_state(&device_memory_state);
    let placement_partial_sources = [
        partial_source("communication groups", &group_communication_state),
        partial_source("allocator", &rank_memory_state),
        partial_source("device memory", &device_memory_state),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let placement_partial_label = placement_partial_sources.join(" · ");
    let scope = evidence_request().scope.label();
    let step_source = capability_status(
        "python",
        "trace_event",
        &["record_type", "span_id", "name", "time"],
    );
    let torch_source =
        capability_status("python", "torch_trace", &["local_step", "module", "stage"]);
    let collective_source =
        capability_status("python", "comm_collective", &["op", "duration_ms", "bytes"]);
    let missing_sources = [
        (step_source == CapabilityStatus::Missing).then_some("step spans"),
        (torch_source == CapabilityStatus::Missing).then_some("module and allocator hooks"),
        (collective_source == CapabilityStatus::Missing).then_some("collective samples"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let dual_optional =
        torch_source != CapabilityStatus::Missing && collective_source != CapabilityStatus::Missing;
    let has_placement =
        matches!(node_state.as_ref(), Some(Ok(payload)) if !payload.value.is_empty());
    let has_evidence = step_source != CapabilityStatus::Missing
        || torch_source != CapabilityStatus::Missing
        || collective_source != CapabilityStatus::Missing
        || has_placement;

    rsx! {
        WorkspacePage {
            title: "Training".to_string(),
            subtitle: "Step timing, physical rank placement, module hooks, and collective measurements.".to_string(),
            actions: rsx! { span { class: "text-xs text-gray-500", "{scope} · {POLL_MS / 1000}s" } },

            if !missing_sources.is_empty() {
                InlineNotice {
                    title: "Sources not reported".to_string(),
                    detail: missing_sources.join(" · "),
                    tone: NoticeTone::Info,
                }
            }

            if has_evidence {
                EvidenceSurface {
                if step_source != CapabilityStatus::Missing {
                    EvidenceSection {
                        title: "Step time".to_string(),
                        subtitle: Some("Latest rank samples and recent median/P95 trend.".to_string()),
                        StepEvidence { state: step_view_state }
                    }
                }

                if let Some(Ok(payload)) = node_state.as_ref() {
                    if !payload.value.is_empty() {
                        EvidenceSection {
                            title: "Placement".to_string(),
                            subtitle: Some("One square per reported accelerator process; selecting a rank links its node, step, and exact communication-group evidence.".to_string()),
                            divided: true,
                            if !placement_partial_sources.is_empty() {
                                div { class: "border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-900",
                                    "Partial cluster evidence · {placement_partial_label}"
                                }
                            }
                            TrainingPlacement {
                                nodes: payload.value.clone(),
                                local_step: latest_step,
                                step_health: step_health.clone(),
                                group_communication: group_view_state.clone(),
                                rank_memory: rank_memory_view_state.clone(),
                                device_memory: device_memory_view_state.clone(),
                            }
                        }
                    }
                }

                if torch_source != CapabilityStatus::Missing || collective_source != CapabilityStatus::Missing {
                    div { class: if dual_optional { "grid items-start border-t border-gray-200 xl:grid-cols-2" } else { "border-t border-gray-200" },
                        if torch_source != CapabilityStatus::Missing {
                            EvidenceSection {
                                title: "Module hotspots".to_string(),
                                subtitle: Some("Measured post-hook duration over the latest ten reported steps.".to_string()),
                                body_class: "p-0".to_string(),
                                EvidenceTable {
                                    state: module_state,
                                    loading: "Loading module hooks",
                                    empty: "No module hook samples",
                                }
                            }
                        }
                        if collective_source != CapabilityStatus::Missing {
                            div { class: if dual_optional { "border-t border-gray-200 xl:border-l xl:border-t-0" } else { "" },
                                EvidenceSection {
                                    title: "Collective communication".to_string(),
                                    subtitle: Some("Call count, average, maximum, and bytes grouped by reported operation.".to_string()),
                                    body_class: "p-0".to_string(),
                                    EvidenceTable {
                                        state: collective_state,
                                        loading: "Loading collective samples",
                                        empty: "No collective samples",
                                    }
                                }
                            }
                        }
                    }
                }
                }
            }
        }
    }
}

fn empty_step_matrix(cluster: bool) -> crate::api::StepMatrixResponse {
    crate::api::StepMatrixResponse {
        samples: Vec::new(),
        rank_count: 0,
        step_count: 0,
        cluster,
        partial: false,
        nodes_queried: 0,
        nodes_failed: Vec::new(),
    }
}

async fn training_query(
    source: &'static str,
    sql: &str,
    request: &EvidenceRequest,
) -> Result<EvidencePayload<DataFrame>> {
    let client = ApiClient::new();
    if request.scope == EvidenceScope::ClusterFanout {
        let response = client.cluster_query(sql, true).await?;
        Ok(cluster_dataframe_payload(
            source,
            request,
            response.dataframe,
            response.meta.nodes_queried,
            response.meta.nodes_failed.len(),
            response.meta.partial,
        ))
    } else {
        let dataframe = client.execute_query(sql).await?;
        let receipt = EvidenceReceipt::local(source, request, dataframe_rows(&dataframe));
        Ok(EvidencePayload::new(dataframe, receipt))
    }
}

fn empty_dataframe_payload(
    source: &'static str,
    request: &EvidenceRequest,
) -> EvidencePayload<DataFrame> {
    let receipt = if request.scope == EvidenceScope::ClusterFanout {
        EvidenceReceipt::cluster(source, request, 0, 0, 0, false)
    } else {
        EvidenceReceipt::local(source, request, 0)
    };
    EvidencePayload::new(DataFrame::default(), receipt)
}

fn payload_value_state<T: Clone>(state: &Option<Result<EvidencePayload<T>>>) -> Option<Result<T>> {
    state
        .clone()
        .map(|result| result.map(|payload| payload.value))
}

fn partial_source(
    label: &'static str,
    state: &Option<Result<EvidencePayload<DataFrame>>>,
) -> Option<String> {
    let payload = state.as_ref()?.as_ref().ok()?;
    (payload.receipt.partial || payload.receipt.failed_peers > 0)
        .then(|| format!("{label}: {} failed peer(s)", payload.receipt.failed_peers))
}

#[allow(clippy::too_many_arguments)]
fn training_evidence_bundle(
    request: &EvidenceRequest,
    steps: Option<&Result<EvidencePayload<crate::api::StepMatrixResponse>>>,
    nodes: Option<&Result<EvidencePayload<Vec<Node>>>>,
    modules: Option<&Result<EvidencePayload<DataFrame>>>,
    collectives: Option<&Result<EvidencePayload<DataFrame>>>,
    groups: Option<&Result<EvidencePayload<DataFrame>>>,
    rank_memory: Option<&Result<EvidencePayload<DataFrame>>>,
    device_memory: Option<&Result<EvidencePayload<DataFrame>>>,
) -> Option<String> {
    let (steps, nodes, modules, collectives, groups, rank_memory, device_memory) = (
        steps?,
        nodes?,
        modules?,
        collectives?,
        groups?,
        rank_memory?,
        device_memory?,
    );
    let mut bundle = EvidenceBundle::new("training", request);
    match steps {
        Ok(payload) => bundle.push(
            &payload.receipt,
            super::super::page_snapshot::format_step_matrix(&payload.value, request),
        ),
        Err(error) => bundle.push_failure("train.step", &error.display_message()),
    }
    match nodes {
        Ok(payload) => bundle.push(
            &payload.receipt,
            super::super::page_snapshot::format_nodes(&payload.value),
        ),
        Err(error) => bundle.push_failure("cluster.nodes", &error.display_message()),
    }
    push_dataframe_result(&mut bundle, "python.torch_trace modules", modules, 12);
    push_dataframe_result(
        &mut bundle,
        "python.comm_collective summary",
        collectives,
        12,
    );
    push_dataframe_result(&mut bundle, "python.comm_collective groups", groups, 24);
    push_dataframe_result(&mut bundle, "python.torch_trace allocator", rank_memory, 8);
    push_dataframe_result(&mut bundle, "gpu.utilization memory", device_memory, 16);
    Some(bundle.render())
}

fn push_dataframe_result(
    bundle: &mut EvidenceBundle,
    source: &'static str,
    result: &Result<EvidencePayload<DataFrame>>,
    max_rows: usize,
) {
    match result {
        Ok(payload) => bundle.push(
            &payload.receipt,
            dataframe_preview(&payload.value, max_rows),
        ),
        Err(error) => bundle.push_failure(source, &error.display_message()),
    }
}

#[component]
fn StepEvidence(state: Option<Result<crate::api::StepMatrixResponse>>) -> Element {
    match state {
        None => rsx! { LoadingPanel { label: "Loading step samples".to_string() } },
        Some(Err(error)) => rsx! { UnavailablePanel {
            label: "Step samples unavailable".to_string(),
            detail: error.display_message(),
        }},
        Some(Ok(matrix)) if matrix.samples.is_empty() => rsx! { UnavailablePanel {
            label: "No train.step samples".to_string(),
            detail: "The selected scope returned no completed step spans.".to_string(),
        }},
        Some(Ok(matrix)) => {
            let health = StepHealth::from_matrix(&matrix);
            let latest = health
                .latest_step
                .map(|step| step.to_string())
                .unwrap_or_else(|| "—".to_string());
            let maximum = health
                .slowest_ms
                .map(|value| format_duration(Some(value)))
                .unwrap_or_else(|| "—".to_string());
            let maximum_detail = health.slowest_rank.map(|rank| format!("rank {rank}"));
            rsx! {
                div { class: "space-y-4",
                    div { class: "grid grid-cols-4 divide-x divide-gray-200",
                        EvidenceMetric { label: "Latest step", value: latest, detail: None }
                        EvidenceMetric { label: "Median", value: format_duration(health.median_ms), detail: None }
                        EvidenceMetric { label: "P95", value: format_duration(health.p95_ms), detail: None }
                        EvidenceMetric { label: "Maximum", value: maximum, detail: maximum_detail }
                    }
                    if matrix.partial || !matrix.nodes_failed.is_empty() {
                        div {
                            class: "rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900",
                            "Partial result · {matrix.nodes_failed.len()} peer endpoint(s) did not return step samples"
                        }
                    }
                    StepTrendChart { points: health.trend }
                }
            }
        }
    }
}

#[component]
fn EvidenceTable(
    state: Option<Result<EvidencePayload<DataFrame>>>,
    loading: &'static str,
    empty: &'static str,
) -> Element {
    match state {
        None => rsx! { div { class: "p-4", LoadingPanel { label: loading.to_string() } } },
        Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel {
            label: format!("{empty} available"),
            detail: error.display_message(),
        }}},
        Some(Ok(payload)) if dataframe_rows(&payload.value) == 0 => rsx! {
            div { class: "p-4", UnavailablePanel {
                label: empty.to_string(),
                detail: "The query returned zero rows.".to_string(),
            }}
        },
        Some(Ok(payload)) => rsx! {
            if payload.receipt.partial || payload.receipt.failed_peers > 0 {
                div { class: "border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-900",
                    "Partial result · {payload.receipt.failed_peers} failed peer endpoint(s)"
                }
            }
            DataFrameView { df: payload.value }
        },
    }
}

fn dataframe_rows(dataframe: &DataFrame) -> usize {
    dataframe
        .cols
        .iter()
        .map(|column| column.len())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn training_queries_remain_bounded() {
        assert!(MODULE_HOTSPOTS_SQL.contains("LIMIT 12"));
        assert!(COMM_SUMMARY_SQL.contains("LIMIT 12"));
        assert!(GROUP_COMMUNICATION_SQL.contains("LIMIT 768"));
        assert!(GROUP_COMMUNICATION_SQL.contains("participate_ranks"));
        assert!(RANK_MEMORY_SQL.contains("ORDER BY local_step DESC, seq DESC LIMIT 1"));
        assert!(RANK_MEMORY_SQL.contains("max_allocated"));
        assert!(RANK_MEMORY_SQL.contains("stage LIKE 'post %'"));
        assert!(DEVICE_MEMORY_SQL.contains("300000000"));
        assert!(DEVICE_MEMORY_SQL.contains("ROW_NUMBER() OVER"));
        assert!(DEVICE_MEMORY_SQL.contains("LIMIT 16"));
        assert!(DEVICE_MEMORY_SQL.contains("process.envs WHERE name = 'RANK'"));
        assert!(DEVICE_MEMORY_SQL.contains("process.envs WHERE name = 'LOCAL_RANK'"));
    }

    #[test]
    fn partial_cluster_payload_remains_visible_after_query_collection() {
        let request =
            EvidenceRequest::new(1, EvidenceScope::ClusterFanout, None, Default::default());
        let state = Some(Ok(cluster_dataframe_payload(
            "python.comm_collective groups",
            &request,
            DataFrame::default(),
            8,
            2,
            true,
        )));

        assert_eq!(
            partial_source("communication groups", &state).as_deref(),
            Some("communication groups: 2 failed peer(s)")
        );
    }
}
