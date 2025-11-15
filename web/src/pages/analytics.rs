use dioxus::prelude::*;
use crate::components::card::Card;
use crate::components::dataframe_view::DataFrameView;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::{use_api, use_api_simple};
use crate::api::ApiClient;
use probing_proto::prelude::{DataFrame, Ele};

#[component]
pub fn Analytics() -> Element {
    let tables_state = use_api(|| {
        let client = ApiClient::new();
        async move { client.execute_query("show tables").await }
    });
    let preview_state = use_api_simple::<DataFrame>();
    let mut preview_title = use_signal(|| String::new());
    let mut preview_open = use_signal(|| false);

    rsx! {
        PageContainer {
            PageTitle {
                title: "Analytics".to_string(),
                subtitle: Some("Query and analyze performance data with SQL".to_string()),
                icon: Some(&icondata::AiAreaChartOutlined),
            }
            Card {
                title: "Tables",
                content_class: Some("") ,
                if tables_state.is_loading() {
                    LoadingState { message: Some("Loading tables...".to_string()) }
                } else if let Some(Ok(df)) = tables_state.data.read().as_ref() {
                    {
                        let mut loading = preview_state.loading;
                        let mut data = preview_state.data;
                        let handler = EventHandler::new(move |row_idx: usize| {
                            let df_ref = tables_state.data.read();
                            let Some(Ok(df)) = df_ref.as_ref() else { return };
                            // Get second column (schema) and third column (table)
                            let schema = match df.cols.get(1).map(|c| c.get(row_idx)) {
                                Some(Ele::Text(name)) => name.to_string(),
                                _ => return,
                            };
                            let table = match df.cols.get(2).map(|c| c.get(row_idx)) {
                                Some(Ele::Text(name)) => name.to_string(),
                                _ => return,
                            };
                            let fqtn = format!("{}.{}", schema, table);
                            *preview_title.write() = format!("{} • latest 10 rows", fqtn);
                            *preview_open.write() = true;
                            spawn(async move {
                                *loading.write() = true;
                                let client = ApiClient::new();
                                let resp = client.execute_preview_last10(&fqtn).await;
                                *data.write() = Some(resp);
                                *loading.write() = false;
                            });
                        });
                        rsx!{ DataFrameView { df: df.clone(), on_row_click: Some(handler) } }
                    }
                } else if let Some(Err(err)) = tables_state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                }
            }

            // Preview Modal
            if *preview_open.read() {
                div { class: "fixed inset-0 z-50 flex items-center justify-center",
                    // Background overlay
                    div { class: "absolute inset-0 bg-black/50", onclick: move |_| {
                        *preview_open.write() = false;
                    } }
                    // Content container
                    div { class: "relative bg-white rounded-lg shadow-lg max-w-5xl w-[90vw] max-h-[80vh] overflow-auto p-4",
                        // Header
                        div { class: "flex items-center justify-between mb-3",
                            h3 { class: "text-lg font-semibold text-gray-900", "{preview_title}" }
                            button { class: "px-3 py-1 text-sm rounded bg-gray-100 hover:bg-gray-200",
                                onclick: move |_| {
                                    *preview_open.write() = false;
                                },
                                "Close"
                            }
                        }
                        // Content
                        if preview_state.is_loading() {
                            LoadingState { message: Some("Loading preview...".to_string()) }
                        } else if let Some(Ok(df)) = preview_state.data.read().as_ref() {
                            DataFrameView { df: df.clone(), on_row_click: None }
                        } else if let Some(Err(err)) = preview_state.data.read().as_ref() {
                            ErrorState { error: format!("{:?}", err), title: None }
                        } else {
                            span { class: "text-gray-500", "Preparing preview..." }
                        }
                    }
                }
            }
            Card {
                title: "Query",
                SqlQueryPanel {}
            }
        }
    }
}

#[component]
fn SqlQueryPanel() -> Element {
    let mut sql = use_signal(|| String::new());
    let query_state = use_api_simple::<DataFrame>();
    let mut is_executing = use_signal(|| false);

    let execute_query = move |_| {
        let query = sql.read().clone();
        if query.trim().is_empty() {
            return;
        }
        
        *is_executing.write() = true;
        let mut loading = query_state.loading;
        let mut data = query_state.data;
        let query_clone = query.clone();
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.execute_query(&query_clone).await;
            *data.write() = Some(result);
            *loading.write() = false;
            *is_executing.write() = false;
        });
    };

    rsx! {
        div {
            class: "space-y-4",
            textarea {
                class: "w-full min-h-[120px] font-mono text-sm p-3 rounded border border-gray-300 bg-white",
                placeholder: "Enter SQL, e.g. SELECT * FROM schema.table LIMIT 10",
                value: "{sql}",
                oninput: move |ev| {
                    *sql.write() = ev.value();
                }
            }
            
            button {
                class: format!("px-6 py-2 bg-indigo-600 text-white rounded-md font-medium hover:bg-indigo-700 transition-colors shadow-sm {}", if *is_executing.read() { "opacity-50 cursor-not-allowed" } else { "" }),
                disabled: *is_executing.read(),
                onclick: execute_query,
                if *is_executing.read() { "Running..." } else { "Run Query" }
            }
            
            if query_state.is_loading() {
                LoadingState { message: Some("Running query...".to_string()) }
            } else if let Some(Ok(df)) = query_state.data.read().as_ref() {
                DataFrameView { df: df.clone(), on_row_click: None }
            } else if let Some(Err(err)) = query_state.data.read().as_ref() {
                ErrorState { error: format!("{:?}", err), title: None }
            }
        }
    }
}