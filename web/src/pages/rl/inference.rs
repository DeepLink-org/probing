use std::collections::HashMap;

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::EngineInfo;
use crate::api::ApiClient;
use crate::components::card::Card;
use crate::components::common::{EmptyState, ErrorState, LoadingState};
use crate::components::rl::metrics_line_chart::{ChartSeries, MetricsLineChart};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api;

const REFRESH_MS: u32 = 5000;
const METRICS_HISTORY_LIMIT: i64 = 500;

const METRIC_DEFS: [(&str, &str); 7] = [
    ("normalized.inflight_requests", "In-flight requests"),
    ("normalized.queue_depth", "Queue depth"),
    ("normalized.throughput_tps", "Throughput (tok/s)"),
    ("normalized.tpot_ms", "TPOT (ms)"),
    ("normalized.ttft_ms", "TTFT (ms)"),
    ("normalized.kv_cache_usage_ratio", "KV cache usage"),
    ("normalized.cache_hit_ratio", "Cache hit ratio"),
];

const SERIES_COLORS: [&str; 6] = [
    "#2563eb", "#dc2626", "#16a34a", "#9333ea", "#ea580c", "#0891b2",
];

#[component]
pub fn Inference() -> Element {
    let mut refresh_tick = use_signal(|| 0u64);
    let engines = use_api({
        let refresh_tick = refresh_tick.clone();
        move || {
            let _ = *refresh_tick.read();
            let client = ApiClient::new();
            async move { client.fetch_inference_engines().await.map(|resp| resp.engines) }
        }
    });
    let metrics = use_api({
        let refresh_tick = refresh_tick.clone();
        move || {
            let _ = *refresh_tick.read();
            let client = ApiClient::new();
            async move {
                client
                    .fetch_inference_engine_metrics(METRICS_HISTORY_LIMIT)
                    .await
            }
        }
    });

    use_effect(move || {
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(REFRESH_MS).await;
                refresh_tick.set(refresh_tick() + 1);
            }
        });
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Inference Engine".to_string(),
                subtitle: Some(
                    "Inference-engine Prometheus metrics (scrape path configurable, e.g. /metrics)".to_string(),
                ),
                icon: Some(&icondata::AiDashboardOutlined),
            }

            div { class: "flex items-center gap-3 mb-4",
                button {
                    class: "px-3 py-1.5 rounded-md text-sm bg-blue-600 text-white hover:bg-blue-700",
                    onclick: move |_| {
                        spawn(async move {
                            let _ = ApiClient::new().scrape_inference_engines().await;
                            refresh_tick.set(refresh_tick() + 1);
                        });
                    },
                    "Scrape now"
                }
                span { class: "text-xs text-slate-500", "Auto refresh every {REFRESH_MS / 1000}s" }
            }

            Card {
                title: "Registered engines",
                content_class: Some(""),
                {engines_panel(&engines)}
            }

            Card {
                title: "Latest normalized metrics",
                content_class: Some(""),
                {metrics_cards(&engines)}
            }

            Card {
                title: "Metric trends (time series)",
                content_class: Some(""),
                {metrics_charts(&metrics)}
            }
        }
    }
}

fn engines_panel(state: &crate::hooks::ApiState<Vec<EngineInfo>>) -> Element {
    if state.is_loading() && state.data.read().is_none() {
        return rsx! { LoadingState { message: Some("Loading engines...".to_string()) } };
    }
    match state.data.read().as_ref() {
        Some(Ok(engines)) if engines.is_empty() => rsx! {
            EmptyState {
                message: "No inference engine registered. Register via /apis/pythonext/engines/register?router_addr=http://host:port&metrics_path=/metrics".to_string(),
            }
        },
        Some(Ok(engines)) => rsx! {
            div { class: "overflow-x-auto",
                table { class: "min-w-full text-sm",
                    thead {
                        tr { class: "text-left text-slate-500 border-b border-slate-200",
                            th { class: "py-2 pr-4", "Engine" }
                            th { class: "py-2 pr-4", "Type" }
                            th { class: "py-2 pr-4", "Framework" }
                            th { class: "py-2 pr-4", "Metrics URL" }
                            th { class: "py-2 pr-4", "Status" }
                        }
                    }
                    tbody {
                        for engine in engines.iter() {
                            tr { class: "border-b border-slate-100",
                                td { class: "py-2 pr-4 font-medium", "{engine.engine_id}" }
                                td { class: "py-2 pr-4", "{engine.engine_type}" }
                                td { class: "py-2 pr-4", "{engine.framework}" }
                                td { class: "py-2 pr-4 font-mono text-xs break-all", "{engine.metrics_url}" }
                                td { class: "py-2 pr-4",
                                    span {
                                        class: if engine.status == "healthy" {
                                            "text-emerald-600"
                                        } else {
                                            "text-amber-600"
                                        },
                                        "{engine.status}"
                                    }
                                    if let Some(error) = &engine.last_scrape_error {
                                        div { class: "text-xs text-red-500 mt-1", "{error}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        Some(Err(error)) => rsx! { ErrorState { error: error.to_string(), title: None } },
        None => rsx! { LoadingState { message: Some("Loading engines...".to_string()) } },
    }
}

fn metrics_cards(state: &crate::hooks::ApiState<Vec<EngineInfo>>) -> Element {
    if state.is_loading() && state.data.read().is_none() {
        return rsx! { LoadingState { message: Some("Loading metrics...".to_string()) } };
    }
    let data = state.data.read();
    let Some(Ok(engines)) = data.as_ref() else {
        return rsx! { EmptyState { message: "No metrics yet.".to_string() } };
    };
    if engines.is_empty() {
        return rsx! { EmptyState { message: "Register an engine to see metrics.".to_string() } };
    }

    let labels = [
        ("inflight_requests", "In-flight requests"),
        ("queue_depth", "Queue depth"),
        ("throughput_tps", "Throughput (tok/s)"),
        ("tpot_ms", "TPOT (ms)"),
        ("ttft_ms", "TTFT (ms)"),
        ("kv_cache_usage_ratio", "KV cache usage"),
        ("cache_hit_ratio", "Cache hit ratio"),
    ];

    rsx! {
        for engine in engines.iter() {
            div { class: "mb-4",
                div { class: "text-sm font-semibold text-slate-700 mb-2", "{engine.engine_id}" }
                div { class: "grid grid-cols-2 md:grid-cols-4 gap-3",
                    for (key, label) in labels.iter() {
                        div { class: "rounded-lg border border-slate-200 bg-slate-50 px-3 py-2",
                            div { class: "text-[11px] uppercase tracking-wide text-slate-500", "{label}" }
                            div { class: "text-lg font-semibold text-slate-900",
                                {
                                    engine.last_normalized
                                        .get(*key)
                                        .map(format_metric_value)
                                        .unwrap_or_else(|| "—".to_string())
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn metrics_charts(state: &crate::hooks::ApiState<DataFrame>) -> Element {
    if state.is_loading() && state.data.read().is_none() {
        return rsx! { LoadingState { message: Some("Loading metric trends...".to_string()) } };
    }
    match state.data.read().as_ref() {
        Some(Ok(df)) if df.is_empty() => rsx! {
            EmptyState {
                message: "No metric samples stored yet. Click Scrape now or wait for the background scraper.".to_string(),
            }
        },
        Some(Ok(df)) => {
            let grouped = group_metric_rows(df);
            if grouped.is_empty() {
                return rsx! {
                    EmptyState {
                        message: "No normalized metric samples found yet.".to_string(),
                    }
                };
            }
            let engine_colors = engine_color_map(&grouped);

            rsx! {
                div { class: "grid grid-cols-1 xl:grid-cols-2 gap-4",
                    for (metric_key, title) in METRIC_DEFS.iter() {
                        {
                            let chart_series = build_chart_series(&grouped, &engine_colors, metric_key);
                            rsx! {
                                MetricsLineChart {
                                    title: title.to_string(),
                                    series: chart_series,
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(Err(error)) => rsx! { ErrorState { error: error.to_string(), title: None } },
        None => rsx! { LoadingState { message: Some("Loading metric trends...".to_string()) } },
    }
}

fn group_metric_rows(df: &DataFrame) -> HashMap<(String, String), Vec<(f64, f64)>> {
    let ts_idx = col_idx(df, "timestamp_ns");
    let engine_idx = col_idx(df, "engine_id");
    let name_idx = col_idx(df, "metric_name");
    let value_idx = col_idx(df, "metric_value");

    let Some((ts_idx, engine_idx, name_idx, value_idx)) = (
        match (ts_idx, engine_idx, name_idx, value_idx) {
            (Some(a), Some(b), Some(c), Some(d)) => Some((a, b, c, d)),
            _ => None,
        }
    ) else {
        return HashMap::new();
    };

    let mut grouped: HashMap<(String, String), Vec<(f64, f64)>> = HashMap::new();
    for row in df.iter() {
        let Some(ts_ns) = ele_i64(row.get(ts_idx)) else {
            continue;
        };
        let Some(engine_id) = ele_str(row.get(engine_idx)) else {
            continue;
        };
        let Some(metric_name) = ele_str(row.get(name_idx)) else {
            continue;
        };
        let Some(metric_value) = ele_f64(row.get(value_idx)) else {
            continue;
        };
        if !metric_name.starts_with("normalized.") {
            continue;
        }

        let ts_ms = ts_ns as f64 / 1_000_000.0;
        grouped
            .entry((metric_name, engine_id))
            .or_default()
            .push((ts_ms, metric_value));
    }

    for points in grouped.values_mut() {
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        dedupe_time_points(points);
    }

    grouped
}

fn build_chart_series(
    grouped: &HashMap<(String, String), Vec<(f64, f64)>>,
    engine_colors: &HashMap<String, &'static str>,
    metric_key: &str,
) -> Vec<ChartSeries> {
    let mut engine_ids: Vec<String> = grouped
        .keys()
        .filter_map(|(name, engine_id)| (name == metric_key).then_some(engine_id.clone()))
        .collect();
    engine_ids.sort();

    engine_ids
        .into_iter()
        .filter_map(|engine_id| {
            grouped
                .get(&(metric_key.to_string(), engine_id.clone()))
                .map(|points| ChartSeries {
                    label: engine_id.clone(),
                    points: points.clone(),
                    color: engine_colors
                        .get(&engine_id)
                        .copied()
                        .unwrap_or("#64748b"),
                })
        })
        .collect()
}

fn engine_color_map(grouped: &HashMap<(String, String), Vec<(f64, f64)>>) -> HashMap<String, &'static str> {
    let mut engines: Vec<String> = grouped
        .keys()
        .map(|(_, engine_id)| engine_id.clone())
        .collect();
    engines.sort();
    engines.dedup();

    engines
        .into_iter()
        .enumerate()
        .map(|(idx, engine_id)| {
            (
                engine_id,
                SERIES_COLORS[idx % SERIES_COLORS.len()],
            )
        })
        .collect()
}

fn dedupe_time_points(points: &mut Vec<(f64, f64)>) {
    if points.len() <= 1 {
        return;
    }
    let mut deduped: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for (ts, value) in points.drain(..) {
        if deduped.last().map(|(last_ts, _)| (*last_ts - ts).abs() < f64::EPSILON) == Some(true)
        {
            if let Some(last) = deduped.last_mut() {
                last.1 = value;
            }
        } else {
            deduped.push((ts, value));
        }
    }
    *points = deduped;
}

fn col_idx(df: &DataFrame, name: &str) -> Option<usize> {
    df.names.iter().position(|col| col == name)
}

fn ele_i64(ele: Option<&Ele>) -> Option<i64> {
    match ele? {
        Ele::I64(v) => Some(*v),
        Ele::I32(v) => Some(*v as i64),
        Ele::F64(v) => Some(*v as i64),
        Ele::F32(v) => Some(*v as i64),
        Ele::Text(v) | Ele::Url(v) => v.parse().ok(),
        Ele::DataTime(v) => i64::try_from(*v).ok(),
        _ => None,
    }
}

fn ele_f64(ele: Option<&Ele>) -> Option<f64> {
    match ele? {
        Ele::F64(v) => Some(*v),
        Ele::F32(v) => Some(*v as f64),
        Ele::I64(v) => Some(*v as f64),
        Ele::I32(v) => Some(*v as f64),
        Ele::Text(v) | Ele::Url(v) => v.parse().ok(),
        Ele::DataTime(v) => Some(*v as f64),
        _ => None,
    }
}

fn ele_str(ele: Option<&Ele>) -> Option<String> {
    match ele? {
        Ele::Text(v) => Some(v.clone()),
        Ele::Url(v) => Some(v.clone()),
        Ele::I64(v) => Some(v.to_string()),
        Ele::I32(v) => Some(v.to_string()),
        Ele::F32(v) => Some(v.to_string()),
        Ele::F64(v) => Some(v.to_string()),
        _ => None,
    }
}

fn format_metric_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_f64() {
                if value.fract() == 0.0 {
                    format!("{value:.0}")
                } else {
                    format!("{value:.3}")
                }
            } else {
                number.to_string()
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::{DataFrame, Ele, Seq};

    #[test]
    fn group_metric_rows_accepts_text_metric_values_and_numeric_engine_ids() {
        let df = DataFrame {
            names: vec![
                "timestamp_ns".into(),
                "engine_id".into(),
                "engine_type".into(),
                "metric_name".into(),
                "metric_value".into(),
                "labels".into(),
            ],
            cols: vec![
                Seq::SeqI64(vec![1_000_000_000, 2_000_000_000]),
                Seq::SeqI64(vec![0, 0]),
                Seq::SeqText(vec!["inference".into(), "inference".into()]),
                Seq::SeqText(vec![
                    "normalized.inflight_requests".into(),
                    "normalized.inflight_requests".into(),
                ]),
                Seq::SeqText(vec!["0".into(), "1".into()]),
                Seq::SeqText(vec!["normalized=1".into(), "normalized=1".into()]),
            ],
            size: 0,
        };

        let grouped = group_metric_rows(&df);
        assert_eq!(grouped.len(), 1);
        let points = grouped
            .get(&(
                "normalized.inflight_requests".to_string(),
                "0".to_string(),
            ))
            .expect("series");
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].1, 0.0);
        assert_eq!(points[1].1, 1.0);
    }

    #[test]
    fn ele_helpers_coerce_common_query_variants() {
        assert_eq!(ele_f64(Some(&Ele::Text("0.5".into()))), Some(0.5));
        assert_eq!(ele_str(Some(&Ele::I64(42))), Some("42".into()));
        assert_eq!(ele_i64(Some(&Ele::Text("99".into()))), Some(99));
    }
}
