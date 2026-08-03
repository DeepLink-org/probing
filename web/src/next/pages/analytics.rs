use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::ApiClient;
use crate::components::dataframe_view::DataFrameView;

use super::super::components::{
    ActionButton, ActionTone, FilterInput, LoadingPanel, SectionCard, UnavailablePanel,
    WorkspacePage,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct TableEntry {
    schema: String,
    table: String,
}

impl TableEntry {
    fn name(&self, global: bool) -> String {
        if global {
            format!("global.{}.{}", self.schema, self.table)
        } else {
            format!("{}.{}", self.schema, self.table)
        }
    }
}

#[component]
pub fn AnalyticsPage() -> Element {
    let mut global = use_signal(|| false);
    let mut filter = use_signal(String::new);
    let mut sql = use_signal(|| "SELECT * FROM python.backtrace LIMIT 10".to_string());
    let catalog = use_resource(move || {
        let global_value = global();
        async move {
            ApiClient::new()
                .execute_query(&catalog_sql(global_value))
                .await
        }
    });
    let mut execution =
        use_action(|query: String| async move { ApiClient::new().execute_query(&query).await });

    rsx! {
        WorkspacePage {
            title: "Analytics".to_string(),
            subtitle: "Browse the reported table contract, compose bounded SQL, and inspect the returned rows.".to_string(),
            actions: Some(rsx! {
                    div { class: "inline-flex rounded-lg border border-gray-300 bg-white p-0.5 text-xs",
                        button { class: scope_class(!global()), onclick: move |_| global.set(false), "Local" }
                        button { class: scope_class(global()), onclick: move |_| global.set(true), "global.*" }
                    }
                }),
            div { class: "grid items-start gap-4 xl:grid-cols-[320px_minmax(0,1fr)]",
                SectionCard {
                    title: "Catalog".to_string(),
                    subtitle: Some("Selecting a table only prepares SQL; it does not run it.".to_string()),
                    body_class: "p-0".to_string(),
                    FilterInput {
                        class: "m-3 w-[calc(100%-1.5rem)]".to_string(),
                        placeholder: "Filter schema or table".to_string(),
                        value: filter(),
                        oninput: move |value| filter.set(value),
                    }
                    match catalog.read().clone() {
                        None => rsx! { div { class: "p-3", LoadingPanel { label: "Loading table catalog".to_string() } } },
                        Some(Err(error)) => rsx! { div { class: "p-3", UnavailablePanel { label: "Catalog unavailable".to_string(), detail: error.display_message() } } },
                        Some(Ok(dataframe)) => rsx! { CatalogList { dataframe, filter: filter(), global: global(), on_select: move |entry: TableEntry| sql.set(format!("SELECT * FROM {} LIMIT 10", entry.name(global()))) } },
                    }
                }
                div { class: "min-w-0 space-y-4",
                    SectionCard {
                        title: "SQL".to_string(),
                        subtitle: Some("Queries execute against the same HTTP/DataFrame contract used by diagnostics.".to_string()),
                        textarea {
                            class: "h-36 w-full resize-y rounded-lg border border-gray-300 bg-gray-950 p-3 font-mono text-xs leading-relaxed text-gray-100",
                            value: "{sql}",
                            spellcheck: "false",
                            oninput: move |event| sql.set(event.value()),
                        }
                        div { class: "mt-3 flex items-center justify-between",
                            span { class: "text-xs text-gray-500", "Use LIMIT while exploring high-volume tables." }
                            ActionButton {
                                label: if execution.pending() { "Running…".to_string() } else { "Run query".to_string() },
                                tone: ActionTone::Primary,
                                disabled: execution.pending() || sql().trim().is_empty(),
                                onclick: move |_| execution.call(sql()),
                            }
                        }
                    }
                    SectionCard {
                        title: "Returned rows".to_string(),
                        subtitle: Some("The table below is the server response, without interpretation.".to_string()),
                        body_class: "p-0".to_string(),
                        match execution.value() {
                            None => rsx! { div { class: "p-4", UnavailablePanel { label: "No query executed".to_string(), detail: "Select a table or edit SQL, then run the query.".to_string() } } },
                            Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Query failed".to_string(), detail: error.to_string() } } },
                            Some(Ok(dataframe)) if dataframe().row_count() == 0 => rsx! { div { class: "p-4", UnavailablePanel { label: "No rows returned".to_string(), detail: "The query completed successfully with an empty result.".to_string() } } },
                            Some(Ok(dataframe)) => rsx! { DataFrameView { df: dataframe() } },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CatalogList(
    dataframe: DataFrame,
    filter: String,
    global: bool,
    on_select: EventHandler<TableEntry>,
) -> Element {
    let query = filter.trim().to_ascii_lowercase();
    let entries = parse_tables(&dataframe)
        .into_iter()
        .filter(|entry| {
            query.is_empty() || entry.name(global).to_ascii_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    rsx! {
        div { class: "max-h-[620px] overflow-y-auto border-t border-gray-100 p-2",
            if entries.is_empty() {
                p { class: "p-3 text-xs text-gray-500", "No matching tables" }
            }
            for entry in entries {
                button {
                    class: "block w-full rounded-lg px-3 py-2 text-left hover:bg-gray-50",
                    onclick: { let value = entry.clone(); move |_| on_select.call(value.clone()) },
                    div { class: "truncate font-mono text-xs font-medium text-gray-800", "{entry.table}" }
                    div { class: "truncate font-mono text-xs text-gray-500", "{entry.schema}" }
                }
            }
        }
    }
}

fn scope_class(active: bool) -> &'static str {
    if active {
        "rounded-md bg-blue-600 px-2.5 py-1 text-white"
    } else {
        "rounded-md px-2.5 py-1 text-gray-600 hover:bg-gray-50"
    }
}

fn catalog_sql(global: bool) -> String {
    if global {
        "SELECT table_schema, table_name FROM information_schema.tables WHERE table_catalog = 'global' ORDER BY table_schema, table_name".to_string()
    } else {
        "SELECT table_schema, table_name FROM information_schema.tables WHERE table_catalog = 'probe' AND table_schema <> 'information_schema' ORDER BY table_schema, table_name".to_string()
    }
}

fn parse_tables(dataframe: &DataFrame) -> Vec<TableEntry> {
    let schema = dataframe
        .names
        .iter()
        .position(|name| name == "table_schema")
        .unwrap_or(0);
    let table = dataframe
        .names
        .iter()
        .position(|name| name == "table_name")
        .unwrap_or(1);
    (0..dataframe.row_count())
        .filter_map(|row| {
            let Ele::Text(schema) = dataframe.cols.get(schema)?.get(row) else {
                return None;
            };
            let Ele::Text(table) = dataframe.cols.get(table)?.get(row) else {
                return None;
            };
            Some(TableEntry { schema, table })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn global_names_keep_catalog_prefix() {
        let entry = TableEntry {
            schema: "python".into(),
            table: "trace_event".into(),
        };
        assert_eq!(entry.name(true), "global.python.trace_event");
    }
}
