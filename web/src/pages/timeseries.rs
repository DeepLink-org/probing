use dioxus::prelude::*;
use crate::components::card::Card;
use crate::components::dataframe_view::DataFrameView;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api_simple;
use crate::api::ApiClient;
use probing_proto::prelude::{DataFrame, Ele};
use crate::styles::{combinations::*, styles::*, conditional_class};

#[component]
pub fn Timeseries() -> Element {
    let tables_state = use_api_simple::<DataFrame>();
    let preview_state = use_api_simple::<DataFrame>();
    let mut preview_title = use_signal(|| String::new());
    let mut preview_open = use_signal(|| false);
    
    use_effect(move || {
        let mut loading = tables_state.loading.clone();
        let mut data = tables_state.data.clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            data.set(Some(client.execute_query("show tables").await));
            loading.set(false);
        });
    });

    rsx! {
        PageContainer {
            PageHeader {
                title: "Time Series Analysis".to_string(),
                subtitle: Some("Analyze performance metrics over time".to_string())
            }
            
            Card {
                title: "Tables",
                content_class: Some("") ,
                if tables_state.is_loading() {
                    LoadingState { message: Some("Loading tables...".to_string()) }
                } else if let Some(Ok(df)) = tables_state.data.read().as_ref() {
                    {
                        let df_clone = df.clone();
                        let mut loading = preview_state.loading.clone();
                        let mut data = preview_state.data.clone();
                        let handler = EventHandler::new(move |row_idx: usize| {
                            // 取第二列 schema 与第三列 table
                            let schema = match df_clone.cols.get(1).map(|c| c.get(row_idx)) {
                                Some(Ele::Text(name)) => name.to_string(),
                                _ => return,
                            };
                            let table = match df_clone.cols.get(2).map(|c| c.get(row_idx)) {
                                Some(Ele::Text(name)) => name.to_string(),
                                _ => return,
                            };
                            let fqtn = format!("{}.{}", schema, table);
                            preview_title.set(format!("{} • latest 10 rows", fqtn));
                            preview_open.set(true);
                            spawn(async move {
                                loading.set(true);
                                let client = ApiClient::new();
                                let resp = client.execute_preview_last10(&fqtn).await;
                                data.set(Some(resp));
                                loading.set(false);
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
                    // 背景遮罩
                    div { class: "absolute inset-0 bg-black/50", onclick: move |_| preview_open.set(false) }
                    // 内容容器
                    div { class: "relative bg-white dark:bg-gray-800 rounded-lg shadow-lg max-w-5xl w-[90vw] max-h-[80vh] overflow-auto p-4",
                        // 头部
                        div { class: "flex items-center justify-between mb-3",
                            h3 { class: "text-lg font-semibold text-gray-900 dark:text-gray-100", "{preview_title}" }
                            button { class: "px-3 py-1 text-sm rounded bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600",
                                onclick: move |_| preview_open.set(false),
                                "Close"
                            }
                        }
                        // 内容
                        if preview_state.is_loading() {
                            LoadingState { message: Some("Loading preview...".to_string()) }
                        } else if let Some(Ok(df)) = preview_state.data.read().as_ref() {
                            DataFrameView { df: df.clone() }
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
        
        is_executing.set(true);
        let mut loading = query_state.loading.clone();
        let mut data = query_state.data.clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            data.set(Some(client.execute_query(&query).await));
            loading.set(false);
        });
        is_executing.set(false);
    };

    rsx! {
        div {
            class: SPACE_Y_4,
            textarea {
                class: TEXTAREA,
                placeholder: "Enter SQL, e.g. SELECT * FROM schema.table LIMIT 10",
                value: "{sql}",
                oninput: move |ev| sql.set(ev.value())
            }
            
            button {
                class: format!("{} {}", BUTTON_PRIMARY, conditional_class(*is_executing.read(), BUTTON_DISABLED, "")),
                onclick: execute_query,
                if *is_executing.read() { "Running..." } else { "Run Query" }
            }
            
            if query_state.is_loading() {
                LoadingState { message: Some("Running query...".to_string()) }
            } else if let Some(Ok(df)) = query_state.data.read().as_ref() {
                DataFrameView { df: df.clone() }
            } else if let Some(Err(err)) = query_state.data.read().as_ref() {
                ErrorState { error: format!("{:?}", err), title: None }
            }
        }
    }
}