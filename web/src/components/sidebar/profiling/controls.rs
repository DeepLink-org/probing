//! Profiling controls embedded in the left sidebar (dark theme).

use dioxus::prelude::*;

use crate::api::{ApiClient, ProfileResponse};
use crate::components::colors::colors;
use crate::hooks::use_api_simple;
use crate::state::profiling::{
    show_profiling_feedback, PROFILING_CHROME_LIMIT, PROFILING_PPROF_FREQ,
    PROFILING_PYTORCH_STEPS, PROFILING_PYTORCH_TIMELINE_RELOAD, PROFILING_RAY_TIMELINE_RELOAD,
    PROFILING_TORCH_ENABLED, PROFILING_TRACE_RELOAD,
};

const PPROF_FREQ_VALUES: [i32; 6] = [0, 10, 100, 1000, 10000, 100000];

fn pprof_freq_index(freq: i32) -> usize {
    match freq {
        f if f <= 0 => 0,
        f if f <= 10 => 1,
        f if f <= 100 => 2,
        f if f <= 1000 => 3,
        f if f <= 10000 => 4,
        _ => 5,
    }
}

#[component]
pub fn PprofControls(control_title_class: String, control_value_class: String) -> Element {
    let freq = *PROFILING_PPROF_FREQ.read();
    let current_idx = pprof_freq_index(freq);
    let label = PPROF_FREQ_VALUES[current_idx];

    rsx! {
        div {
            class: "space-y-2",
            div { class: "{control_title_class}", "Pprof Frequency" }
            div {
                class: "space-y-1",
                div {
                    class: "{control_value_class} flex items-center justify-between",
                    span { "{label} Hz" }
                }
                input {
                    r#type: "range",
                    min: "0",
                    max: "5",
                    step: "1",
                    value: "{current_idx}",
                    class: "w-full accent-blue-500",
                    onchange: move |ev| {
                        if let Ok(idx) = ev.value().parse::<usize>() {
                            if idx < PPROF_FREQ_VALUES.len() {
                                let mapped = PPROF_FREQ_VALUES[idx];
                                let previous = *PROFILING_PPROF_FREQ.read();
                                *PROFILING_PPROF_FREQ.write() = mapped;
                                let expr = if mapped <= 0 {
                                    "set probing.pprof.sample_freq=;".to_string()
                                } else {
                                    format!("set probing.pprof.sample_freq={mapped};")
                                };
                                spawn(async move {
                                    match ApiClient::new().execute_query(&expr).await {
                                        Ok(_) => show_profiling_feedback("Setting applied", false),
                                        Err(err) => {
                                            *PROFILING_PPROF_FREQ.write() = previous;
                                            show_profiling_feedback(err.display_message(), true);
                                        }
                                    }
                                });
                            }
                        }
                    },
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

    rsx! {
        div {
            class: "space-y-2",
            div { class: "{control_title_class}", "Torch Profiling" }
            label {
                class: "flex items-center gap-2 cursor-pointer select-none",
                input {
                    r#type: "checkbox",
                    class: "sr-only",
                    checked: is_enabled,
                    onchange: move |_| {
                        let previous = *PROFILING_TORCH_ENABLED.read();
                        let next = !previous;
                        *PROFILING_TORCH_ENABLED.write() = next;
                        let expr = if next {
                            "set probing.torch.profiling=on;".to_string()
                        } else {
                            "set probing.torch.profiling=;".to_string()
                        };
                        spawn(async move {
                            match ApiClient::new().execute_query(&expr).await {
                                Ok(_) => show_profiling_feedback("Setting applied", false),
                                Err(err) => {
                                    *PROFILING_TORCH_ENABLED.write() = previous;
                                    show_profiling_feedback(err.display_message(), true);
                                }
                            }
                        });
                    },
                }
                span {
                    class: if is_enabled { "{toggle_enabled_class}" } else { "{toggle_disabled_class}" },
                    span {
                        class: if is_enabled {
                            "inline-block h-4 w-4 transform rounded-full bg-white transition-transform translate-x-5"
                        } else {
                            "inline-block h-4 w-4 transform rounded-full bg-white transition-transform translate-x-1"
                        },
                    }
                }
                span { class: "{toggle_label_class}",
                    if is_enabled { "Enabled" } else { "Disabled" }
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
    let limit = *PROFILING_CHROME_LIMIT.read();

    rsx! {
        div {
            class: "space-y-3",
            div {
                class: "space-y-1",
                div { class: "{control_title_class}", "Event Limit" }
                div {
                    class: "flex items-center gap-2",
                    span { class: "{control_value_class}", "{limit}" }
                    input {
                        r#type: "range",
                        min: "100",
                        max: "5000",
                        step: "100",
                        value: "{limit}",
                        class: "flex-1 accent-blue-500",
                        oninput: move |ev| {
                            if let Ok(val) = ev.value().parse::<usize>() {
                                *PROFILING_CHROME_LIMIT.write() = val;
                            }
                        },
                    }
                }
            }
            button {
                class: format!(
                    "w-full px-2 py-1.5 text-xs font-medium rounded bg-{} text-white hover:bg-{}",
                    colors::PRIMARY,
                    colors::PRIMARY_HOVER
                ),
                onclick: move |_| {
                    *PROFILING_TRACE_RELOAD.write() += 1;
                    show_profiling_feedback("Reloading timeline…", false);
                },
                "Reload Timeline"
            }
        }
    }
}

#[component]
pub fn RayTimelineControls(control_title_class: String) -> Element {
    rsx! {
        div {
            class: "space-y-2",
            div { class: "{control_title_class}", "Ray Timeline" }
            button {
                class: format!(
                    "w-full px-2 py-1.5 text-xs font-medium rounded bg-{} text-white hover:bg-{}",
                    colors::PRIMARY,
                    colors::PRIMARY_HOVER
                ),
                onclick: move |_| {
                    *PROFILING_RAY_TIMELINE_RELOAD.write() += 1;
                    show_profiling_feedback("Reloading timeline…", false);
                },
                "Reload Ray Timeline"
            }
        }
    }
}

#[component]
pub fn PyTorchTimelineControls(control_title_class: String, input_class: String) -> Element {
    let profile_state = use_api_simple::<ProfileResponse>();
    let mut timeline_loading = use_signal(|| false);

    rsx! {
        div {
            class: "space-y-3",
            div {
                class: "space-y-2",
                div { class: "{control_title_class}", "Steps" }
                input {
                    r#type: "number",
                    min: "1",
                    max: "100",
                    value: "{*PROFILING_PYTORCH_STEPS.read()}",
                    class: "{input_class}",
                    oninput: move |ev| {
                        if let Ok(val) = ev.value().parse::<i32>() {
                            *PROFILING_PYTORCH_STEPS.write() = val.clamp(1, 100);
                        }
                    },
                }
            }
            div {
                class: "space-y-2",
                button {
                    class: format!(
                        "w-full px-2 py-1.5 text-xs font-medium rounded bg-{} text-white hover:bg-{} disabled:opacity-50",
                        colors::SUCCESS,
                        colors::SUCCESS_HOVER
                    ),
                    disabled: profile_state.is_loading(),
                    onclick: {
                        let mut profile_state = profile_state.clone();
                        move |_| {
                            spawn(async move {
                                *profile_state.loading.write() = true;
                                let client = ApiClient::new();
                                let n = *PROFILING_PYTORCH_STEPS.read();
                                match client.start_pytorch_profile(n).await {
                                    Ok(res) if res.success => {
                                        show_profiling_feedback(
                                            res.message.unwrap_or_else(|| "Profile started".to_string()),
                                            false,
                                        );
                                    }
                                    Ok(res) => {
                                        show_profiling_feedback(
                                            res.error.unwrap_or_else(|| "Failed to start".to_string()),
                                            true,
                                        );
                                    }
                                    Err(err) => show_profiling_feedback(err.display_message(), true),
                                }
                                *profile_state.loading.write() = false;
                            });
                        }
                    },
                    if profile_state.is_loading() { "Starting…" } else { "Start Profile" }
                }
                button {
                    class: format!(
                        "w-full px-2 py-1.5 text-xs font-medium rounded bg-{} text-white hover:bg-{} disabled:opacity-50",
                        colors::PRIMARY,
                        colors::PRIMARY_HOVER
                    ),
                    disabled: timeline_loading(),
                    onclick: move |_| {
                        timeline_loading.set(true);
                        spawn(async move {
                            let client = ApiClient::new();
                            match client.get_pytorch_timeline().await {
                                Ok(_) => {
                                    *PROFILING_PYTORCH_TIMELINE_RELOAD.write() += 1;
                                    show_profiling_feedback("Timeline loaded", false);
                                }
                                Err(err) => show_profiling_feedback(err.display_message(), true),
                            }
                            timeline_loading.set(false);
                        });
                    },
                    if timeline_loading() { "Loading…" } else { "Load Timeline" }
                }
            }
        }
    }
}
