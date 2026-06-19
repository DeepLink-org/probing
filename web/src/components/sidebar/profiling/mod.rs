//! Profiling submenu, view switcher, and embedded controls in the sidebar.

use dioxus::prelude::*;
use dioxus_router::{Link, use_route};
use icondata::Icon as IconData;

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::components::sidebar::nav_item::sidebar_item_class;
use crate::state::profiling::{
    normalize_profiling_view, profiling_view_label, PROFILING_VIEWS,
};

mod controls;
use controls::{
    PprofControls, PyTorchTimelineControls, RayTimelineControls, TorchControls,
    TraceTimelineControls,
};

fn profiling_view_icon(id: &str) -> &'static IconData {
    match id {
        "pprof" => &icondata::CgPerformance,
        "torch" => &icondata::AiFireOutlined,
        "trace" => &icondata::AiThunderboltOutlined,
        "pytorch" => &icondata::SiPytorch,
        "ray" => &icondata::AiClockCircleOutlined,
        _ => &icondata::AiSearchOutlined,
    }
}

#[component]
pub fn ProfilingSidebarItem(show_dropdown: Signal<bool>) -> Element {
    let route = use_route::<Route>();
    let is_active = matches!(route, Route::ProfilingViewPage { .. });
    let expanded = *show_dropdown.read();
    let button_class = format!(
        "w-full focus:outline-none focus:ring-2 focus:ring-blue-400 focus:ring-offset-2 focus:ring-offset-slate-900 {}",
        sidebar_item_class(is_active)
    );

    let current_view = match route {
        Route::ProfilingViewPage { view } => normalize_profiling_view(&view).to_string(),
        _ => String::new(),
    };

    rsx! {
        div {
            button {
                class: "{button_class}",
                aria_expanded: if expanded { "true" } else { "false" },
                aria_label: "Profiling menu",
                title: "CPU/torch flamegraphs and chrome trace timelines",
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
                    for spec in PROFILING_VIEWS {
                        ProfilingSubItem {
                            view: spec.id.to_string(),
                            label: spec.sidebar_label.to_string(),
                            tooltip: spec.tooltip.to_string(),
                            icon: profiling_view_icon(spec.id),
                            current_view: current_view.clone(),
                        }
                    }

                    if is_active {
                        ProfilingControlsPanel { key: "{current_view}", current_view }
                    }
                }
            }
        }
    }
}

#[component]
fn ProfilingSubItem(
    view: String,
    label: String,
    tooltip: String,
    icon: &'static IconData,
    current_view: String,
) -> Element {
    let is_selected = current_view == view;
    let button_class = format!("w-full {}", sidebar_item_class(is_selected));
    let check_class = format!("ml-auto text-{} font-semibold", colors::PRIMARY_TEXT_DARK);

    rsx! {
        Link {
            to: Route::ProfilingViewPage { view: view.clone() },
            class: "{button_class}",
            title: "{tooltip}",
            Icon { icon, class: "w-4 h-4" }
            span { "{label}" }
            if is_selected {
                span { class: "{check_class}", "✓" }
            }
        }
    }
}

#[component]
fn ProfilingControlsPanel(current_view: String) -> Element {
    let panel_border_class = format!("mt-4 pt-4 border-t border-{}", colors::SIDEBAR_BORDER);
    let control_title_class =
        format!("text-xs font-semibold text-{}", colors::SIDEBAR_TEXT_SECONDARY);
    let control_value_class = format!("text-xs text-{}", colors::SIDEBAR_TEXT_MUTED);
    let toggle_enabled_class = format!(
        "relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors bg-{}",
        colors::PRIMARY
    );
    let toggle_disabled_class = format!(
        "relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors bg-{}",
        colors::SIDEBAR_ACTIVE_BG
    );
    let toggle_label_class = format!("text-xs text-{}", colors::SIDEBAR_TEXT_SECONDARY);
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
                class: "px-1 space-y-4",
                p {
                    class: "text-[10px] uppercase tracking-wide text-slate-500 mb-2",
                    "{profiling_view_label(&current_view)} controls"
                }
                match current_view.as_str() {
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
                    "trace" => rsx! {
                        TraceTimelineControls {
                            control_title_class: control_title_class.clone(),
                            control_value_class: control_value_class.clone(),
                            input_class: input_class.clone(),
                        }
                    },
                    "pytorch" => rsx! {
                        PyTorchTimelineControls {
                            control_title_class: control_title_class.clone(),
                            input_class: input_class.clone(),
                        }
                    },
                    "ray" => rsx! {
                        RayTimelineControls {
                            control_title_class: control_title_class.clone(),
                        }
                    },
                    _ => rsx! { div {} },
                }
            }
        }
    }
}
