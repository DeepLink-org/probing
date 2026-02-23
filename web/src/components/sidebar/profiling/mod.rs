//! Profiling submenu and view switcher. Uses [nav_item::sidebar_item_class](crate::components::sidebar::nav_item::sidebar_item_class) for style.

use dioxus::prelude::*;
use dioxus_router::{use_navigator, use_route};
use icondata::Icon as IconData;

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::components::sidebar::nav_item::sidebar_item_class;
use crate::state::profiling::PROFILING_VIEW;

mod controls;
use controls::{
    PprofControls, PyTorchTimelineControls, RayTimelineControls, TorchControls,
    TraceTimelineControls,
};

#[component]
pub fn ProfilingSidebarItem(show_dropdown: Signal<bool>) -> Element {
    let route = use_route::<Route>();
    let is_active = route == Route::ProfilingPage {};
    let expanded = *show_dropdown.read();
    let button_class = format!("w-full {} focus:outline-none focus:ring-2 focus:ring-blue-400 focus:ring-offset-2 focus:ring-offset-slate-900", sidebar_item_class(is_active));

    rsx! {
        div {
            button {
                class: "{button_class}",
                aria_expanded: if expanded { "true" } else { "false" },
                aria_label: "Profiling menu",
                onclick: move |_| {
                    let current = *show_dropdown.read();
                    *show_dropdown.write() = !current;
                },
                Icon { icon: &icondata::AiSearchOutlined, class: "w-4 h-4" }
                span { "Profiling" }
            }

            if expanded {
                div {
                    class: "ml-4 mt-0.5 space-y-0.5",
                    ProfilingSubItem {
                        view: "pprof".to_string(),
                        label: "pprof".to_string(),
                        icon: &icondata::CgPerformance,
                    }
                    ProfilingSubItem {
                        view: "torch".to_string(),
                        label: "torch".to_string(),
                        icon: &icondata::SiPytorch,
                    }
                    ProfilingSubItem {
                        view: "trace-timeline".to_string(),
                        label: "Trace".to_string(),
                        icon: &icondata::AiThunderboltOutlined,
                    }
                    ProfilingSubItem {
                        view: "pytorch-timeline".to_string(),
                        label: "PyTorch".to_string(),
                        icon: &icondata::SiPytorch,
                    }
                    ProfilingSubItem {
                        view: "ray-timeline".to_string(),
                        label: "Ray".to_string(),
                        icon: &icondata::AiClockCircleOutlined,
                    }

                    if is_active {
                        ProfilingControlsPanel {}
                    }
                }
            }
        }
    }
}

#[component]
pub fn ProfilingSubItem(view: String, label: String, icon: &'static IconData) -> Element {
    let route = use_route::<Route>();
    let navigator = use_navigator();
    let is_selected = *PROFILING_VIEW.read() == view;
    let is_on_profiling_page = route == Route::ProfilingPage {};
    let button_class = format!("w-full {}", sidebar_item_class(is_selected));
    let check_class = format!("ml-auto text-{} font-semibold", colors::PRIMARY_TEXT_DARK);

    rsx! {
        button {
            class: "{button_class}",
            onclick: {
                let v = view.clone();
                let nav = navigator.clone();
                let on_page = is_on_profiling_page;
                move |_| {
                    *PROFILING_VIEW.write() = v.clone();
                    if !on_page {
                        nav.push(Route::ProfilingPage {});
                    }
                }
            },
            Icon { icon, class: "w-4 h-4" }
            span { "{label}" }
            if is_selected {
                span { class: "{check_class}", "✓" }
            }
        }
    }
}

#[component]
pub fn ProfilingControlsPanel() -> Element {
    let current_view = PROFILING_VIEW.read();
    let panel_border_class = format!("mt-4 pt-4 border-t border-{}", colors::SIDEBAR_BORDER);
    let control_title_class =
        format!("text-xs font-semibold text-{}", colors::SIDEBAR_TEXT_SECONDARY);
    let control_value_class = format!("text-xs text-{}", colors::SIDEBAR_TEXT_MUTED);
    let toggle_enabled_class = format!(
        "relative inline-flex h-6 w-11 items-center rounded-full transition-colors w-full bg-{}",
        colors::PRIMARY
    );
    let toggle_disabled_class = format!(
        "relative inline-flex h-6 w-11 items-center rounded-full transition-colors w-full bg-{}",
        colors::SIDEBAR_ACTIVE_BG
    );
    let toggle_label_class =
        format!("ml-2 text-xs text-{}", colors::SIDEBAR_TEXT_SECONDARY);
    let input_class = format!(
        "w-full px-2 py-1 border border-{} bg-{} text-{} rounded text-xs focus:border-{} focus:outline-none",
        colors::SIDEBAR_INPUT_BORDER,
        colors::SIDEBAR_INPUT_BG,
        colors::SIDEBAR_TEXT_SECONDARY,
        colors::PRIMARY_BORDER
    );

    rsx! {
        div {
            class: "{panel_border_class}",
            div {
                class: "px-3 space-y-4",
                {
                    let view = (*current_view).clone();
                    let content: Element = match view.as_str() {
                        "pprof" => rsx! {
                            PprofControls {
                                control_title_class: control_title_class.clone(),
                                control_value_class: control_value_class.clone(),
                            }
                        },
                        "torch" => rsx! {
                            TorchControls {
                                control_title_class: control_title_class.clone(),
                                toggle_enabled_class: toggle_enabled_class.clone(),
                                toggle_disabled_class: toggle_disabled_class.clone(),
                                toggle_label_class: toggle_label_class.clone(),
                            }
                        },
                        "trace-timeline" => rsx! {
                            TraceTimelineControls {
                                control_title_class: control_title_class.clone(),
                                control_value_class: control_value_class.clone(),
                                input_class: input_class.clone(),
                            }
                        },
                        "pytorch-timeline" => rsx! {
                            PyTorchTimelineControls {
                                control_title_class: control_title_class.clone(),
                                input_class: input_class.clone(),
                            }
                        },
                        "ray-timeline" => rsx! {
                            RayTimelineControls {
                                control_title_class: control_title_class.clone(),
                                input_class: input_class.clone(),
                            }
                        },
                        _ => rsx! { div {} },
                    };
                    content
                }
            }
        }
    }
}
