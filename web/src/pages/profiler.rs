use dioxus::prelude::*;
use crate::components::common::{LoadingState, ErrorState};
use crate::components::nav_drawer::NavDrawer;
use crate::hooks::use_api_simple;
use crate::api::ApiClient;

/// 从配置中更新本地状态
fn apply_config(config: &[(String, String)], mut pprof_freq: Signal<i32>, mut torch_enabled: Signal<bool>) {
    // 先重置本地状态
    pprof_freq.set(0);
    torch_enabled.set(false);
    
    for (name, value) in config {
        match name.as_str() {
            "probing.pprof.sample_freq" => {
                if let Ok(v) = value.parse::<i32>() {
                    pprof_freq.set(v.max(0));
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
                torch_enabled.set(enabled);
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
    
    // 加载配置
    use_effect(move || {
        let mut loading = config_state.loading.clone();
        let mut data = config_state.data.clone();
        let pprof_freq_clone = pprof_freq.clone();
        let torch_enabled_clone = torch_enabled.clone();
        spawn(async move {
            loading.set(true);
            let client = ApiClient::new();
            match client.get_profiler_config().await {
                Ok(config) => {
                    apply_config(&config, pprof_freq_clone, torch_enabled_clone);
                    data.set(Some(Ok(config)));
                }
                Err(err) => data.set(Some(Err(err))),
            }
            loading.set(false);
        });
    });

    // 切换 Tab 时与服务端重新对账，保持同步
    use_effect(move || {
        let _tab = selected_tab.read().clone(); // depend on tab change
        let pprof_freq_clone = pprof_freq.clone();
        let torch_enabled_clone = torch_enabled.clone();
        spawn(async move {
            let client = ApiClient::new();
            if let Ok(config) = client.get_profiler_config().await {
                apply_config(&config, pprof_freq_clone, torch_enabled_clone);
            }
        });
    });

    // 加载火焰图
    use_effect(move || {
        let tab = selected_tab.read().clone();
        let pprof_on = *pprof_freq.read() > 0;
        let torch = *torch_enabled.read();
        
        let active_profiler = match (tab.as_str(), pprof_on, torch) {
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
            class: "flex h-screen bg-gray-50",
            NavDrawer {
                selected_tab: selected_tab,
                pprof_freq: pprof_freq,
                torch_enabled: torch_enabled,
                on_tab_change: move |tab| {
                    // 立即切换 Tab 并主动对账一次，避免延迟
                    selected_tab.set(tab);
                    let pprof_freq = pprof_freq.clone();
                    let torch_enabled = torch_enabled.clone();
                    spawn(async move {
                        let client = ApiClient::new();
                        // reset -> fetch -> apply
                        if let Ok(config) = client.get_profiler_config().await {
                            apply_config(&config, pprof_freq, torch_enabled);
                        }
                    });
                },
                on_pprof_freq_change: move |new_freq| {
                    // 本地更新+回写服务端
                    pprof_freq.set(new_freq);
                    spawn(async move {
                        let client = ApiClient::new();
                        let expr = if new_freq <= 0 { "set probing.pprof.sample_freq=;".to_string() } else { format!("set probing.pprof.sample_freq={};", new_freq) };
                        let _ = client.execute_query(&expr).await;
                    });
                },
                on_torch_toggle: move |enabled| {
                    let mut torch_enabled_clone = torch_enabled.clone();
                    spawn(async move {
                        let client = ApiClient::new();
                        // torch: 使用 profiling 规格字符串表示开关；启用时设为 "on"，否则清空
                        let expr = if enabled {
                            "set probing.torch.profiling=on;".to_string()
                        } else {
                            "set probing.torch.profiling=;".to_string()
                        };
                        let _ = client.execute_query(&expr).await;
                        torch_enabled_clone.set(enabled);
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