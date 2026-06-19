use std::collections::HashMap;

use dioxus::prelude::*;
use probing_proto::prelude::{CallFrame, Value};

use crate::components::icon::{Icon, RustIcon};
use crate::components::source_viewer::{SourceLocationLink, SourceViewerCard};
use crate::components::workspace::{AccentSurface, StatusBadge, SurfaceCardBody};
use crate::utils::callframe::{classify_frame, frame_ip, frame_location, frame_title, FrameKind};
use crate::utils::source_ref::file_display_name;

#[component]
pub fn CallStackView(
    callstack: CallFrame,
    index: usize,
    is_last: bool,
    #[props(optional, default = false)] default_open: bool,
) -> Element {
    let kind = classify_frame(&callstack);
    let title = frame_title(&callstack);
    let location = frame_location(&callstack);
    let ip = frame_ip(&callstack);
    let mut open = use_signal(|| default_open);

    let (badge_label, badge_cls) = kind.status_badge();
    let connector = if is_last { "hidden" } else { "block" };
    let location_link = frame_location_link(&callstack, location.as_ref());

    rsx! {
        div { class: "relative flex gap-0 pb-4 last:pb-0",
            div { class: "relative flex flex-col items-center shrink-0 w-8 pt-3",
                span {
                    class: "relative z-10 inline-flex items-center justify-center w-6 h-6 rounded-full ring-4 {kind.timeline_ring()} bg-white text-[10px] font-bold tabular-nums text-gray-600",
                    "{index}"
                }
                span {
                    class: "absolute top-[1.125rem] left-1/2 -translate-x-1/2 w-2 h-2 rounded-full {kind.timeline_dot()} z-20"
                }
                div { class: "absolute left-1/2 -translate-x-1/2 top-8 bottom-0 w-px bg-gradient-to-b from-gray-200 to-transparent {connector}" }
            }

            div { class: "flex-1 min-w-0",
                AccentSurface {
                    accent: kind.accent_border(),
                    div {
                        class: "w-full px-3 py-2.5 bg-gradient-to-r from-slate-50/80 to-white border-b border-gray-100",
                        div { class: "flex items-start gap-2 min-w-0",
                            div { class: "shrink-0 mt-0.5", {frame_icon(kind)} }
                            div { class: "flex-1 min-w-0",
                                div {
                                    class: "text-sm font-mono font-medium text-gray-900 truncate cursor-pointer hover:text-gray-700",
                                    title: "{title}",
                                    onclick: move |_| {
                                        let cur = *open.read();
                                        *open.write() = !cur;
                                    },
                                    "{title}"
                                }
                                if let Some((path, line, label)) = location_link {
                                    SourceLocationLink {
                                        path: path,
                                        line: line,
                                        label: Some(label),
                                        class: "text-[11px] mt-0.5 block truncate max-w-full".to_string(),
                                    }
                                }
                            }
                            div { class: "flex items-center gap-1.5 shrink-0 pt-0.5",
                                StatusBadge { label: badge_label, badge_class: badge_cls }
                                button {
                                    r#type: "button",
                                    class: "p-0.5 rounded hover:bg-gray-100 transition-transform duration-200",
                                    class: if *open.read() { "rotate-180" } else { "" },
                                    aria_label: "Toggle frame details",
                                    onclick: move |_| {
                                        let cur = *open.read();
                                        *open.write() = !cur;
                                    },
                                    Icon { icon: &icondata::AiDownOutlined, class: "w-3.5 h-3.5 text-gray-400" }
                                }
                            }
                        }
                    }
                    if *open.read() {
                        SurfaceCardBody {
                            class: "px-4 py-3 border-t border-gray-100 bg-white/60 space-y-3",
                            FrameDetails {
                                kind: kind,
                                callstack: callstack.clone(),
                                ip: ip.map(|s| s.to_string()),
                            }
                        }
                    }
                }
            }
        }
    }
}

fn frame_icon(kind: FrameKind) -> Element {
    rsx! {
        match kind {
            FrameKind::Python => rsx! {
                Icon { icon: &icondata::SiPython, class: kind.icon_classes() }
            },
            FrameKind::Rust => rsx! {
                RustIcon { class: kind.icon_classes() }
            },
            FrameKind::Cpp => rsx! {
                Icon { icon: &icondata::SiCplusplus, class: kind.icon_classes() }
            },
        }
    }
}

fn frame_location_link(
    frame: &CallFrame,
    location: Option<&(String, i64)>,
) -> Option<(String, Option<i64>, String)> {
    match frame {
        CallFrame::PyFrame { file, lineno, .. } if !file.is_empty() => Some((
            file.clone(),
            Some(*lineno),
            format!("{}:{lineno}", file_display_name(file)),
        )),
        CallFrame::CFrame { file, .. } => location.map(|(path, line)| {
            let source_path = if file.is_empty() {
                path.clone()
            } else {
                file.clone()
            };
            (
                source_path,
                Some(*line),
                format!("{}:{line}", file_display_name(path)),
            )
        }),
        _ => None,
    }
}

#[component]
fn FrameDetails(kind: FrameKind, callstack: CallFrame, ip: Option<String>) -> Element {
    match callstack {
        CallFrame::PyFrame { file, lineno, locals, .. } => {
            rsx! {
                if !locals.is_empty() {
                    CompactLocals { locals: locals }
                }
                if !file.is_empty() {
                    SourceViewerCard {
                        path: file,
                        line: Some(lineno),
                        default_expanded: true,
                        collapsible: false,
                    }
                }
            }
        }
        CallFrame::CFrame { .. } => {
            rsx! {
                div { class: "space-y-2 text-sm",
                    if let Some(ip_addr) = ip {
                        div { class: "text-[11px] text-gray-400 font-mono px-2 py-1 rounded bg-gray-50 inline-block",
                            "ip {ip_addr}"
                        }
                    }
                    if kind == FrameKind::Rust {
                        div {
                            class: "text-xs text-orange-800 bg-orange-50 border border-orange-100 rounded-md px-2 py-1 inline-flex items-center gap-1",
                            Icon { icon: &icondata::AiInfoCircleOutlined, class: "w-3.5 h-3.5" }
                            "Demangled Rust symbol — no source file"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CompactLocals(locals: HashMap<String, Value>) -> Element {
    rsx! {
        div {
            class: "rounded-lg border border-gray-100 overflow-hidden",
            div { class: "px-3 py-1.5 bg-gray-50 border-b border-gray-100 text-[10px] font-semibold uppercase tracking-wide text-gray-500",
                "Locals ({locals.len()})"
            }
            div { class: "overflow-x-auto max-h-48",
                table { class: "min-w-full text-xs",
                    thead {
                        tr { class: "text-left text-gray-400 border-b border-gray-100",
                            th { class: "px-3 py-1.5 font-medium w-8", "#" }
                            th { class: "px-3 py-1.5 font-medium", "Name" }
                            th { class: "px-3 py-1.5 font-medium", "Value" }
                        }
                    }
                    tbody {
                        for (name, value) in locals {
                            tr { class: "border-b border-gray-50 last:border-0 hover:bg-gray-50/80",
                                td { class: "px-3 py-1.5 font-mono text-gray-400 tabular-nums", "{value.id}" }
                                td { class: "px-3 py-1.5 font-mono text-gray-800", "{name}" }
                                td { class: "px-3 py-1.5 text-gray-700 break-all font-mono",
                                    if let Some(val) = &value.value {
                                        "{val}"
                                    } else {
                                        span { class: "text-gray-400 italic", "None" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
