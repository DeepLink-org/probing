use dioxus::prelude::*;
use crate::components::card::Card;
use crate::components::dataframe_view::DataFrameView;
use crate::components::page::{PageContainer, PageHeader};
use crate::components::common::{LoadingState, ErrorState};
use crate::hooks::use_api_simple;
use crate::api::ApiClient;
use probing_proto::prelude::DataFrame;
use crate::styles::{combinations::*, styles::*, conditional_class};

#[component]
pub fn Timeseries() -> Element {
    let tables_state = use_api_simple::<DataFrame>();
    
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
                title: "数据表列表",
                content_class: Some("") ,
                if tables_state.is_loading() {
                    LoadingState { message: Some("加载数据中...".to_string()) }
                } else if let Some(Ok(df)) = tables_state.data.read().as_ref() {
                    DataFrameView { df: df.clone() }
                } else if let Some(Err(err)) = tables_state.data.read().as_ref() {
                    ErrorState { error: format!("{:?}", err), title: None }
                }
            }

            Card {
                title: "查询工具",
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
                placeholder: "输入 SQL 查询，例如: SELECT * FROM table_name LIMIT 10",
                value: "{sql}",
                oninput: move |ev| sql.set(ev.value())
            }
            
            button {
                class: format!("{} {}", BUTTON_PRIMARY, conditional_class(*is_executing.read(), BUTTON_DISABLED, "")),
                onclick: execute_query,
                if *is_executing.read() { "执行查询中..." } else { "执行查询" }
            }
            
            if query_state.is_loading() {
                LoadingState { message: Some("执行查询中...".to_string()) }
            } else if let Some(Ok(df)) = query_state.data.read().as_ref() {
                DataFrameView { df: df.clone() }
            } else if let Some(Err(err)) = query_state.data.read().as_ref() {
                ErrorState { error: format!("{:?}", err), title: None }
            }
        }
    }
}