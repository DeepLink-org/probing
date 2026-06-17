use dioxus::prelude::*;
use probing_proto::prelude::CallFrame;

use crate::components::collapsible_card::CollapsibleCardWithIcon;
use crate::components::icon::{Icon, RustIcon};
use crate::components::value_list::ValueList;
use crate::utils::callframe::{classify_frame, frame_ip, frame_location, frame_title, FrameKind};

#[component]
pub fn CallStackView(callstack: CallFrame, index: usize, is_last: bool) -> Element {
    let kind = classify_frame(&callstack);
    let title = frame_title(&callstack);
    let location = frame_location(&callstack);
    let ip = frame_ip(&callstack);

    let icon = rsx! {
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
    };

    let connector = if is_last {
        "hidden"
    } else {
        "absolute left-[1.125rem] top-9 bottom-0 w-px bg-gray-200"
    };

    rsx! {
        div { class: "relative flex gap-3",
            div { class: "flex flex-col items-center shrink-0 w-9",
                span {
                    class: "inline-flex items-center justify-center w-7 h-7 rounded-full bg-gray-100 text-[11px] font-semibold text-gray-500 tabular-nums",
                    "{index}"
                }
                div { class: "{connector}" }
            }
            div { class: "flex-1 min-w-0 pb-3",
                CollapsibleCardWithIcon {
                    title: title.clone(),
                    badge: Some(kind.label().to_string()),
                    badge_classes: Some(kind.badge_classes().to_string()),
                    accent_border: Some(kind.accent_border().to_string()),
                    default_open: index == 0,
                    icon: icon,
                    FrameDetails {
                        kind: kind,
                        callstack: callstack.clone(),
                        location: location,
                        ip: ip.map(|s| s.to_string()),
                    }
                }
            }
        }
    }
}

#[component]
fn FrameDetails(
    kind: FrameKind,
    callstack: CallFrame,
    location: Option<(String, i64)>,
    ip: Option<String>,
) -> Element {
    match callstack {
        CallFrame::PyFrame { file, func, lineno, locals } => {
            let url = crate::utils::base_path::with_base(&format!("/apis/files?path={file}"));
            rsx! {
                div { class: "space-y-3",
                    div { class: "text-sm text-gray-600",
                        span { class: "font-medium text-gray-700", "at " }
                        span { class: "font-mono", "{func}" }
                        span { " @ " }
                        a {
                            href: "{url}",
                            target: "_blank",
                            class: "font-mono text-blue-600 hover:underline",
                            "{file}:{lineno}"
                        }
                    }
                    if !locals.is_empty() {
                        div {
                            h4 { class: "text-xs font-semibold uppercase tracking-wide text-gray-500 mb-2", "Locals" }
                            ValueList { variables: locals }
                        }
                    }
                }
            }
        }
        CallFrame::CFrame { file, .. } => {
            let file_url = (!file.is_empty()).then(|| {
                crate::utils::base_path::with_base(&format!("/apis/files?path={file}"))
            });
            rsx! {
                div { class: "space-y-2 text-sm",
                    if let Some((path, line)) = location {
                        if let Some(url) = file_url {
                            div { class: "text-gray-600",
                                span { class: "font-medium text-gray-700", "source " }
                                a {
                                    href: "{url}",
                                    target: "_blank",
                                    class: "font-mono text-blue-600 hover:underline break-all",
                                    "{path}:{line}"
                                }
                            }
                        } else {
                            div { class: "text-gray-600 font-mono break-all", "{path}:{line}" }
                        }
                    }
                    if let Some(ip_addr) = ip {
                        div { class: "text-xs text-gray-400 font-mono", "ip {ip_addr}" }
                    }
                    if kind == FrameKind::Rust {
                        div {
                            class: "text-xs text-orange-700/80 bg-orange-50/60 border border-orange-100 rounded px-2 py-1 inline-block",
                            "Demangled Rust symbol"
                        }
                    }
                }
            }
        }
    }
}
