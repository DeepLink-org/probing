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

    rsx! {
        WorkspacePage {
            title: "Pulsing actors".to_string(),
            subtitle: "Actor registry, emitted spans, metric samples, and membership returned by the Pulsing tables.".to_string(),
            SectionCard {
                title: "Reported coverage".to_string(),
                subtitle: Some("Counts describe available rows and do not infer actor health.".to_string()),
                div { class: "grid grid-cols-3 divide-x divide-gray-200",
                    EvidenceMetric { label: "Actors", value: row_count(&actor_state) }
                    EvidenceMetric { label: "Spans captured", value: scalar_count(&count_state) }
                    EvidenceMetric { label: "Members", value: row_count(&member_state) }
                }
            }
            div { class: "grid items-start gap-4 xl:grid-cols-2",
                EvidenceTable { title: "Actors".to_string(), subtitle: "Current actor registry rows.".to_string(), state: actor_state, empty: "No actors registered".to_string() }
                EvidenceTable { title: "Cluster members".to_string(), subtitle: "Reported membership rows; empty may indicate standalone mode.".to_string(), state: member_state, empty: "No cluster members returned".to_string() }
            }
            EvidenceTable { title: "Span records".to_string(), subtitle: "Latest 500 actor spans ordered by reported start time.".to_string(), state: span_state, empty: "No spans recorded".to_string() }
            EvidenceTable { title: "Metric samples".to_string(), subtitle: "Latest 100 reported metric rows.".to_string(), state: metric_state, empty: "No metrics recorded".to_string() }
        }
    }
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
    #[test]
    fn empty_success_is_distinct_from_failure() {
        let state = Some(Ok(DataFrame::default()));
        assert_eq!(row_count(&state), "0");
    }
}
