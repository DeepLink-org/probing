use dioxus::prelude::*;

use crate::api::{ApiClient, ProfileResponse};
use crate::components::chrome_tracing_iframe::ChromeTracingIframe;
use crate::components::colors::colors;
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api_simple;

#[component]
pub fn ChromeTracing() -> Element {
    let mut data_source = use_signal(|| "trace".to_string()); // "trace" or "pytorch"
    let limit = use_signal(|| 1000usize);
    let pytorch_steps = use_signal(|| 5i32);
    let state = use_api_simple::<String>();
    let profile_state = use_api_simple::<ProfileResponse>();
    let iframe_key = use_signal(|| 0);

    // Create dependency, recalculate when limit changes
    let limit_value = use_memo({
        let limit = limit.clone();
        move || *limit.read()
    });

    // Create data source dependency
    let data_source_value = use_memo({
        let data_source = data_source.clone();
        move || data_source.read().clone()
    });

    // Refetch data when data source or limit changes (only for trace data source)
    use_effect({
        let data_source_value = data_source_value.clone();
        let limit_value = limit_value.clone();
        let mut loading = state.loading;
        let mut data = state.data;
        let mut iframe_key = iframe_key.clone();
        move || {
            let source = data_source_value.read().clone();
            let limit_val = *limit_value.read();
            if source == "trace" {
                spawn(async move {
                    *loading.write() = true;
                    let client = ApiClient::new();
                    let result = client.get_chrome_tracing_json(Some(limit_val)).await;
                    *data.write() = Some(result);
                    *loading.write() = false;
                    // Update iframe key to force reload
                    *iframe_key.write() += 1;
                });
            }
        }
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Chrome Tracing".to_string(),
                subtitle: Some("Chrome DevTools timeline".to_string()),
                icon: Some(&icondata::AiThunderboltOutlined),
            }

            // Data source selector
            div {
                class: "mb-4 p-4 bg-white rounded-lg shadow",
                div {
                    class: "flex items-center space-x-4 mb-4",
                    span {
                        class: "text-sm font-medium text-gray-700",
                        "Data Source:"
                    }
                    button {
                        class: if *data_source.read() == "trace" {
                            format!("px-4 py-2 text-sm font-medium rounded-md bg-{} text-white", colors::PRIMARY)
                        } else {
                            format!("px-4 py-2 text-sm font-medium rounded-md bg-{} text-gray-700 hover:bg-{}", colors::BTN_SECONDARY_BG, colors::BTN_SECONDARY_HOVER)
                        },
                        onclick: move |_| *data_source.write() = "trace".to_string(),
                        "Trace Events"
                    }
                    button {
                        class: if *data_source.read() == "pytorch" {
                            format!("px-4 py-2 text-sm font-medium rounded-md bg-{} text-white", colors::PRIMARY)
                        } else {
                            format!("px-4 py-2 text-sm font-medium rounded-md bg-{} text-gray-700 hover:bg-{}", colors::BTN_SECONDARY_BG, colors::BTN_SECONDARY_HOVER)
                        },
                        onclick: move |_| *data_source.write() = "pytorch".to_string(),
                        "PyTorch Profiler"
                    }
                }

                // Trace Events controls
                if *data_source.read() == "trace" {
                    div {
                        class: "space-y-2",
                        div {
                            class: "flex items-center justify-between",
                            span {
                                class: "text-sm text-gray-600",
                                "Number of Events"
                            }
                            span {
                                class: "text-sm text-gray-800 font-mono",
                                "{*limit.read()} events"
                            }
                        }
                        input {
                            r#type: "range",
                            min: "100",
                            max: "5000",
                            step: "100",
                            value: "{*limit.read()}",
                            class: "w-full",
                            oninput: {
                                let mut limit = limit.clone();
                                move |ev| {
                                    if let Ok(val) = ev.value().parse::<usize>() {
                                        *limit.write() = val;
                                    }
                                }
                            }
                        }
                        div {
                            class: "flex justify-between text-xs text-gray-500",
                            span { "100" }
                            span { "5000" }
                        }
                    }
                }

                // PyTorch Profiler controls
                if *data_source.read() == "pytorch" {
                    div {
                        class: "space-y-4",
                        div {
                            class: "space-y-2",
                            div {
                                class: "flex items-center justify-between",
                                span {
                                    class: "text-sm text-gray-600",
                                    "Number of Steps"
                                }
                                input {
                                    r#type: "number",
                                    min: "1",
                                    max: "100",
                                    value: "{*pytorch_steps.read()}",
                                    class: "w-20 px-2 py-1 border border-gray-300 rounded text-sm",
                                    oninput: {
                                        let mut steps = pytorch_steps.clone();
                                        move |ev| {
                                            if let Ok(val) = ev.value().parse::<i32>() {
                                                *steps.write() = val.max(1).min(100);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        div {
                            class: "flex items-center space-x-4",
                            button {
                                class: format!("px-4 py-2 text-sm font-medium rounded-md bg-{} text-white hover:bg-{} disabled:bg-gray-400 disabled:cursor-not-allowed", colors::SUCCESS, colors::SUCCESS_HOVER),
                                disabled: profile_state.is_loading(),
                                onclick: {
                                    let mut profile_state = profile_state.clone();
                                    let steps = pytorch_steps.clone();
                                    move |_| {
                                        spawn(async move {
                                            *profile_state.loading.write() = true;
                                            let client = ApiClient::new();
                                            let result = client.start_pytorch_profile(*steps.read()).await;
                                            *profile_state.data.write() = Some(result);
                                            *profile_state.loading.write() = false;
                                        });
                                    }
                                },
                                if profile_state.is_loading() {
                                    "Starting Profile..."
                                } else {
                                    "Start Profile"
                                }
                            }
                            button {
                                class: format!("px-4 py-2 text-sm font-medium rounded-md bg-{} text-white hover:bg-{} disabled:bg-gray-400 disabled:cursor-not-allowed", colors::PRIMARY, colors::PRIMARY_HOVER),
                                disabled: state.is_loading(),
                                onclick: {
                                    let mut state = state.clone();
                                    let mut iframe_key = iframe_key.clone();
                                    move |_| {
                                        spawn(async move {
                                            *state.loading.write() = true;
                                            *state.data.write() = None; // Clear previous data
                                            let client = ApiClient::new();
                                            let result = client.get_pytorch_timeline().await;
                                            match &result {
                                                Ok(ref data) => {
                                                    log::info!("PyTorch timeline loaded successfully, length: {}", data.len());
                                                }
                                                Err(ref err) => {
                                                    log::error!("Failed to load PyTorch timeline: {:?}", err);
                                                }
                                            }
                                            *state.data.write() = Some(result);
                                            *state.loading.write() = false;
                                            *iframe_key.write() += 1;
                                        });
                                    }
                                },
                                if state.is_loading() {
                                    "Loading Timeline..."
                                } else {
                                    "Load Timeline"
                                }
                            }
                        }
                        if let Some(Ok(ref profile_result)) = profile_state.data.read().as_ref() {
                            if profile_result.success {
                                div {
                                    class: "mt-2 p-2 bg-green-50 border border-green-200 rounded text-sm text-green-800",
                                    if let Some(ref msg) = profile_result.message {
                                        "{msg}"
                                    } else {
                                        "Profile started successfully"
                                    }
                                }
                            } else {
                                div {
                                    class: format!("mt-2 p-2 bg-{} border border-{} rounded text-sm text-{}", colors::ERROR_LIGHT, colors::ERROR_BORDER, colors::ERROR_TEXT),
                                    if let Some(ref err) = profile_result.error {
                                        "{err}"
                                    } else {
                                        "Failed to start profile"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Chrome Tracing Viewer (shared component + custom no-data placeholder)
            {
                let show_placeholder = !state.is_loading() && state.data.read().as_ref().is_none();
                if show_placeholder {
                    rsx! {
                            div {
                                class: "bg-white rounded-lg shadow p-8 text-center",
                                div {
                                    class: "text-gray-500",
                                if *data_source.read() == "pytorch" {
                                    p { class: "mb-4 text-lg", "PyTorch Profiler Timeline" }
                                    p { class: "text-sm", "Click 'Start Profile' to begin profiling, then click 'Load Timeline' to view the results." }
                                } else {
                                    p { class: "mb-4 text-lg", "Trace Events Timeline" }
                                    p { class: "text-sm", "Select the number of events and the timeline will load automatically." }
                                }
                            }
                        }
                    }
                } else {
                    rsx! {
                        ChromeTracingIframe {
                            state: state.clone(),
                            iframe_key: iframe_key,
                            loading_message: Some(if *data_source.read() == "pytorch" {
                                "Loading PyTorch timeline data...".to_string()
                            } else {
                                "Loading trace data...".to_string()
                            }),
                            empty_message: Some("Timeline data is empty. Make sure the profiler has been executed.".to_string()),
                        }
                    }
                }
            }
        }
    }
}
