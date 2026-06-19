use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::components::colors::colors;
use crate::components::card::Card;
use crate::components::callstack_view::CallStackView;
use crate::components::common::{EmptyState, ErrorState, LoadingState};
use crate::components::icon::{Icon, RustIcon};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_api;
use crate::api::ApiClient;
use crate::app::Route;
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::utils::callframe::{count_by_kind, matches_mode, mode_for_kind, FrameKind};

#[component]
pub fn Stack(tid: Option<String>) -> Element {
    let navigator = use_navigator();
    let tid_display = tid.clone();
    let tid_for_redirect = tid.clone();
    let mut mode = use_signal(|| String::from("mixed"));

    use_effect(move || {
        if tid_for_redirect.is_none() {
            if let Some(ctx_tid) = INVESTIGATION_CONTEXT.read().tid {
                navigator.replace(Route::StackWithTidPage {
                    tid: ctx_tid.to_string(),
                });
            }
        }
    });

    let state = use_api(move || {
        let tid_clone = tid.clone();
        let client = ApiClient::new();
        async move { client.get_callstack_with_mode(tid_clone, "mixed").await }
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Stacks".to_string(),
                subtitle: tid_display.as_ref().map(|t| format!("Call stack for thread {t}")),
                icon: Some(&icondata::AiApartmentOutlined),
            }
            Card {
                title: "Call Stack",
                header_right: Some(rsx! {
                    ModeSelector { mode: mode }
                }),
                if state.is_loading() {
                    LoadingState { message: Some("Loading call stack...".to_string()) }
                } else if let Some(Ok(callframes)) = state.data.read().as_ref() {
                    {
                        let current_mode = mode.read().clone();
                        let filtered: Vec<_> = callframes
                            .iter()
                            .filter(|cf| matches_mode(cf, current_mode.as_str()))
                            .cloned()
                            .collect();
                        let (py_count, rust_count, cpp_count) = count_by_kind(callframes);
                        let filtered_len = filtered.len();

                        rsx! {
                            div { class: "space-y-4",
                                StackSummary {
                                    total: callframes.len(),
                                    py: py_count,
                                    rust: rust_count,
                                    cpp: cpp_count,
                                    shown: filtered_len,
                                    mode: current_mode.clone(),
                                    on_filter: move |next: &'static str| *mode.write() = next.to_string(),
                                }
                                if filtered.is_empty() {
                                    EmptyState {
                                        message: if callframes.is_empty() {
                                            "No call stack data available".to_string()
                                        } else {
                                            format!("No frames match the \"{}\" filter", mode_label(&current_mode))
                                        }
                                    }
                                } else {
                                    div { class: "pt-1",
                                        for (idx , cf) in filtered.iter().enumerate() {
                                            CallStackView {
                                                callstack: cf.clone(),
                                                index: idx,
                                                is_last: idx + 1 == filtered_len,
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(Err(err)) = state.data.read().as_ref() {
                    ErrorState { error: err.display_message(), title: Some("Failed to load stack".to_string()) }
                }
            }
        }
    }
}

#[component]
fn ModeSelector(mode: Signal<String>) -> Element {
    let active = |key: &str| {
        if mode.read().as_str() == key {
            format!("bg-{} text-white shadow-sm ring-1 ring-{}", colors::PRIMARY, colors::PRIMARY)
        } else {
            format!(
                "bg-{} text-gray-600 hover:bg-{} hover:text-gray-900",
                colors::BTN_SECONDARY_BG,
                colors::BTN_SECONDARY_HOVER
            )
        }
    };

    rsx! {
        div { class: "inline-flex items-center gap-1 p-1 rounded-lg bg-gray-100/80",
            ModeButton {
                label: "All",
                active_class: active("mixed"),
                onclick: move |_| *mode.write() = String::from("mixed"),
            }
            ModeButton {
                icon: rsx! { Icon { icon: &icondata::SiPython, class: "w-3.5 h-3.5" } },
                label: "Python",
                active_class: active("py"),
                onclick: move |_| *mode.write() = String::from("py"),
            }
            ModeButton {
                icon: rsx! { RustIcon { class: "w-3.5 h-3.5" } },
                label: "Rust",
                active_class: active("rust"),
                onclick: move |_| *mode.write() = String::from("rust"),
            }
            ModeButton {
                icon: rsx! { Icon { icon: &icondata::SiCplusplus, class: "w-3.5 h-3.5" } },
                label: "Native",
                active_class: active("cpp"),
                onclick: move |_| *mode.write() = String::from("cpp"),
            }
        }
    }
}

#[component]
fn ModeButton(
    label: &'static str,
    active_class: String,
    onclick: EventHandler<MouseEvent>,
    #[props(optional)] icon: Option<Element>,
) -> Element {
    rsx! {
        button {
            class: "inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs font-medium transition-colors {active_class}",
            onclick: move |e| onclick.call(e),
            if let Some(icon_el) = icon {
                {icon_el}
            }
            "{label}"
        }
    }
}

#[component]
fn StackSummary(
    total: usize,
    py: usize,
    rust: usize,
    cpp: usize,
    shown: usize,
    mode: String,
    on_filter: EventHandler<&'static str>,
) -> Element {
    let mode_for_py = mode.clone();
    let mode_for_rust = mode.clone();
    let mode_for_cpp = mode.clone();
    let chip_active = |key: &str| {
        if mode == key {
            "ring-2 ring-blue-400 border-blue-300 bg-blue-50/50"
        } else {
            "hover:border-gray-300 hover:bg-gray-50 cursor-pointer"
        }
    };

    rsx! {
        div { class: "flex flex-wrap items-center gap-2 text-sm",
            span { class: "text-gray-600",
                if mode == "mixed" {
                    "{total} frames"
                } else {
                    "Showing {shown} of {total}"
                }
            }
            SummaryChip {
                icon: rsx! { Icon { icon: &icondata::SiPython, class: "w-3.5 h-3.5 text-emerald-600" } },
                count: py,
                label: "Python",
                active_class: chip_active("py"),
                onclick: move |_| {
                    if mode_for_py == "py" {
                        on_filter.call("mixed");
                    } else {
                        on_filter.call(mode_for_kind(FrameKind::Python));
                    }
                },
            }
            SummaryChip {
                icon: rsx! { RustIcon { class: "w-3.5 h-3.5 text-orange-600" } },
                count: rust,
                label: "Rust",
                active_class: chip_active("rust"),
                onclick: move |_| {
                    if mode_for_rust == "rust" {
                        on_filter.call("mixed");
                    } else {
                        on_filter.call(mode_for_kind(FrameKind::Rust));
                    }
                },
            }
            SummaryChip {
                icon: rsx! { Icon { icon: &icondata::SiCplusplus, class: "w-3.5 h-3.5 text-blue-600" } },
                count: cpp,
                label: "Native",
                active_class: chip_active("cpp"),
                onclick: move |_| {
                    if mode_for_cpp == "cpp" {
                        on_filter.call("mixed");
                    } else {
                        on_filter.call(mode_for_kind(FrameKind::Cpp));
                    }
                },
            }
            if mode != "mixed" {
                button {
                    class: "text-xs text-gray-500 hover:text-gray-800 underline underline-offset-2",
                    onclick: move |_| on_filter.call("mixed"),
                    "Clear filter"
                }
            }
        }
    }
}

#[component]
fn SummaryChip(
    icon: Element,
    count: usize,
    label: &'static str,
    active_class: &'static str,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            type: "button",
            class: "inline-flex items-center gap-1.5 px-2 py-1 rounded-full bg-white border border-gray-200 text-xs text-gray-600 transition-colors {active_class}",
            onclick: move |_| onclick.call(()),
            {icon}
            span { class: "font-medium tabular-nums", "{count}" }
            span { class: "text-gray-400", "{label}" }
        }
    }
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "py" => "Python",
        "rust" => "Rust",
        "cpp" => "Native",
        _ => "All",
    }
}
