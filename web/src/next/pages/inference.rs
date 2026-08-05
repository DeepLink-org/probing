use std::collections::HashMap;

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::{ApiClient, EngineInfo};
use crate::components::rl::metrics_line_chart::{ChartSeries, MetricsLineChart};
use crate::hooks::{use_page_visible, use_poll_tick_gated};
use crate::state::inference::INFERENCE_REFRESH;
use crate::utils::error::{AppError, Result};

use super::super::components::{
    EvidenceMetric, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};

const POLL_MS: u32 = 5_000;
const HISTORY_LIMIT: i64 = 500;
const METRICS: [(&str, &str); 7] = [
    ("normalized.inflight_requests", "In-flight requests"),
    ("normalized.queue_depth", "Queue depth"),
    ("normalized.throughput_tps", "Throughput (tok/s)"),
    ("normalized.tpot_ms", "TPOT (ms)"),
    ("normalized.ttft_ms", "TTFT (ms)"),
    ("normalized.kv_cache_usage_ratio", "KV cache usage"),
    ("normalized.cache_hit_ratio", "Cache hit ratio"),
];
const COLORS: [&str; 6] = [
    "#2563eb", "#dc2626", "#16a34a", "#9333ea", "#ea580c", "#0891b2",
];

#[derive(Clone, Debug, PartialEq)]
struct InferenceEvidence {
    engines: Vec<EngineInfo>,
    history: MetricHistoryEvidence,
}

#[derive(Clone, Debug, PartialEq)]
enum MetricHistoryEvidence {
    NotApplicable,
    Loaded(DataFrame),
    Unavailable(AppError),
}

#[component]
pub fn InferencePage() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let refresh_key = use_memo(move || u64::from(poll()).wrapping_add(*INFERENCE_REFRESH.read()));
    let evidence = use_resource(move || {
        let _ = refresh_key();
        async move { load_inference_evidence().await }
    });
    let evidence_state = evidence.read().clone();

    rsx! {
        WorkspacePage {
            title: "Inference".to_string(),
            subtitle: "Registered serving engines, their latest normalized measurements, and recent metric history.".to_string(),
            actions: rsx! { span { class: "text-xs text-gray-500", "Live · {POLL_MS / 1000}s" } },
            SectionCard {
                title: "Registered engines".to_string(),
                subtitle: Some("Endpoints and scrape state reported by the inference registry.".to_string()),
                body_class: "p-0".to_string(),
                EngineRegistry { state: evidence_state.clone() }
            }
            SectionCard {
                title: "Latest normalized metrics".to_string(),
                subtitle: Some("Values from the latest successful scrape for each engine.".to_string()),
                LatestMetrics { state: evidence_state.clone() }
            }
            SectionCard {
                title: "Metric history".to_string(),
                subtitle: Some("Stored normalized samples; one line per registered engine.".to_string()),
                MetricHistory { state: evidence_state }
            }
        }
    }
}

async fn load_inference_evidence() -> Result<InferenceEvidence> {
    let client = ApiClient::new();
    let engines = client.fetch_inference_engines().await?.engines;
    let history = if engines.is_empty() {
        MetricHistoryEvidence::NotApplicable
    } else {
        match client.fetch_inference_engine_metrics(HISTORY_LIMIT).await {
            Ok(dataframe) => MetricHistoryEvidence::Loaded(dataframe),
            Err(error) => MetricHistoryEvidence::Unavailable(error),
        }
    };
    Ok(InferenceEvidence { engines, history })
}

#[component]
fn EngineRegistry(state: Option<Result<InferenceEvidence>>) -> Element {
    match state {
        None => {
            rsx! { div { class: "p-4", LoadingPanel { label: "Loading engine registry".to_string() } } }
        }
        Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel {
            label: "Engine registry unavailable".to_string(),
            detail: error.display_message(),
        }}},
        Some(Ok(evidence)) if evidence.engines.is_empty() => {
            rsx! { div { class: "p-4", UnavailablePanel {
                label: "No inference engines registered".to_string(),
                detail: "Register a supported serving engine, then use Scrape now to collect metrics.".to_string(),
            }}}
        }
        Some(Ok(evidence)) => rsx! {
            div { class: "overflow-x-auto",
                table { class: "w-full border-collapse text-xs",
                    thead {
                        tr { class: "border-b border-gray-200 bg-gray-50 text-left text-xs uppercase tracking-wide text-gray-500",
                            th { class: "px-3 py-2 font-medium", "Engine" }
                            th { class: "px-3 py-2 font-medium", "Type / framework" }
                            th { class: "px-3 py-2 font-medium", "Router" }
                            th { class: "px-3 py-2 font-medium", "Metrics URL" }
                            th { class: "px-3 py-2 font-medium", "Scrape state" }
                        }
                    }
                    tbody { class: "divide-y divide-gray-100",
                        for engine in evidence.engines {
                            tr { class: "hover:bg-gray-50/70",
                                td { class: "px-3 py-2 font-medium text-gray-900", "{engine.engine_id}" }
                                td { class: "px-3 py-2 text-gray-600", "{engine.engine_type} · {engine.framework}" }
                                td { class: "px-3 py-2 font-mono text-xs text-gray-600", "{engine.router_addr}" }
                                td { class: "px-3 py-2 font-mono text-xs text-gray-600", "{engine.metrics_url}" }
                                td { class: "px-3 py-2",
                                    div { class: "text-gray-700", "{engine.status}" }
                                    if let Some(error) = engine.last_scrape_error {
                                        div { class: "mt-0.5 max-w-72 break-words text-xs leading-4 text-red-700", "{error}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    }
}

#[component]
fn LatestMetrics(state: Option<Result<InferenceEvidence>>) -> Element {
    match state {
        None => rsx! { LoadingPanel { label: "Loading latest metrics".to_string() } },
        Some(Err(error)) => rsx! { UnavailablePanel {
            label: "Latest metrics unavailable".to_string(),
            detail: error.display_message(),
        }},
        Some(Ok(evidence)) if evidence.engines.is_empty() => rsx! { UnavailablePanel {
            label: "No engine metrics".to_string(),
            detail: "Metric queries start only after an engine is registered.".to_string(),
        }},
        Some(Ok(evidence)) => rsx! {
            div { class: "space-y-4",
                for engine in evidence.engines {
                    div {
                        div { class: "mb-2 text-xs font-semibold text-gray-700", "{engine.engine_id}" }
                        div { class: "grid grid-cols-2 gap-2 lg:grid-cols-4",
                            for (key, label) in METRICS.iter() {
                                div { class: "rounded-lg border border-gray-200 bg-gray-50 px-3 py-2",
                                    EvidenceMetric {
                                        label: label.to_string(),
                                        value: engine.last_normalized.get(*key).map(format_metric_value).unwrap_or_else(|| "—".to_string()),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    }
}

#[component]
fn MetricHistory(state: Option<Result<InferenceEvidence>>) -> Element {
    match state {
        None => rsx! { LoadingPanel { label: "Loading metric history".to_string() } },
        Some(Err(_)) => rsx! { UnavailablePanel {
            label: "Metric history not requested".to_string(),
            detail: "The engine registry is unavailable for this refresh.".to_string(),
        }},
        Some(Ok(evidence)) => match evidence.history {
            MetricHistoryEvidence::NotApplicable => rsx! { UnavailablePanel {
                label: "No metric history yet".to_string(),
                detail: "History queries remain idle until at least one engine is registered.".to_string(),
            }},
            MetricHistoryEvidence::Unavailable(error) => rsx! { UnavailablePanel {
                label: "Metric history unavailable".to_string(),
                detail: metric_history_error_detail(&error),
            }},
            MetricHistoryEvidence::Loaded(dataframe) => {
                let grouped = group_metric_rows(&dataframe);
                if grouped.is_empty() {
                    return rsx! { UnavailablePanel {
                        label: "No normalized metric history".to_string(),
                        detail: "No stored normalized samples were returned.".to_string(),
                    }};
                }
                let colors = engine_colors(&grouped);
                rsx! {
                    div { class: "grid grid-cols-1 gap-4 xl:grid-cols-2",
                        for (metric, title) in METRICS.iter() {
                            MetricsLineChart {
                                title: title.to_string(),
                                series: chart_series(&grouped, &colors, metric),
                            }
                        }
                    }
                }
            }
        },
    }
}

fn metric_history_error_detail(error: &AppError) -> String {
    let raw = error.to_string();
    if raw.contains("Schema error") || raw.contains("No field named") {
        "Metric history storage is not available in this runtime; current engine status remains usable."
            .to_string()
    } else {
        error.display_message()
    }
}

fn group_metric_rows(dataframe: &DataFrame) -> HashMap<(String, String), Vec<(f64, f64)>> {
    let index = |name: &str| dataframe.names.iter().position(|column| column == name);
    let (Some(timestamp), Some(engine), Some(metric), Some(value)) = (
        index("timestamp_ns"),
        index("engine_id"),
        index("metric_name"),
        index("metric_value"),
    ) else {
        return HashMap::new();
    };
    let mut grouped = HashMap::<(String, String), Vec<(f64, f64)>>::new();
    for row in dataframe.iter() {
        let (Some(timestamp), Some(engine), Some(metric), Some(value)) = (
            ele_f64(row.get(timestamp)),
            ele_string(row.get(engine)),
            ele_string(row.get(metric)),
            ele_f64(row.get(value)),
        ) else {
            continue;
        };
        if metric.starts_with("normalized.") {
            grouped
                .entry((metric, engine))
                .or_default()
                .push((timestamp / 1_000_000.0, value));
        }
    }
    for points in grouped.values_mut() {
        points.sort_by(|left, right| left.0.total_cmp(&right.0));
        points.dedup_by(|left, right| {
            if (left.0 - right.0).abs() < f64::EPSILON {
                left.1 = right.1;
                true
            } else {
                false
            }
        });
    }
    grouped
}

fn engine_colors(
    grouped: &HashMap<(String, String), Vec<(f64, f64)>>,
) -> HashMap<String, &'static str> {
    let mut engines = grouped
        .keys()
        .map(|(_, engine)| engine.clone())
        .collect::<Vec<_>>();
    engines.sort();
    engines.dedup();
    engines
        .into_iter()
        .enumerate()
        .map(|(index, engine)| (engine, COLORS[index % COLORS.len()]))
        .collect()
}

fn chart_series(
    grouped: &HashMap<(String, String), Vec<(f64, f64)>>,
    colors: &HashMap<String, &'static str>,
    metric: &str,
) -> Vec<ChartSeries> {
    let mut engines = grouped
        .keys()
        .filter_map(|(name, engine)| (name == metric).then_some(engine.clone()))
        .collect::<Vec<_>>();
    engines.sort();
    engines
        .into_iter()
        .filter_map(|engine| {
            grouped
                .get(&(metric.to_string(), engine.clone()))
                .map(|points| ChartSeries {
                    label: engine.clone(),
                    points: points.clone(),
                    color: colors.get(&engine).copied().unwrap_or("#64748b"),
                })
        })
        .collect()
}

fn ele_f64(value: Option<&Ele>) -> Option<f64> {
    match value? {
        Ele::F64(value) => Some(*value),
        Ele::F32(value) => Some(*value as f64),
        Ele::I64(value) => Some(*value as f64),
        Ele::I32(value) => Some(*value as f64),
        Ele::Text(value) | Ele::Url(value) => value.parse().ok(),
        Ele::DataTime(value) => Some(*value as f64),
        _ => None,
    }
}

fn ele_string(value: Option<&Ele>) -> Option<String> {
    match value? {
        Ele::Text(value) | Ele::Url(value) => Some(value.clone()),
        Ele::I64(value) => Some(value.to_string()),
        Ele::I32(value) => Some(value.to_string()),
        Ele::F64(value) => Some(value.to_string()),
        Ele::F32(value) => Some(value.to_string()),
        _ => None,
    }
}

fn format_metric_value(value: &serde_json::Value) -> String {
    value
        .as_f64()
        .map(|value| {
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                format!("{value:.3}")
            }
        })
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::Seq;

    #[test]
    fn metric_rows_accept_text_values_and_numeric_engine_ids() {
        let dataframe = DataFrame {
            names: vec![
                "timestamp_ns".into(),
                "engine_id".into(),
                "metric_name".into(),
                "metric_value".into(),
            ],
            cols: vec![
                Seq::SeqI64(vec![1_000_000_000]),
                Seq::SeqI64(vec![0]),
                Seq::SeqText(vec!["normalized.queue_depth".into()]),
                Seq::SeqText(vec!["3".into()]),
            ],
            size: 0,
        };
        let grouped = group_metric_rows(&dataframe);
        assert_eq!(
            grouped.get(&("normalized.queue_depth".into(), "0".into())),
            Some(&vec![(1_000.0, 3.0)])
        );
    }

    #[test]
    fn schema_errors_are_presented_as_capability_absence() {
        let error = AppError::Api(
            "Schema error: No field named metric_name. Valid fields are _error".to_string(),
        );
        assert_eq!(
            metric_history_error_detail(&error),
            "Metric history storage is not available in this runtime; current engine status remains usable."
        );
    }
}
