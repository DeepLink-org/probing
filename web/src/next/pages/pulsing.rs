use std::collections::{BTreeMap, BTreeSet};

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::ApiClient;
use crate::components::dataframe_view::DataFrameView;

use super::super::components::{
    EvidenceMetric, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};

#[component]
pub fn PulsingPage() -> Element {
    let actors = use_resource(|| async move { ApiClient::new().fetch_pulsing_actors().await });
    let spans = use_resource(|| async move { ApiClient::new().fetch_pulsing_spans().await });
    let span_count =
        use_resource(|| async move { ApiClient::new().fetch_pulsing_span_count().await });
    let metrics = use_resource(|| async move { ApiClient::new().fetch_pulsing_metrics().await });
    let members = use_resource(|| async move { ApiClient::new().fetch_pulsing_members().await });
    let actor_state = actors.read().clone();
    let span_state = spans.read().clone();
    let count_state = span_count.read().clone();
    let metric_state = metrics.read().clone();
    let member_state = members.read().clone();
    let span_evidence = span_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(summarize_spans);

    rsx! {
        WorkspacePage {
            title: "Pulsing".to_string(),
            subtitle: "Actor inventory, operation latency, explicit span errors, and runtime membership from the current Pulsing process.".to_string(),
            SectionCard {
                title: "Reported coverage".to_string(),
                subtitle: Some("Counts state what was returned; they do not infer actor liveness or health.".to_string()),
                div { class: "grid grid-cols-4 divide-x divide-gray-200",
                    EvidenceMetric { label: "Registered actors", value: row_count(&actor_state) }
                    EvidenceMetric { label: "Total spans", value: scalar_count(&count_state) }
                    EvidenceMetric { label: "Actors in window", value: span_evidence.as_ref().map(|evidence| evidence.actor_count.to_string()).unwrap_or_else(|| "—".to_string()) }
                    EvidenceMetric { label: "Members", value: row_count(&member_state) }
                }
            }
            SectionCard {
                title: "Operation latency".to_string(),
                subtitle: Some("Latest 500 returned spans grouped by actor and operation; P95 and maximum use reported duration_us.".to_string()),
                body_class: "p-0".to_string(),
                match span_state.clone() {
                    None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading operation spans".to_string() } } },
                    Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Operation latency unavailable".to_string(), detail: error.display_message() } } },
                    Some(Ok(_)) if span_evidence.as_ref().is_none_or(|evidence| evidence.operations.is_empty()) => rsx! { div { class: "p-4", UnavailablePanel { label: "No spans recorded".to_string(), detail: "The span query completed successfully with zero usable duration rows.".to_string() } } },
                    Some(Ok(_)) => rsx! { OperationTable { rows: span_evidence.clone().unwrap_or_default().operations } },
                }
            }
            SectionCard {
                title: "Recent spans".to_string(),
                subtitle: Some("Most recent returned records; status is shown verbatim and only explicit error values are counted above.".to_string()),
                body_class: "p-0".to_string(),
                match span_state {
                    None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading recent spans".to_string() } } },
                    Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Recent spans unavailable".to_string(), detail: error.display_message() } } },
                    Some(Ok(_)) if span_evidence.as_ref().is_none_or(|evidence| evidence.recent.is_empty()) => rsx! { div { class: "p-4", UnavailablePanel { label: "No recent spans".to_string(), detail: "No usable span rows were returned.".to_string() } } },
                    Some(Ok(_)) => rsx! { RecentSpanTable { rows: span_evidence.unwrap_or_default().recent } },
                }
            }
            div { class: "grid items-start gap-4 xl:grid-cols-2",
                EvidenceTable { title: "Actor registry".to_string(), subtitle: "Current actor registry rows reported by Pulsing.".to_string(), state: actor_state, empty: "No actors registered".to_string() }
                EvidenceTable { title: "Runtime members".to_string(), subtitle: "Current membership rows; an empty result may be valid in standalone mode.".to_string(), state: member_state, empty: "No runtime members returned".to_string() }
            }
            EvidenceTable { title: "Metric samples".to_string(), subtitle: "Latest 100 metric rows, shown without changing metric names or units.".to_string(), state: metric_state, empty: "No metrics recorded".to_string() }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct PulsingSpanEvidence {
    actor_count: usize,
    operations: Vec<OperationEvidence>,
    recent: Vec<RecentSpan>,
}

#[derive(Clone, Debug, PartialEq)]
struct OperationEvidence {
    actor: String,
    operation: String,
    samples: usize,
    p95_us: f64,
    max_us: f64,
    errors: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct RecentSpan {
    actor: String,
    operation: String,
    duration_us: f64,
    status: String,
}

fn summarize_spans(dataframe: &DataFrame) -> PulsingSpanEvidence {
    let actor = column_index(dataframe, "attr_actor_name");
    let operation = column_index(dataframe, "name");
    let duration = column_index(dataframe, "duration_us");
    let status = column_index(dataframe, "status_code");
    let mut grouped = BTreeMap::<(String, String), Vec<(f64, bool)>>::new();
    let mut actors = BTreeSet::new();
    let mut recent = Vec::new();

    for row in 0..dataframe.row_count() {
        let Some(duration_us) = duration
            .and_then(|column| numeric_value(dataframe.cols[column].get(row)))
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            continue;
        };
        let actor = actor
            .map(|column| text_value(dataframe.cols[column].get(row)))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unattributed)".to_string());
        let operation = operation
            .map(|column| text_value(dataframe.cols[column].get(row)))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unnamed)".to_string());
        let status = status
            .map(|column| text_value(dataframe.cols[column].get(row)))
            .unwrap_or_default();
        let error = explicit_error_status(&status);
        if actor != "(unattributed)" {
            actors.insert(actor.clone());
        }
        grouped
            .entry((actor.clone(), operation.clone()))
            .or_default()
            .push((duration_us, error));
        if recent.len() < 12 {
            recent.push(RecentSpan {
                actor,
                operation,
                duration_us,
                status,
            });
        }
    }

    let mut operations = grouped
        .into_iter()
        .map(|((actor, operation), samples)| {
            let mut durations = samples
                .iter()
                .map(|(duration, _)| *duration)
                .collect::<Vec<_>>();
            durations.sort_by(f64::total_cmp);
            let p95_index = ((durations.len() as f64 * 0.95).ceil() as usize)
                .saturating_sub(1)
                .min(durations.len().saturating_sub(1));
            OperationEvidence {
                actor,
                operation,
                samples: samples.len(),
                p95_us: durations[p95_index],
                max_us: durations.last().copied().unwrap_or_default(),
                errors: samples.iter().filter(|(_, error)| *error).count(),
            }
        })
        .collect::<Vec<_>>();
    operations.sort_by(|a, b| b.p95_us.total_cmp(&a.p95_us));

    PulsingSpanEvidence {
        actor_count: actors.len(),
        operations,
        recent,
    }
}

#[component]
fn OperationTable(rows: Vec<OperationEvidence>) -> Element {
    rsx! { div { class: "overflow-x-auto", table { class: "w-full text-left text-xs",
        thead { class: "bg-gray-50 uppercase tracking-wide text-gray-500", tr {
            th { class: "px-4 py-2", "Actor" } th { class: "px-4 py-2", "Operation" }
            th { class: "px-4 py-2 text-right", "Spans" } th { class: "px-4 py-2 text-right", "P95" }
            th { class: "px-4 py-2 text-right", "Maximum" } th { class: "px-4 py-2 text-right", "Explicit errors" }
        } }
        tbody { class: "divide-y divide-gray-100", for row in rows.iter().take(24) { tr {
            td { class: "max-w-64 truncate px-4 py-2 font-medium text-gray-800", title: "{row.actor}", "{row.actor}" }
            td { class: "max-w-80 truncate px-4 py-2 font-mono text-gray-700", title: "{row.operation}", "{row.operation}" }
            td { class: "px-4 py-2 text-right font-mono", "{row.samples}" }
            td { class: "px-4 py-2 text-right font-mono", "{format_duration_us(row.p95_us)}" }
            td { class: "px-4 py-2 text-right font-mono", "{format_duration_us(row.max_us)}" }
            td { class: if row.errors > 0 { "px-4 py-2 text-right font-mono font-medium text-red-700" } else { "px-4 py-2 text-right font-mono text-gray-500" }, "{row.errors}" }
        } } }
    } } }
}

#[component]
fn RecentSpanTable(rows: Vec<RecentSpan>) -> Element {
    rsx! { div { class: "overflow-x-auto", table { class: "w-full text-left text-xs",
        thead { class: "bg-gray-50 uppercase tracking-wide text-gray-500", tr {
            th { class: "px-4 py-2", "Actor" } th { class: "px-4 py-2", "Operation" }
            th { class: "px-4 py-2 text-right", "Duration" } th { class: "px-4 py-2", "Reported status" }
        } }
        tbody { class: "divide-y divide-gray-100", for row in rows { tr {
            td { class: "max-w-64 truncate px-4 py-2", title: "{row.actor}", "{row.actor}" }
            td { class: "max-w-96 truncate px-4 py-2 font-mono", title: "{row.operation}", "{row.operation}" }
            td { class: "px-4 py-2 text-right font-mono", "{format_duration_us(row.duration_us)}" }
            td { class: if explicit_error_status(&row.status) { "px-4 py-2 font-mono font-medium text-red-700" } else { "px-4 py-2 font-mono text-gray-500" }, if row.status.is_empty() { "—" } else { "{row.status}" } }
        } } }
    } } }
}

#[component]
fn EvidenceTable(
    title: String,
    subtitle: String,
    state: Option<crate::utils::error::Result<DataFrame>>,
    empty: String,
) -> Element {
    rsx! {
        SectionCard { title, subtitle: Some(subtitle), body_class: "p-0".to_string(),
            match state {
                None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading rows".to_string() } } },
                Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Rows unavailable".to_string(), detail: error.display_message() } } },
                Some(Ok(dataframe)) if dataframe.row_count() == 0 => rsx! { div { class: "p-4", UnavailablePanel { label: empty, detail: "The query completed successfully with an empty result.".to_string() } } },
                Some(Ok(dataframe)) => rsx! { DataFrameView { df: dataframe } },
            }
        }
    }
}

fn column_index(dataframe: &DataFrame, name: &str) -> Option<usize> {
    dataframe.names.iter().position(|column| column == name)
}

fn text_value(value: Ele) -> String {
    match value {
        Ele::Text(value) => value,
        Ele::I64(value) => value.to_string(),
        Ele::I32(value) => value.to_string(),
        Ele::F64(value) => value.to_string(),
        Ele::F32(value) => value.to_string(),
        _ => String::new(),
    }
}

fn numeric_value(value: Ele) -> Option<f64> {
    match value {
        Ele::I64(value) => Some(value as f64),
        Ele::I32(value) => Some(value as f64),
        Ele::F64(value) => Some(value),
        Ele::F32(value) => Some(value as f64),
        _ => None,
    }
}

fn explicit_error_status(status: &str) -> bool {
    status.eq_ignore_ascii_case("error") || status.trim() == "2"
}

fn format_duration_us(us: f64) -> String {
    if us < 1_000.0 {
        format!("{us:.0}µs")
    } else if us < 1_000_000.0 {
        format!("{:.1}ms", us / 1_000.0)
    } else {
        format!("{:.2}s", us / 1_000_000.0)
    }
}

fn row_count(state: &Option<crate::utils::error::Result<DataFrame>>) -> String {
    match state {
        None => "…".to_string(),
        Some(Err(_)) => "—".to_string(),
        Some(Ok(dataframe)) => dataframe.row_count().to_string(),
    }
}

fn scalar_count(state: &Option<crate::utils::error::Result<DataFrame>>) -> String {
    match state {
        None => "…".to_string(),
        Some(Err(_)) => "—".to_string(),
        Some(Ok(dataframe)) => match dataframe.cols.first().map(|column| column.get(0)) {
            Some(Ele::I64(value)) => value.to_string(),
            Some(Ele::I32(value)) => value.to_string(),
            _ => "0".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::Seq;

    #[test]
    fn span_summary_uses_reported_duration_and_explicit_errors() {
        let dataframe = DataFrame::new(
            vec!["attr_actor_name", "name", "duration_us", "status_code"]
                .into_iter()
                .map(String::from)
                .collect(),
            vec![
                Seq::SeqText(vec!["worker".into(), "worker".into(), "driver".into()]),
                Seq::SeqText(vec!["step".into(), "step".into(), "schedule".into()]),
                Seq::SeqI64(vec![100, 300, 50]),
                Seq::SeqText(vec!["OK".into(), "ERROR".into(), "UNSET".into()]),
            ],
        );

        let summary = summarize_spans(&dataframe);
        assert_eq!(summary.actor_count, 2);
        let step = summary
            .operations
            .iter()
            .find(|row| row.operation == "step")
            .unwrap();
        assert_eq!(step.samples, 2);
        assert_eq!(step.p95_us, 300.0);
        assert_eq!(step.errors, 1);
    }

    #[test]
    fn empty_success_is_distinct_from_failure() {
        let state = Some(Ok(DataFrame::default()));
        assert_eq!(row_count(&state), "0");
    }
}
