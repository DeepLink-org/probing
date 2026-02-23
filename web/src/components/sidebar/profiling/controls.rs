use dioxus::prelude::*;

use crate::api::{ApiClient, ProfileResponse};
use crate::components::colors::colors;
use crate::hooks::use_api_simple;
use crate::state::profiling::{
    PROFILING_CHROME_LIMIT, PROFILING_PPROF_FREQ, PROFILING_PYTORCH_STEPS,
    PROFILING_PYTORCH_TIMELINE_RELOAD, PROFILING_RAY_TIMELINE_RELOAD, PROFILING_TORCH_ENABLED,
};

#[component]
pub fn PprofControls(control_title_class: String, control_value_class: String) -> Element {
    const FREQ_VALUES: [i32; 4] = [0, 10, 100, 1000];

    let freq = *PROFILING_PPROF_FREQ.read();
    let current_idx = match freq {
        f if f <= 0 => 0,
        f if f <= 10 => 1,
        f if f <= 100 => 2,
        _ => 3,
    };
    let label = FREQ_VALUES[current_idx];

    rsx! {
        div {
            class: "space-y-2",
            div {
                class: "{control_title_class}",
                "Pprof Frequency"
            }
            div {
                class: "space-y-1",
                div {
                    class: "{control_value_class} flex items-center justify-between",
                    span { "{label} Hz" }
                }
                input {
                    r#type: "range",
                    min: "0",
                    max: "3",
                    step: "1",
                    value: "{current_idx}",
                    class: "w-full",
                    oninput: move |ev| {
                        if let Ok(idx) = ev.value().parse::<usize>() {
                            if idx < FREQ_VALUES.len() {
                                let mapped = FREQ_VALUES[idx];
                                *PROFILING_PPROF_FREQ.write() = mapped;
                                spawn(async move {
                                    let client = ApiClient::new();
                                    let expr = if mapped <= 0 {
                                        "set probing.pprof.sample_freq=;".to_string()
                                    } else {
                                        format!("set probing.pprof.sample_freq={};", mapped)
                                    };
                                    let _ = client.execute_query(&expr).await;
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn TorchControls(
    control_title_class: String,
    toggle_enabled_class: String,
    toggle_disabled_class: String,
    toggle_label_class: String,
) -> Element {
    let is_enabled = *PROFILING_TORCH_ENABLED.read();
    let toggle_class = if is_enabled {
        toggle_enabled_class.clone()
    } else {
        toggle_disabled_class.clone()
    };

    rsx! {
        div {
            class: "space-y-2",
            div {
                class: "{control_title_class}",
                "Torch Profiling"
            }
            button {
                class: "{toggle_class}",
                onclick: move |_| {
                    let enabled = !*PROFILING_TORCH_ENABLED.read();
                    spawn(async move {
                        let client = ApiClient::new();
                        let expr = if enabled {
                            "set probing.torch.profiling=on;".to_string()
                        } else {
                            "set probing.torch.profiling=;".to_string()
                        };
                        let _ = client.execute_query(&expr).await;
                        *PROFILING_TORCH_ENABLED.write() = enabled;
                    });
                },
                span {
                    class: "inline-block h-4 w-4 transform rounded-full bg-white transition-transform",
                    class: if *PROFILING_TORCH_ENABLED.read() {
                        "translate-x-6"
                    } else {
                        "translate-x-1"
                    }
                }
                span {
                    class: "{toggle_label_class}",
                    if *PROFILING_TORCH_ENABLED.read() {
                        "Enabled"
                    } else {
                        "Disabled"
                    }
                }
            }
        }
    }
}

#[component]
pub fn TraceTimelineControls(
    control_title_class: String,
    control_value_class: String,
    input_class: String,
) -> Element {
    rsx! {
        div {
            class: "space-y-3",
            div {
                class: "space-y-1",
                div {
                    class: "{control_title_class}",
                    "Event Limit"
                }
                div {
                    class: "flex items-center gap-2",
                    span {
                        class: "{control_value_class}",
                        "{*PROFILING_CHROME_LIMIT.read()}"
                    }
                    input {
                        r#type: "range",
                        min: "100",
                        max: "5000",
                        step: "100",
                        value: "{*PROFILING_CHROME_LIMIT.read()}",
                        class: "flex-1",
                        oninput: move |ev| {
                            if let Ok(val) = ev.value().parse::<usize>() {
                                *PROFILING_CHROME_LIMIT.write() = val;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn RayTimelineControls(control_title_class: String, input_class: String) -> Element {
    rsx! {
        div {
            class: "space-y-3",
            div {
                class: "space-y-2",
                div {
                    class: "{control_title_class}",
                    "Ray Timeline Controls"
                }
                button {
                    class: format!("w-full px-3 py-2 text-xs font-medium rounded bg-{} text-white hover:bg-{}", colors::PRIMARY, colors::PRIMARY_HOVER),
                    onclick: move |_| {
                        *PROFILING_RAY_TIMELINE_RELOAD.write() += 1;
                    },
                    "Reload Ray Timeline"
                }
            }
        }
    }
}

#[component]
pub fn PyTorchTimelineControls(control_title_class: String, input_class: String) -> Element {
    let pytorch_profile_state = use_api_simple::<ProfileResponse>();
    let pytorch_timeline_state = use_api_simple::<String>();

    rsx! {
        div {
            class: "space-y-3",
            div {
                class: "space-y-2",
                div {
                    class: "{control_title_class}",
                    "Steps"
                }
                input {
                    r#type: "number",
                    min: "1",
                    max: "100",
                    value: "{*PROFILING_PYTORCH_STEPS.read()}",
                    class: "{input_class}",
                    oninput: move |ev| {
                        if let Ok(val) = ev.value().parse::<i32>() {
                            *PROFILING_PYTORCH_STEPS.write() = val.max(1).min(100);
                        }
                    }
                }
            }
            div {
                class: "space-y-2",
                button {
                    class: format!("w-full px-3 py-2 text-xs font-medium rounded bg-{} text-white hover:bg-{} disabled:bg-gray-400 disabled:cursor-not-allowed", colors::SUCCESS, colors::SUCCESS_HOVER),
                    disabled: pytorch_profile_state.is_loading(),
                    onclick: {
                        let mut profile_state = pytorch_profile_state.clone();
                        move |_| {
                            spawn(async move {
                                *profile_state.loading.write() = true;
                                let client = ApiClient::new();
                                let steps = *PROFILING_PYTORCH_STEPS.read();
                                let result = client.start_pytorch_profile(steps).await;
                                *profile_state.data.write() = Some(result);
                                *profile_state.loading.write() = false;
                            });
                        }
                    },
                    if pytorch_profile_state.is_loading() {
                        "Starting..."
                    } else {
                        "Start Profile"
                    }
                }
                button {
                    class: format!("w-full px-3 py-2 text-xs font-medium rounded bg-{} text-white hover:bg-{} disabled:bg-gray-400 disabled:cursor-not-allowed", colors::PRIMARY, colors::PRIMARY_HOVER),
                    disabled: pytorch_timeline_state.is_loading(),
                    onclick: {
                        let mut timeline_state = pytorch_timeline_state.clone();
                        move |_| {
                            spawn(async move {
                                *timeline_state.loading.write() = true;
                                *timeline_state.data.write() = None;
                                let client = ApiClient::new();
                                let result = client.get_pytorch_timeline().await;
                                let is_ok = result.is_ok();
                                *timeline_state.data.write() = Some(result);
                                *timeline_state.loading.write() = false;
                                if is_ok {
                                    *PROFILING_PYTORCH_TIMELINE_RELOAD.write() += 1;
                                }
                            });
                        }
                    },
                    if pytorch_timeline_state.is_loading() {
                        "Loading..."
                    } else {
                        "Load Timeline"
                    }
                }
            }
            if let Some(Ok(ref profile_result)) = pytorch_profile_state.data.read().as_ref() {
                if profile_result.success {
                    div {
                        class: format!("p-1.5 bg-{} border border-{} rounded text-xs text-{}", colors::SUCCESS_LIGHT, colors::SUCCESS_BORDER, colors::SUCCESS_TEXT),
                        if let Some(ref msg) = profile_result.message {
                            "{msg}"
                        } else {
                            "Profile started"
                        }
                    }
                } else {
                    div {
                        class: format!("p-1.5 bg-{} border border-{} rounded text-xs text-{}", colors::ERROR_LIGHT, colors::ERROR_BORDER, colors::ERROR_TEXT),
                        if let Some(ref err) = profile_result.error {
                            "{err}"
                        } else {
                            "Failed to start"
                        }
                    }
                }
            }
        }
    }
}
