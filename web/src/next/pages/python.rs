use dioxus::prelude::*;

use crate::api::{ApiClient, TraceableItem, VariableRecord};

use super::super::components::{
    ActionButton, ActionTone, FilterInput, LoadingPanel, SectionCard, UnavailablePanel,
    WorkspacePage,
};

#[component]
pub fn PythonPage() -> Element {
    let mut refresh = use_signal(|| 0_u32);
    let mut selected = use_signal(String::new);
    let mut watch = use_signal(String::new);
    let mut filter = use_signal(String::new);
    let mut records_for = use_signal(String::new);
    let active = use_resource(move || {
        let refresh_tick = refresh();
        async move {
            let _ = refresh_tick;
            ApiClient::new().get_trace_info().await
        }
    });
    let catalog = use_resource(|| async move { ApiClient::new().get_traceable_items(None).await });
    let record_function = records_for();
    let records = use_resource(move || {
        let function = record_function.clone();
        let refresh_tick = refresh();
        async move {
            let _ = refresh_tick;
            if function.is_empty() {
                Ok(Vec::new())
            } else {
                ApiClient::new()
                    .get_trace_variables(Some(&function), Some(100))
                    .await
            }
        }
    });
    let mut start = use_action(move |(function, variables): (String, String)| async move {
        let variables = variables
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let response = ApiClient::new()
            .start_trace(
                &function,
                (!variables.is_empty()).then_some(variables),
                false,
            )
            .await?;
        refresh += 1;
        Ok::<_, crate::utils::error::AppError>(response)
    });
    let mut stop = use_action(move |function: String| async move {
        let response = ApiClient::new().stop_trace(&function).await?;
        refresh += 1;
        Ok::<_, crate::utils::error::AppError>(response)
    });

    rsx! {
        WorkspacePage {
            title: "Python variable tracing".to_string(),
            subtitle: "Watch reported variable changes at selected functions; this is separate from distributed spans and profiler timelines.".to_string(),
            SectionCard {
                title: "Trace request".to_string(),
                subtitle: Some("The target and watch list below are sent explicitly when Start is pressed.".to_string()),
                div { class: "grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]",
                    label { class: "text-xs font-medium uppercase tracking-wide text-gray-500",
                        "Function"
                        input { class: "mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-xs", placeholder: "module.function", value: "{selected}", oninput: move |event| selected.set(event.value()) }
                    }
                    label { class: "text-xs font-medium uppercase tracking-wide text-gray-500",
                        "Variables (comma-separated)"
                        input { class: "mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 font-mono text-xs", placeholder: "loss, hidden_states", value: "{watch}", oninput: move |event| watch.set(event.value()) }
                    }
                    div { class: "self-end",
                        ActionButton {
                            label: if start.pending() { "Starting…".to_string() } else { "Start trace".to_string() },
                            tone: ActionTone::Primary,
                            disabled: selected().trim().is_empty() || start.pending(),
                            onclick: move |_| start.call((selected(), watch())),
                        }
                    }
                }
                if let Some(Err(error)) = start.value() {
                    p { class: "mt-2 text-xs text-red-700", "Start failed: {error}" }
                }
            }
            div { class: "grid items-start gap-4 xl:grid-cols-[420px_minmax(0,1fr)]",
                SectionCard { title: "Active watches".to_string(), subtitle: Some("Select Records to inspect the latest returned values.".to_string()), body_class: "p-0".to_string(),
                    match active.read().clone() {
                        None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading active watches".to_string() } } },
                        Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Active watches unavailable".to_string(), detail: error.display_message() } } },
                        Some(Ok(items)) if items.is_empty() => rsx! { div { class: "p-4", UnavailablePanel { label: "No active watches".to_string(), detail: "Choose a function from the catalog and start a trace.".to_string() } } },
                        Some(Ok(items)) => rsx! { div { class: "divide-y divide-gray-100", for function in items { ActiveWatch { function, selected: records_for(), stop_pending: stop.pending(), on_records: move |value| records_for.set(value), on_stop: move |value| stop.call(value) } } } },
                    }
                }
                SectionCard { title: if records_for().is_empty() { "Variable records".to_string() } else { format!("Variable records · {}", records_for()) }, subtitle: Some("Latest 100 changes returned for the selected function.".to_string()), body_class: "p-0".to_string(),
                    match records.read().clone() {
                        None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading records".to_string() } } },
                        Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Variable records unavailable".to_string(), detail: error.display_message() } } },
                        Some(Ok(rows)) if rows.is_empty() => rsx! { div { class: "p-4", UnavailablePanel { label: "No variable records".to_string(), detail: if records_for().is_empty() { "Select Records on an active watch.".to_string() } else { "The watch has not reported a variable change yet.".to_string() } } } },
                        Some(Ok(rows)) => rsx! { VariableRecords { rows } },
                    }
                }
            }
            SectionCard { title: "Traceable catalog".to_string(), subtitle: Some("Selecting an item prepares the request above; it does not start tracing.".to_string()), body_class: "p-0".to_string(),
                FilterInput { class: "m-3 w-[calc(100%-1.5rem)]".to_string(), placeholder: "Filter module, function, or variable".to_string(), value: filter(), oninput: move |value| filter.set(value) }
                match catalog.read().clone() {
                    None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading traceable catalog".to_string() } } },
                    Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Traceable catalog unavailable".to_string(), detail: error.display_message() } } },
                    Some(Ok(items)) => rsx! { TraceCatalog { items, filter: filter(), on_select: move |item: TraceableItem| { selected.set(item.name); watch.set(item.variables.join(", ")); } } },
                }
            }
        }
    }
}

#[component]
fn ActiveWatch(
    function: String,
    selected: String,
    stop_pending: bool,
    on_records: EventHandler<String>,
    on_stop: EventHandler<String>,
) -> Element {
    rsx! { div { class: if selected == function { "flex items-center gap-2 bg-blue-50 px-4 py-3" } else { "flex items-center gap-2 px-4 py-3" },
        span { class: "min-w-0 flex-1 break-all font-mono text-xs text-gray-800", "{function}" }
        ActionButton { label: "Records".to_string(), compact: true, onclick: { let value = function.clone(); move |_| on_records.call(value.clone()) } }
        ActionButton { label: "Stop".to_string(), tone: ActionTone::Danger, compact: true, disabled: stop_pending, onclick: { let value = function.clone(); move |_| on_stop.call(value.clone()) } }
    } }
}

#[component]
fn TraceCatalog(
    items: Vec<TraceableItem>,
    filter: String,
    on_select: EventHandler<TraceableItem>,
) -> Element {
    let needle = filter.trim().to_ascii_lowercase();
    let visible = items
        .into_iter()
        .filter(|item| {
            needle.is_empty()
                || item.name.to_ascii_lowercase().contains(&needle)
                || item
                    .variables
                    .iter()
                    .any(|value| value.to_ascii_lowercase().contains(&needle))
        })
        .collect::<Vec<_>>();
    rsx! { div { class: "grid max-h-[520px] grid-cols-2 gap-2 overflow-y-auto border-t border-gray-100 p-2 xl:grid-cols-3",
        for item in visible { button { class: "min-w-0 rounded-lg border border-gray-200 bg-white p-3 text-left hover:border-blue-300 hover:bg-blue-50", onclick: { let value = item.clone(); move |_| on_select.call(value.clone()) },
            div { class: "truncate font-mono text-xs font-medium text-gray-800", "{item.name}" }
            div { class: "mt-1 text-xs text-gray-500", "{item.item_type} · {item.variables.len()} variables" }
        } }
    } }
}

#[component]
fn VariableRecords(rows: Vec<VariableRecord>) -> Element {
    rsx! { div { class: "max-h-[420px] overflow-y-auto divide-y divide-gray-100",
        for row in rows { div { class: "grid grid-cols-[140px_minmax(0,1fr)_110px] gap-3 px-4 py-2 text-xs",
            span { class: "break-all font-mono text-gray-700", "{row.variable_name}" }
            span { class: "break-all font-mono text-gray-900", "{row.value}" }
            span { class: "truncate font-mono text-gray-500", "{row.value_type}" }
        } }
    } }
}

#[cfg(test)]
mod tests {
    #[test]
    fn watch_list_ignores_empty_items() {
        let items = "loss, , hidden"
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(items, vec!["loss", "hidden"]);
    }
}
