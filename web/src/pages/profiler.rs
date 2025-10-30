use dioxus::prelude::*;
use crate::components::common::{LoadingState, ErrorState};
use crate::components::nav_drawer::NavDrawer;
use crate::hooks::use_api_simple;
use crate::api::ApiClient;

#[component]
pub fn Profiler() -> Element {
    let mut selected_tab = use_signal(|| "pprof".to_string());
    let mut pprof_enabled = use_signal(|| false);
    let mut torch_enabled = use_signal(|| false);
    
    let config_state = use_api_simple::<Vec<Vec<String>>>();
    let flamegraph_state = use_api_simple::<String>();
    
    // 加载配置
    use_effect(move || {
        let mut loading = config_state.loading.clone();
        let mut data = config_state.data.clone();
        let mut pprof_enabled = pprof_enabled.clone();
        let mut torch_enabled = torch_enabled.clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            match client.get_profiler_config().await {
                Ok(config) => {
                    for row in &config {
                        if row.len() >= 2 {
                            match row[0].as_str() {
                                "probing.pprof.sample_freq" if !row[1].is_empty() => pprof_enabled.set(true),
                                "probing.torch.sample_ratio" if !row[1].is_empty() => torch_enabled.set(true),
                                _ => {}
                            }
                        }
                    }
                    data.set(Some(Ok(config)));
                }
                Err(err) => data.set(Some(Err(err))),
            }
            loading.set(false);
        });
    });

    // 加载火焰图
    use_effect(move || {
        let tab = selected_tab.read().clone();
        let pprof = *pprof_enabled.read();
        let torch = *torch_enabled.read();
        
        let active_profiler = match (tab.as_str(), pprof, torch) {
            ("pprof", true, _) => "pprof",
            ("torch", _, true) => "torch",
            _ => return,
        };
        
        let mut loading = flamegraph_state.loading.clone();
        let mut data = flamegraph_state.data.clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            data.set(Some(client.get_flamegraph(active_profiler).await));
            loading.set(false);
        });
    });

    rsx! {
        div {
            class: "flex h-screen bg-gray-50 dark:bg-gray-900",
            NavDrawer {
                selected_tab: selected_tab,
                pprof_enabled: pprof_enabled,
                torch_enabled: torch_enabled,
                on_tab_change: move |tab| selected_tab.set(tab),
                on_pprof_toggle: move |enabled| pprof_enabled.set(enabled),
                on_torch_toggle: move |enabled| torch_enabled.set(enabled),
            }
            
            div {
                class: "flex-1 flex flex-col min-w-0",
                div {
                    class: "flex-1 w-full relative",
                    if !*pprof_enabled.read() && !*torch_enabled.read() {
                        EmptyState {
                            message: "No profilers are currently enabled. Enable a profiler using the switches in the sidebar.".to_string()
                        }
                    } else if flamegraph_state.is_loading() {
                        LoadingState { message: Some("Loading flamegraph...".to_string()) }
                    } else if let Some(Ok(flamegraph)) = flamegraph_state.data.read().as_ref() {
                        div {
                            class: "absolute inset-0 w-full h-full",
                            div {
                                class: "w-full h-full",
                                dangerous_inner_html: "{flamegraph}"
                            }
                        }
                    } else if let Some(Err(err)) = flamegraph_state.data.read().as_ref() {
                        ErrorState {
                            error: format!("Failed to load flamegraph: {:?}", err),
                            title: Some("Error Loading Flamegraph".to_string())
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EmptyState(message: String) -> Element {
    rsx! {
        div {
            class: "absolute inset-0 flex items-center justify-center",
            div {
                class: "text-center",
                h2 { class: "text-2xl font-bold text-gray-900 dark:text-white mb-4", "No Profilers Enabled" }
                p { class: "text-gray-600 dark:text-gray-400 mb-6", "{message}" }
            }
        }
    }
}