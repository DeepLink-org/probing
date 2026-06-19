//! Stack controls in the left sidebar — collapsible like Profiling.

use dioxus::prelude::*;
use dioxus_router::{use_route, Link};

use crate::app::Route;
use crate::components::colors::colors;
use crate::components::icon::Icon;
use crate::components::sidebar::nav_item::sidebar_item_class;
use crate::state::stack::{bump_stack_refresh, stack_tid_label, STACK_MODE, STACK_SNAPSHOT};
use crate::utils::callframe::{mode_for_kind, FrameKind};

#[component]
pub fn StackSidebarItem(show_dropdown: Signal<bool>) -> Element {
    let route = use_route::<Route>();
    let is_active = matches!(route, Route::StackPage {} | Route::StackWithTidPage { .. });
    let expanded = *show_dropdown.read();
    let on_default = matches!(route, Route::StackPage {});
    let tid = match &route {
        Route::StackWithTidPage { tid } => Some(tid.clone()),
        _ => None,
    };

    let button_class = format!(
        "w-full focus:outline-none focus:ring-2 focus:ring-blue-400 focus:ring-offset-2 focus:ring-offset-slate-900 {}",
        sidebar_item_class(is_active)
    );

    rsx! {
        div {
            button {
                class: "{button_class}",
                aria_expanded: if expanded { "true" } else { "false" },
                aria_label: "Stacks menu",
                title: "Live call stack — expand for filters",
                onclick: move |_| {
                    let current = *show_dropdown.read();
                    *show_dropdown.write() = !current;
                },
                Icon { icon: &icondata::AiApartmentOutlined, class: "w-4 h-4" }
                span { "Stacks" }
            }

            if expanded {
                div {
                    class: "ml-4 mt-0.5 space-y-0.5",
                    StackSubLink {
                        label: "Default thread",
                        hint: "Process default sampling thread",
                        is_selected: on_default,
                        route: Route::StackPage {},
                    }
                    if let Some(tid) = tid {
                        StackSubLink {
                            label: format!("tid {tid}"),
                            hint: "Stack for a specific thread (from Dashboard)",
                            is_selected: true,
                            route: Route::StackWithTidPage { tid: tid.clone() },
                        }
                    }
                    if is_active {
                        StackControlsPanel {}
                    }
                }
            }
        }
    }
}

#[component]
fn StackSubLink(label: String, hint: String, is_selected: bool, route: Route) -> Element {
    let class = format!("w-full {}", sidebar_item_class(is_selected));
    rsx! {
        Link {
            to: route,
            class: "{class}",
            title: "{hint}",
            span { class: "flex-1 min-w-0 truncate text-sm", "{label}" }
            if is_selected {
                span { class: "ml-auto text-[10px] text-blue-300/80 shrink-0", "✓" }
            }
        }
    }
}

#[component]
fn StackControlsPanel() -> Element {
    let route = use_route::<Route>();
    let tid = match route {
        Route::StackWithTidPage { tid } => Some(tid),
        _ => None,
    };
    let tid_label = stack_tid_label(tid.as_deref());
    let snapshot = STACK_SNAPSHOT.read().clone();
    let mode = STACK_MODE();
    let panel_border = colors::SIDEBAR_PANEL_BORDER;
    let title_class = colors::SIDEBAR_CONTROL_TITLE;
    let value_class = colors::SIDEBAR_CONTROL_VALUE;

    rsx! {
        div {
            class: "{panel_border}",
            div {
                class: "px-1 space-y-2.5",
                p {
                    class: "text-[10px] uppercase tracking-wide text-slate-500",
                    "Stack"
                }
                p {
                    class: "{value_class} font-mono text-[11px]",
                    title: "Current stack target",
                    "{tid_label}"
                }
                if snapshot.loaded {
                    p {
                        class: "{value_class} text-[11px] leading-snug tabular-nums",
                        "{snapshot.shown}/{snapshot.total} shown · py {snapshot.py} · rust {snapshot.rust} · native {snapshot.cpp}"
                    }
                }
                div { class: "space-y-1",
                    p { class: "{title_class} text-[10px]", "Filter" }
                    div { class: "flex flex-wrap gap-1",
                        FilterChip { filter: "mixed", label: "All", active: mode == "mixed" }
                        FilterChip {
                            filter: mode_for_kind(FrameKind::Python),
                            label: "Py",
                            active: mode == mode_for_kind(FrameKind::Python),
                        }
                        FilterChip {
                            filter: mode_for_kind(FrameKind::Rust),
                            label: "Rs",
                            active: mode == mode_for_kind(FrameKind::Rust),
                        }
                        FilterChip {
                            filter: mode_for_kind(FrameKind::Cpp),
                            label: "Nat",
                            active: mode == mode_for_kind(FrameKind::Cpp),
                        }
                    }
                }
                button {
                    r#type: "button",
                    class: "w-full px-2 py-1 text-[11px] rounded-md border border-slate-600 bg-slate-800/80 text-slate-300 hover:bg-slate-700 transition-colors",
                    onclick: move |_| bump_stack_refresh(),
                    "Refresh"
                }
            }
        }
    }
}

#[component]
fn FilterChip(filter: &'static str, label: &'static str, active: bool) -> Element {
    let class = if active {
        "px-1.5 py-0.5 text-[10px] rounded border border-blue-500/50 bg-blue-600/25 text-blue-100"
    } else {
        "px-1.5 py-0.5 text-[10px] rounded border border-slate-600/80 text-slate-400 hover:text-slate-200 hover:border-slate-500"
    };
    rsx! {
        button {
            r#type: "button",
            class: "{class}",
            onclick: move |_| {
                if STACK_MODE() == filter && filter != "mixed" {
                    *STACK_MODE.write() = String::from("mixed");
                } else {
                    *STACK_MODE.write() = filter.to_string();
                }
            },
            "{label}"
        }
    }
}
