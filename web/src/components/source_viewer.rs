//! Read-only source preview: floating overlay + clickable location links.

use dioxus::prelude::*;

use crate::agent::{
    ask_agent_about_source, ask_and_run_agent_about_source, run_skill_with_source,
    suggested_skills_for_source,
};
use crate::api::ApiClient;
use crate::components::icon::Icon;
use crate::components::workspace::{AccentSurface, ChipButton, SurfaceCardBody, SurfaceIconHeader};
use crate::hooks::use_app_resource;
use crate::state::source_viewer::{close_source_viewer, open_source_viewer, source_viewer_target};
use crate::state::ui_tasks::ui_agent_busy;
use crate::utils::source_ref::{
    file_display_name, language_class, slice_source, SourceSlice, DEFAULT_SOURCE_CONTEXT,
};

/// Centered floating dialog for source preview (mounted via [`crate::components::app_overlays::AppOverlays`]).
#[component]
pub fn SourceViewerOverlay() -> Element {
    let Some(target) = source_viewer_target() else {
        return rsx! {};
    };

    let aria_label = match target.line {
        Some(l) => format!("{}:{l}", file_display_name(&target.path)),
        None => file_display_name(&target.path),
    };

    rsx! {
        div {
            class: "source-viewer-overlay fixed inset-0 z-[9999] flex items-center justify-center p-4 sm:p-8 bg-black/50",
            onclick: move |_| close_source_viewer(),
            div {
                class: "source-viewer-dialog relative w-full max-w-3xl max-h-[80vh] overflow-hidden",
                role: "dialog",
                aria_modal: "true",
                aria_label: "{aria_label}",
                onclick: move |e| e.stop_propagation(),
                button {
                    r#type: "button",
                    class: "absolute top-3 right-3 z-10 p-1.5 rounded-md bg-white/90 border border-gray-200 text-gray-500 hover:bg-gray-100 hover:text-gray-800 shadow-sm",
                    title: "Close (Esc)",
                    aria_label: "Close source preview",
                    onclick: move |e| {
                        e.stop_propagation();
                        close_source_viewer();
                    },
                    Icon { icon: &icondata::AiCloseOutlined, class: "w-4 h-4" }
                }
                SourceViewerCard {
                    key: "{target.path}:{target.line:?}",
                    path: target.path.clone(),
                    line: target.line,
                    default_expanded: true,
                    collapsible: false,
                    floating: true,
                }
            }
        }
    }
}

/// Clickable `path:line` — opens overlay by default, or runs `on_activate` (e.g. expand stack frame).
#[component]
pub fn SourceLocationLink(
    path: String,
    #[props(optional)] line: Option<i64>,
    #[props(optional)] label: Option<String>,
    #[props(optional, default = String::new())] class: String,
    #[props(optional, default = true)] use_overlay: bool,
    #[props(optional)] on_activate: Option<EventHandler<()>>,
) -> Element {
    let text = label.unwrap_or_else(|| match line {
        Some(l) => format!("{path}:{l}"),
        None => path.clone(),
    });

    rsx! {
        button {
            r#type: "button",
            class: "font-mono text-blue-600 hover:text-blue-800 hover:underline text-left break-all {class}",
            title: if use_overlay { "Preview source" } else { "Show source in frame" },
            onclick: move |e| {
                e.stop_propagation();
                if use_overlay {
                    open_source_viewer(path.clone(), line);
                } else if let Some(handler) = on_activate {
                    handler.call(());
                }
            },
            "{text}"
        }
    }
}

/// Compact chip for Investigate messages and step cards.
#[component]
pub fn SourceRefChip(path: String, #[props(optional)] line: Option<i64>) -> Element {
    let label = match line {
        Some(l) => format!("{}:{l}", file_display_name(&path)),
        None => file_display_name(&path),
    };

    rsx! {
        button {
            r#type: "button",
            class: "inline-flex items-center gap-1 px-2 py-1 text-[11px] font-mono rounded-md border border-gray-200 bg-white text-blue-700 hover:bg-blue-50 hover:border-blue-200 transition-colors",
            title: "{path}",
            onclick: move |e| {
                e.stop_propagation();
                open_source_viewer(path.clone(), line);
            },
            Icon { icon: &icondata::AiFileTextOutlined, class: "w-3 h-3 shrink-0" }
            span { class: "truncate max-w-[14rem]", "{label}" }
        }
    }
}

/// Inline card, or floating overlay when `floating` is set.
#[component]
pub fn SourceViewerCard(
    path: String,
    #[props(optional)] line: Option<i64>,
    #[props(optional, default = DEFAULT_SOURCE_CONTEXT)] context_lines: usize,
    #[props(optional, default = true)] default_expanded: bool,
    #[props(optional, default = false)] collapsible: bool,
    #[props(optional, default = false)] floating: bool,
) -> Element {
    let highlight = line.and_then(|l| u32::try_from(l).ok());
    let mut expanded = use_signal(|| default_expanded);
    let display_name = file_display_name(&path);
    let subtitle = line_label(highlight);
    let accent = if highlight.is_some() {
        "border-l-amber-500"
    } else {
        "border-l-slate-300"
    };

    let card = if collapsible {
        rsx! {
            AccentSurface {
                accent: accent,
                button {
                    class: "w-full text-left",
                    onclick: move |_| {
                        let cur = *expanded.read();
                        *expanded.write() = !cur;
                    },
                    SurfaceIconHeader {
                        icon: &icondata::AiFileTextOutlined,
                        icon_class: "w-4 h-4 text-gray-600",
                        title: display_name.clone(),
                        subtitle: Some(format!("{path}{subtitle}")),
                    }
                }
                if *expanded.read() {
                    SourceViewerBody {
                        path: path.clone(),
                        line,
                        context_lines,
                        floating,
                    }
                }
            }
        }
    } else {
        rsx! {
            AccentSurface {
                accent: accent,
                SurfaceIconHeader {
                    icon: &icondata::AiFileTextOutlined,
                    icon_class: "w-4 h-4 text-gray-600",
                    title: display_name.clone(),
                    subtitle: Some(format!("{path}{subtitle}")),
                }
                SourceViewerBody { path, line, context_lines, floating }
            }
        }
    };

    if floating {
        rsx! {
            div {
                class: "shadow-[0_20px_60px_-12px_rgba(0,0,0,0.35)] rounded-xl overflow-hidden",
                {card}
            }
        }
    } else {
        card
    }
}

#[component]
fn SourceViewerBody(
    path: String,
    #[props(optional)] line: Option<i64>,
    #[props(optional, default = DEFAULT_SOURCE_CONTEXT)] context_lines: usize,
    #[props(optional, default = false)] floating: bool,
) -> Element {
    let highlight = line.and_then(|l| u32::try_from(l).ok());
    let fetch_path = path.clone();
    let content = use_app_resource(move || {
        let p = fetch_path.clone();
        async move { ApiClient::new().read_file(&p).await }
    });

    let loaded = content.read().clone();

    match loaded.as_ref() {
        None => {
            let loading_class = if floating {
                "flex items-center justify-center px-4 py-8 text-sm text-gray-500"
            } else {
                "px-4 py-6 text-sm text-gray-500 text-center"
            };
            rsx! {
                SurfaceCardBody {
                    class: loading_class,
                    "Loading source…"
                }
            }
        }
        Some(Err(e)) => rsx! {
            SurfaceCardBody {
                class: "px-4 py-3 text-sm text-red-700 bg-red-50",
                "Could not read file: {e.display_message()}"
            }
        },
        Some(Ok(text)) => {
            let slice = slice_source(text, highlight, context_lines);
            rsx! {
                SourceSliceView {
                    slice,
                    path: path.clone(),
                    line,
                    floating,
                }
            }
        }
    }
}

#[component]
fn SourceSliceView(slice: SourceSlice, path: String, line: Option<i64>, floating: bool) -> Element {
    let range_note = slice_range_label(&slice);
    let raw_url = crate::utils::base_path::with_base(&format!(
        "/apis/files?path={}",
        urlencoding::encode(&path)
    ));
    let lang_label = language_class(&path);
    let body_class = if floating {
        "max-h-[min(60vh,520px)] overflow-auto rounded-md border border-gray-100 bg-white"
    } else {
        "max-h-96 overflow-auto rounded-md border border-gray-100 bg-white"
    };

    rsx! {
        SurfaceCardBody {
            class: "px-0 py-0 space-y-0",
            SourceSliceHeader {
                highlight: slice.highlight_line,
                range_note: range_note,
                lang_label: lang_label.to_string(),
                raw_url: raw_url,
            }
            SourceAgentBar {
                path: path.clone(),
                line: line,
                slice: slice.clone(),
            }
            div { class: body_class,
                SourceSliceBody { slice, path }
            }
        }
    }
}

#[component]
fn SourceAgentBar(path: String, line: Option<i64>, slice: SourceSlice) -> Element {
    let busy = ui_agent_busy();
    let skills = suggested_skills_for_source();
    let path_ask = path.clone();
    let path_run = path.clone();
    let slice_ask = slice.clone();
    let slice_run = slice.clone();

    rsx! {
        div {
            class: "flex flex-wrap items-center gap-1.5 px-3 py-2 border-b border-violet-100 bg-violet-50/40",
            Icon { icon: &icondata::AiRobotOutlined, class: "w-3.5 h-3.5 text-violet-600 shrink-0" }
            span { class: "text-[10px] font-medium text-violet-800 shrink-0", "Investigate" }
            button {
                r#type: "button",
                class: "inline-flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md border border-violet-200 bg-white text-violet-800 hover:bg-violet-50 disabled:opacity-50",
                disabled: busy,
                title: "Open Investigate with a question about this code",
                onclick: move |e| {
                    e.stop_propagation();
                    ask_agent_about_source(&path_ask, line, &slice_ask);
                },
                "Ask Agent"
            }
            button {
                r#type: "button",
                class: "inline-flex items-center gap-1 px-2 py-1 text-[10px] font-medium rounded-md bg-violet-600 text-white hover:bg-violet-700 disabled:opacity-50",
                disabled: busy,
                title: "Run Investigate immediately with source context",
                onclick: move |e| {
                    e.stop_propagation();
                    ask_and_run_agent_about_source(&path_run, line, &slice_run);
                },
                "Ask & Run"
            }
            for pb in skills {
                {
                    let path_pb = path.clone();
                    let slice_pb = slice.clone();
                    let id = pb.clone();
                    rsx! {
                        ChipButton {
                            label: pb,
                            disabled: busy,
                            onclick: move |_| {
                                run_skill_with_source(&id, &path_pb, line, &slice_pb);
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SourceSliceHeader(
    highlight: Option<u32>,
    range_note: String,
    lang_label: String,
    raw_url: String,
) -> Element {
    rsx! {
        div { class: "flex items-center justify-between gap-2 px-3 py-1.5 border-b border-gray-100 bg-gray-50/80",
            div { class: "flex items-center gap-2 min-w-0 text-[10px] text-gray-500",
                if let Some(ln) = highlight {
                    span {
                        class: "inline-flex items-center px-1.5 py-0.5 rounded bg-amber-100 text-amber-900 font-semibold tabular-nums",
                        "▶ line {ln}"
                    }
                }
                if !range_note.is_empty() {
                    span { class: "truncate", "{range_note}" }
                }
                span { class: "text-gray-400 uppercase tracking-wide", "{lang_label}" }
            }
            a {
                href: "{raw_url}",
                target: "_blank",
                class: "shrink-0 text-[10px] text-blue-600 hover:underline",
                "Open raw"
            }
        }
    }
}

#[component]
fn SourceSliceBody(slice: SourceSlice, path: String) -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(lang) = crate::utils::source_ref::highlight_language(&path) {
        return rsx! {
            HighlightedSourceBlock { slice, language: lang }
        };
    }

    rsx! {
        PlainSourceLines { slice }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[component]
fn HighlightedSourceBlock(slice: SourceSlice, language: dioxus_code::Language) -> Element {
    use dioxus_code::{Code, CodeTheme, SourceCode, Theme};

    const CODE_THEME: CodeTheme = CodeTheme::fixed(Theme::GITHUB_LIGHT);

    rsx! {
        div { class: "px-2 py-2 source-viewer-code text-[11px]",
            if slice.text.is_empty() {
                p { class: "text-xs text-gray-500 px-2", "Empty file" }
            } else {
                Code {
                    src: SourceCode::new(language, slice.text.clone()),
                    theme: CODE_THEME,
                }
            }
        }
    }
}

#[component]
fn PlainSourceLines(slice: SourceSlice) -> Element {
    let highlight = slice.highlight_line;
    let rows: Vec<(usize, String)> = slice
        .text
        .lines()
        .enumerate()
        .map(|(i, line)| (slice.start_line + i, line.to_string()))
        .collect();

    rsx! {
        div { class: "font-mono text-[11px] leading-5",
            if rows.is_empty() {
                p { class: "text-xs text-gray-500 px-3 py-2", "Empty file" }
            } else {
                for (line_no, line_text) in rows {
                    {
                        let is_hl = highlight == Some(line_no as u32);
                        rsx! {
                            div {
                                class: format!(
                                    "flex min-w-0 {}",
                                    if is_hl {
                                        "bg-amber-50 border-l-2 border-amber-400"
                                    } else {
                                        "border-l-2 border-transparent"
                                    }
                                ),
                                span {
                                    class: format!(
                                        "shrink-0 w-10 px-2 text-right select-none tabular-nums {}",
                                        if is_hl { "text-amber-800 font-semibold" } else { "text-gray-400" }
                                    ),
                                    "{line_no}"
                                }
                                span {
                                    class: "flex-1 px-2 py-px text-gray-800 whitespace-pre overflow-x-auto",
                                    "{line_text}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn slice_range_label(slice: &SourceSlice) -> String {
    if slice.total_lines == 0 {
        String::new()
    } else if slice.start_line == 1 && slice.end_line == slice.total_lines {
        format!("{} lines", slice.total_lines)
    } else {
        format!(
            "lines {}–{} of {}",
            slice.start_line, slice.end_line, slice.total_lines
        )
    }
}

fn line_label(line: Option<u32>) -> String {
    line.map(|l| format!(":{l}")).unwrap_or_default()
}
