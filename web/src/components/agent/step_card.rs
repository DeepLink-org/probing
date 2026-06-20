//! Skill step results as in-app tool cards (tables, API text, navigation).

use dioxus::prelude::*;
use dioxus_router::Link;
use probing_proto::prelude::{DataFrame, Ele};

use crate::agent::StepOutcome;
use crate::components::agent::view_route::{agent_view_label, agent_view_to_route};
use crate::components::colors::colors;
use crate::components::dataframe_view::DataFrameView;
use crate::components::markdown_view::MarkdownView;
use crate::components::source_viewer::SourceRefChip;
use crate::components::stat_card::StatCard;
use crate::components::workspace::{
    AccentSurface, StatusBadge, SurfaceCardBody, SurfaceIconHeader,
};
use crate::state::agent::{AgentStepCardData, AgentStepKind, AgentStepStatus};
use crate::state::investigation::apply_context_from_dataframe_row;
use crate::utils::source_ref::{extract_source_refs, parse_files_api_path};

fn status_accent(status: &AgentStepStatus) -> &'static str {
    match status {
        AgentStepStatus::Ok => "border-l-emerald-500",
        AgentStepStatus::Warn => "border-l-amber-500",
        AgentStepStatus::Skipped => "border-l-slate-300",
        AgentStepStatus::Error => "border-l-red-500",
    }
}

fn status_badge(status: &AgentStepStatus) -> (&'static str, &'static str) {
    match status {
        AgentStepStatus::Ok => ("OK", "bg-emerald-50 text-emerald-700 border-emerald-200"),
        AgentStepStatus::Warn => ("WARN", "bg-amber-50 text-amber-800 border-amber-200"),
        AgentStepStatus::Skipped => ("SKIP", "bg-slate-100 text-slate-600 border-slate-200"),
        AgentStepStatus::Error => ("ERR", "bg-red-50 text-red-700 border-red-200"),
    }
}

fn kind_icon(kind: &AgentStepKind) -> &'static icondata::Icon {
    match kind {
        AgentStepKind::Sql => &icondata::AiDatabaseOutlined,
        AgentStepKind::Api => &icondata::AiApiOutlined,
        AgentStepKind::Navigate => &icondata::AiLinkOutlined,
    }
}

fn ele_display(ele: &Ele) -> String {
    match ele {
        Ele::Nil => "—".to_string(),
        Ele::BOOL(x) => x.to_string(),
        Ele::I32(x) => x.to_string(),
        Ele::I64(x) => x.to_string(),
        Ele::F32(x) => format!("{x:.4}"),
        Ele::F64(x) => format!("{x:.4}"),
        Ele::Text(x) => x.clone(),
        Ele::Url(x) => x.clone(),
        Ele::DataTime(x) => x.to_string(),
    }
}

fn sql_headline_stats(df: &DataFrame) -> Vec<(String, String)> {
    let nrows = df.cols.iter().map(|c| c.len()).max().unwrap_or(0);
    if nrows == 0 || nrows > 2 {
        return Vec::new();
    }
    let mut stats = Vec::new();
    for (name, col) in df.names.iter().zip(df.cols.iter()) {
        if stats.len() >= 4 {
            break;
        }
        let first = col.get(0);
        if matches!(first, Ele::Nil) {
            continue;
        }
        let val = ele_display(&first);
        if val == "—" || val.is_empty() {
            continue;
        }
        stats.push((name.clone(), val));
    }
    stats
}

pub fn step_outcome_to_card(outcome: StepOutcome) -> AgentStepCardData {
    match outcome {
        StepOutcome::Sql {
            step_id,
            title,
            dataframe,
            row_count,
            empty_message,
            cluster_note,
        } => {
            let status = if row_count > 0 {
                AgentStepStatus::Ok
            } else if empty_message.is_some() {
                AgentStepStatus::Warn
            } else {
                AgentStepStatus::Skipped
            };
            AgentStepCardData {
                step_id,
                title,
                kind: AgentStepKind::Sql,
                status,
                body_text: empty_message.unwrap_or_default(),
                dataframe: Some(dataframe),
                row_count: Some(row_count),
                navigate_view: None,
                api_path: None,
                cluster_note,
            }
        }
        StepOutcome::ApiText {
            step_id,
            title,
            text,
            path,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Api,
            status: AgentStepStatus::Ok,
            body_text: text,
            dataframe: None,
            row_count: None,
            navigate_view: None,
            api_path: path,
            cluster_note: None,
        },
        StepOutcome::UiNavigate {
            step_id,
            title,
            view,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Navigate,
            status: AgentStepStatus::Ok,
            body_text: String::new(),
            dataframe: None,
            row_count: None,
            navigate_view: Some(view),
            api_path: None,
            cluster_note: None,
        },
        StepOutcome::Skipped {
            step_id,
            title,
            reason,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Sql,
            status: AgentStepStatus::Skipped,
            body_text: reason,
            dataframe: None,
            row_count: None,
            navigate_view: None,
            api_path: None,
            cluster_note: None,
        },
        StepOutcome::Error {
            step_id,
            title,
            message,
        } => AgentStepCardData {
            step_id,
            title,
            kind: AgentStepKind::Sql,
            status: AgentStepStatus::Error,
            body_text: message,
            dataframe: None,
            row_count: None,
            navigate_view: None,
            api_path: None,
            cluster_note: None,
        },
    }
}

#[component]
pub fn AgentSkillRunCard(
    title: String,
    skill_id: String,
    category: String,
    docs: String,
) -> Element {
    let docs_preview: String = docs.lines().take(4).collect::<Vec<_>>().join("\n");
    rsx! {
        AccentSurface {
            accent: "border-l-blue-500",
            SurfaceIconHeader {
                icon: &icondata::AiRobotOutlined,
                icon_class: "w-4 h-4 text-blue-600",
                title: title,
                subtitle: Some(format!("{category} · {skill_id}")),
            }
            if !docs_preview.is_empty() {
                SurfaceCardBody {
                    class: "px-3 py-2",
                    MarkdownView {
                        content: docs_preview,
                        class: "text-xs text-gray-600".to_string(),
                    }
                }
            }
        }
    }
}

#[component]
pub fn AgentStepCard(step: AgentStepCardData) -> Element {
    let mut expanded = use_signal(|| {
        matches!(step.kind, AgentStepKind::Navigate)
            || step.row_count.unwrap_or(0) > 0
            || matches!(step.status, AgentStepStatus::Error)
    });

    let (badge_label, badge_cls) = status_badge(&step.status);
    let accent = status_accent(&step.status);
    let kind = step.kind.clone();
    let icon = kind_icon(&kind);

    let row_badge = step.row_count.map(|n| {
        if n == 0 {
            "0 rows".to_string()
        } else {
            format!("{n} rows")
        }
    });

    let headline_stats = step
        .dataframe
        .as_ref()
        .map(sql_headline_stats)
        .unwrap_or_default();

    let navigate_view = step.navigate_view.clone();
    let view_label = navigate_view
        .as_ref()
        .map(|v| agent_view_label(v))
        .unwrap_or_default();
    let nav_route = navigate_view.as_ref().map(|v| agent_view_to_route(v));

    let files_api_path = step.api_path.as_ref().and_then(|p| parse_files_api_path(p));
    let body_source_refs = extract_source_refs(&step.body_text);

    rsx! {
        AccentSurface {
            accent: accent,
            button {
                class: "w-full text-left px-3 py-2.5 bg-gray-50/80 border-b border-gray-100 hover:bg-gray-100/80 transition-colors",
                onclick: move |_| {
                    let cur = *expanded.read();
                    *expanded.write() = !cur;
                },
                div { class: "flex items-center gap-2 min-w-0",
                    crate::components::icon::Icon { icon, class: "w-4 h-4 text-gray-600 shrink-0" }
                    div { class: "flex-1 min-w-0",
                        div { class: "text-xs font-medium text-gray-900 truncate", title: "{step.title}",
                            "{step.title}"
                        }
                        div { class: "text-[10px] text-gray-400 font-mono truncate", "{step.step_id}" }
                    }
                    div { class: "flex items-center gap-1 shrink-0",
                        StatusBadge { label: badge_label, badge_class: badge_cls }
                        if let Some(ref cn) = step.cluster_note {
                            span {
                                class: "text-[10px] px-1.5 py-0.5 rounded bg-violet-50 text-violet-700 border border-violet-100 max-w-[9rem] truncate",
                                title: "{cn}",
                                "{cn}"
                            }
                        }
                        if let Some(ref rb) = row_badge {
                            span { class: "text-[10px] text-gray-400", "{rb}" }
                        }
                        div {
                            class: "transition-transform duration-200",
                            class: if *expanded.read() { "rotate-180" } else { "rotate-0" },
                            svg {
                                class: "w-3.5 h-3.5 text-gray-400",
                                fill: "none",
                                stroke: "currentColor",
                                view_box: "0 0 24 24",
                                path {
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                    stroke_width: "2",
                                    d: "M19 9l-7 7-7-7"
                                }
                            }
                        }
                    }
                }
            }

            if *expanded.read() {
                SurfaceCardBody {
                    class: "px-3 py-2 space-y-2",
                    if !headline_stats.is_empty() {
                        div { class: "grid grid-cols-2 gap-2",
                            for (label, value) in headline_stats {
                                StatCard {
                                    label: label.clone(),
                                    value: value.clone(),
                                }
                            }
                        }
                    }

                    if let Some(ref view) = navigate_view {
                        if let Some(route) = nav_route.clone() {
                            div { class: "flex flex-col gap-2",
                                p { class: "text-xs text-gray-600",
                                    "Open the full view in the main workspace to inspect details."
                                }
                                Link {
                                    to: route,
                                    class: format!(
                                        "inline-flex items-center justify-center gap-2 px-3 py-2 text-sm font-medium rounded-lg bg-{} text-white hover:opacity-90 transition-opacity",
                                        colors::PRIMARY
                                    ),
                                    crate::components::icon::Icon { icon: &icondata::AiExportOutlined, class: "w-4 h-4" }
                                    span { "Open {view_label}" }
                                    span { class: "text-white/70 font-mono text-xs", "{view}" }
                                }
                            }
                        }
                    } else if let Some(ref df) = step.dataframe {
                        if step.row_count.unwrap_or(0) > 0 {
                            {
                                let df_for_click = df.clone();
                                rsx! {
                                    p { class: "text-[10px] text-gray-500 mb-1",
                                        "Click a row to set investigation context (tid / trace_id / span columns)."
                                    }
                                    div { class: "max-h-56 overflow-auto rounded-md border border-gray-100 text-xs",
                                        DataFrameView {
                                            df: df.clone(),
                                            on_row_click: EventHandler::new(move |row: usize| {
                                                apply_context_from_dataframe_row(&df_for_click, row);
                                            }),
                                        }
                                    }
                                }
                            }
                        } else if !step.body_text.is_empty() {
                            MarkdownView {
                                content: step.body_text.clone(),
                                class: "text-xs text-amber-800".to_string(),
                            }
                        }
                    } else if !step.body_text.is_empty() {
                        if let Some(ref file_path) = files_api_path {
                            div { class: "flex flex-wrap gap-1.5",
                                SourceRefChip {
                                    path: file_path.clone(),
                                    line: None,
                                }
                            }
                        } else if matches!(step.kind, AgentStepKind::Api) {
                            pre { class: "text-[11px] font-mono text-gray-800 bg-gray-50 rounded-md p-2 overflow-x-auto whitespace-pre-wrap max-h-48 border border-gray-100",
                                "{step.body_text}"
                            }
                        } else {
                            MarkdownView {
                                content: step.body_text.clone(),
                                class: "text-xs".to_string(),
                            }
                        }
                    }

                    if !body_source_refs.is_empty() {
                        div { class: "flex flex-wrap gap-1.5 pt-1",
                            for (i, reference) in body_source_refs.iter().enumerate() {
                                SourceRefChip {
                                    key: "{i}",
                                    path: reference.path.clone(),
                                    line: reference.line.map(i64::from),
                                }
                            }
                        }
                    }

                    if let Some(ref path) = step.api_path {
                        if files_api_path.is_none() {
                            p { class: "text-[10px] text-gray-400 font-mono truncate", "GET {path}" }
                        }
                    }
                }
            }
        }
    }
}
