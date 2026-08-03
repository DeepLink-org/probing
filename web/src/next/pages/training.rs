use dioxus::prelude::*;
use probing_proto::prelude::DataFrame;

use crate::api::ApiClient;
use crate::components::dataframe_view::DataFrameView;
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::state::training::{
    placement_availability, TRAINING_CLUSTER_SCOPE, TRAINING_PLACEMENT_AVAILABILITY,
    TRAINING_REFRESH,
};
use crate::utils::error::Result;

use super::super::components::{
    EvidenceMetric, EvidenceSection, EvidenceSurface, LoadingPanel, UnavailablePanel, WorkspacePage,
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

#[component]
pub fn TrainingPage() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let refresh_key = use_memo(move || poll().wrapping_add(*TRAINING_REFRESH.read()));

    let steps = use_resource(move || {
        let _ = refresh_key();
        let cluster = *TRAINING_CLUSTER_SCOPE.read();
        async move {
            ApiClient::new()
                .fetch_step_matrix(STEP_LIMIT, cluster)
                .await
        }
    });
    let nodes = use_resource(move || {
        let _ = refresh_key();
        async move { ApiClient::new().get_nodes().await }
    });
    let modules = use_resource(move || {
        let _ = refresh_key();
        let cluster = *TRAINING_CLUSTER_SCOPE.read();
        async move { training_query(MODULE_HOTSPOTS_SQL, cluster).await }
    });
    let collectives = use_resource(move || {
        let _ = refresh_key();
        let cluster = *TRAINING_CLUSTER_SCOPE.read();
        async move { training_query(COMM_SUMMARY_SQL, cluster).await }
    });

    let step_state = steps.read().clone();
    let node_state = nodes.read().clone();
    let module_state = modules.read().clone();
    let collective_state = collectives.read().clone();
    let latest_step = step_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(|matrix| StepHealth::from_matrix(matrix).latest_step);
    let placement_state = node_state.clone();
    use_effect(move || {
        *TRAINING_PLACEMENT_AVAILABILITY.write() = placement_availability(placement_state.as_ref());
    });
    let scope = if *TRAINING_CLUSTER_SCOPE.read() {
        "cluster fan-out"
    } else {
        "local node"
    };

    rsx! {
        WorkspacePage {
            title: "Training".to_string(),
            subtitle: "Step timing, physical rank placement, module hooks, and collective measurements.".to_string(),
            actions: rsx! { span { class: "text-xs text-gray-500", "{scope} · {POLL_MS / 1000}s" } },

            EvidenceSurface {
                EvidenceSection {
                    title: "Step time".to_string(),
                    subtitle: Some("Latest rank samples and recent median/P95 trend.".to_string()),
                    StepEvidence { state: step_state }
                }

                if let Some(Ok(nodes)) = node_state.as_ref() {
                    if !nodes.is_empty() {
                        EvidenceSection {
                            title: "Placement".to_string(),
                            subtitle: Some("One square per reported accelerator process, grouped by physical host and local rank.".to_string()),
                            divided: true,
                            TrainingPlacement { nodes: nodes.clone(), local_step: latest_step }
                        }
                    }
                }

                div { class: "grid items-start border-t border-gray-200 xl:grid-cols-2",
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
                    div { class: "border-t border-gray-200 xl:border-l xl:border-t-0",
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

async fn training_query(sql: &str, cluster: bool) -> Result<DataFrame> {
    let client = ApiClient::new();
    if cluster {
        client
            .cluster_query(sql, true)
            .await
            .map(|response| response.dataframe)
    } else {
        client.execute_query(sql).await
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
                            "Partial result · {matrix.nodes_failed.len()} node(s) did not return step samples"
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
    state: Option<Result<DataFrame>>,
    loading: &'static str,
    empty: &'static str,
) -> Element {
    match state {
        None => rsx! { div { class: "p-4", LoadingPanel { label: loading.to_string() } } },
        Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel {
            label: format!("{empty} available"),
            detail: error.display_message(),
        }}},
        Some(Ok(dataframe)) if dataframe_rows(&dataframe) == 0 => rsx! {
            div { class: "p-4", UnavailablePanel {
                label: empty.to_string(),
                detail: "The query returned zero rows.".to_string(),
            }}
        },
        Some(Ok(dataframe)) => rsx! { DataFrameView { df: dataframe } },
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
    }
}
