use dioxus::prelude::*;
use crate::components::common::{LoadingState, ErrorState};
use crate::components::nav_drawer::NavDrawer;
use crate::hooks::use_api_simple;
use crate::api::ApiClient;

/// 从配置中更新本地状态
fn apply_config(config: &[(String, String)], mut pprof_freq: Signal<i32>, mut torch_enabled: Signal<bool>) {
    *pprof_freq.write() = 0;
    *torch_enabled.write() = false;
    
    for (name, value) in config {
        match name.as_str() {
            "probing.pprof.sample_freq" => {
                if let Ok(v) = value.parse::<i32>() {
                    *pprof_freq.write() = v.max(0);
                }
            },
            "probing.torch.profiling" => {
                let lowered = value.trim().to_lowercase();
                let enabled = !lowered.is_empty()
                    && lowered != "0"
                    && lowered != "false"
                    && lowered != "off"
                    && lowered != "disable"
                    && lowered != "disabled";
                *torch_enabled.write() = enabled;
            },
            _ => {}
        }
    }
}

#[component]
pub fn Profiler() -> Element {
    let mut selected_tab = use_signal(|| "pprof".to_string());
    let mut pprof_freq = use_signal(|| 99_i32);
    let torch_enabled = use_signal(|| false);
    
    let config_state = use_api_simple::<Vec<(String, String)>>();
    let flamegraph_state = use_api_simple::<String>();
    
    use_effect(move || {
        let mut loading = config_state.loading;
        let mut data = config_state.data;
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_profiler_config().await;
            match result {
                Ok(ref config) => {
                    apply_config(config, pprof_freq, torch_enabled);
                }
                Err(_) => {}
            }
            *data.write() = Some(result);
            *loading.write() = false;
        });
    });

    use_effect(move || {
        let tab = selected_tab.read().clone();
        drop(tab);
        spawn(async move {
            let client = ApiClient::new();
            if let Ok(config) = client.get_profiler_config().await {
                apply_config(&config, pprof_freq, torch_enabled);
            }
        });
    });

    use_effect(move || {
        let tab = selected_tab.read().clone();
        let pprof_on = *pprof_freq.read() > 0;
        let torch = *torch_enabled.read();
        
        let active_profiler = match (tab.as_str(), pprof_on, torch) {
            ("pprof", true, _) => "pprof",
            ("torch", _, true) => "torch",
            _ => return,
        };
        
        let mut loading = flamegraph_state.loading;
        let mut data = flamegraph_state.data;
        spawn(async move {
            *loading.write() = true;
            let client = ApiClient::new();
            let result = client.get_flamegraph(active_profiler).await;
            *data.write() = Some(result);
            *loading.write() = false;
        });
    });

    rsx! {
        div {
            class: "flex h-screen bg-gray-50",
            NavDrawer {
                selected_tab: selected_tab,
                pprof_freq: pprof_freq,
                torch_enabled: torch_enabled,
                on_tab_change: move |tab| {
                    *selected_tab.write() = tab;
                    spawn(async move {
                        let client = ApiClient::new();
                        if let Ok(config) = client.get_profiler_config().await {
                            apply_config(&config, pprof_freq, torch_enabled);
                        }
                    });
                },
                on_pprof_freq_change: move |new_freq| {
                    // 本地更新+回写服务端
                    *pprof_freq.write() = new_freq;
                    spawn(async move {
                        let client = ApiClient::new();
                        let expr = if new_freq <= 0 { "set probing.pprof.sample_freq=;".to_string() } else { format!("set probing.pprof.sample_freq={};", new_freq) };
                        let _ = client.execute_query(&expr).await;
                    });
                },
                on_torch_toggle: move |enabled| {
                    let mut torch_enabled = torch_enabled;
                    spawn(async move {
                        let client = ApiClient::new();
                        let expr = if enabled {
                            "set probing.torch.profiling=on;".to_string()
                        } else {
                            "set probing.torch.profiling=;".to_string()
                        };
                        let _ = client.execute_query(&expr).await;
                        *torch_enabled.write() = enabled;
                    });
                },
            }
            
            div {
                class: "flex-1 flex flex-col min-w-0",
                div {
                class: "flex-1 w-full relative",
                if !(*pprof_freq.read() > 0) && !*torch_enabled.read() {
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
                h2 { class: "text-2xl font-bold text-gray-900 mb-4", "No Profilers Enabled" }
                p { class: "text-gray-600 mb-6", "{message}" }
            }
        }
    }
}