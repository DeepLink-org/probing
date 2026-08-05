use dioxus::prelude::*;
use dioxus_router::{use_route, Outlet};

use crate::api::ApiClient;
use crate::components::agent::LlmSettingsOverlay;
use crate::components::global_command_panel::GlobalCommandPanel;
use crate::components::icon::Icon;
use crate::components::keyboard_shortcuts::{GlobalShortcutInstaller, ShortcutsHelpOverlay};
use crate::components::ui_task_runtime::UiTaskRuntime;
use crate::state::agent::{
    load_agent_panel_width, save_agent_panel_width, AgentPanelWidth, AGENT_PANEL_OPEN,
    AGENT_PANEL_WIDTH,
};
use crate::state::commands::COMMAND_PANEL_OPEN;
use crate::state::investigation::investigation_context_key;
use crate::state::investigation::{InvestigationContext, INVESTIGATION_CONTEXT};
use crate::state::investigation_url::InvestigationUrlSync;
use crate::state::llm_config::load_llm_config;
use crate::state::llm_config::LLM_SETTINGS_OPEN;
use crate::state::page_context::{apply_page_descriptor, PageContextDescriptor, PAGE_CONTEXT};
use crate::state::training::TRAINING_CLUSTER_SCOPE;

use super::capabilities::CapabilityCatalogPoller;
use super::page_snapshot::refresh_next_page_snapshot;
use super::pages::InvestigateSession;
use super::routes::NextRoute;
use super::settings::{
    load_next_shell_settings, save_next_sidebar_compact, MEMORY_CLUSTER_SCOPE,
    MEMORY_WINDOW_MINUTES, NEXT_SIDEBAR_COMPACT,
};
use super::sidebar::NextSidebar;

fn sidebar_width_class(compact: bool) -> &'static str {
    if compact {
        "w-14"
    } else {
        "w-72"
    }
}

#[component]
pub fn NextShell() -> Element {
    let route = use_route::<NextRoute>();
    let mut last_evidence_key = use_signal(String::new);

    use_effect(move || {
        load_llm_config();
        load_agent_panel_width();
        load_next_shell_settings();
        spawn(async move {
            let _ = ApiClient::new().load_skill_store().await;
        });
    });

    let route_for_context = route.clone();
    let investigation = INVESTIGATION_CONTEXT.read().clone();
    let evidence_key = evidence_refresh_key(
        &route_for_context,
        &investigation,
        *TRAINING_CLUSTER_SCOPE.read(),
        *MEMORY_CLUSTER_SCOPE.read(),
        *MEMORY_WINDOW_MINUTES.read(),
    );
    use_effect(move || {
        if *last_evidence_key.read() == evidence_key {
            return;
        }
        *last_evidence_key.write() = evidence_key.clone();
        let descriptor = route_for_context.page_spec();
        apply_page_descriptor(PageContextDescriptor {
            page_id: descriptor.id.to_string(),
            title: descriptor.title.to_string(),
            path: descriptor.canonical_path.to_string(),
            description: descriptor.description.to_string(),
            suggested_skills: descriptor
                .skills
                .iter()
                .map(|skill| (*skill).to_string())
                .collect(),
            investigation_summary: investigation.summary(),
            investigation_coordinates: investigation.coordinates_summary(),
            investigation_key: investigation_context_key(&investigation),
        });
    });

    let sidebar_compact = *NEXT_SIDEBAR_COMPACT.read();
    let sidebar_width = sidebar_width_class(sidebar_compact);
    let workspace = route.page_spec().workspace;
    let investigation_route_key = format!("{route:?}");

    rsx! {
        GlobalShortcutInstaller {}
        UiTaskRuntime {}
        CapabilityCatalogPoller {}
        InvestigationUrlSync { route_key: investigation_route_key }
        if *COMMAND_PANEL_OPEN.read() {
            GlobalCommandPanel {}
        }
        ShortcutsHelpOverlay {}
        LlmSettingsOverlay {}
        div { class: "flex h-screen overflow-hidden bg-gray-50 text-gray-950",
            a {
                href: "#main-content",
                class: "sr-only fixed left-3 top-3 z-[100] rounded-md bg-white px-3 py-2 text-sm font-semibold text-blue-800 shadow-lg ring-2 ring-blue-600 focus:not-sr-only",
                "Skip to main content"
            }
            aside {
                class: "flex {sidebar_width} shrink-0 flex-col border-r border-slate-800 bg-slate-950 text-slate-100 transition-[width] duration-150",
                NextSidebar {
                    route: route.clone(),
                    compact: sidebar_compact,
                    on_toggle_compact: move |_| {
                        let compact = *NEXT_SIDEBAR_COMPACT.read();
                        save_next_sidebar_compact(!compact);
                    },
                }
            }

            div { class: "flex min-w-0 flex-1 flex-col",
                div { class: "relative min-h-0 flex-1 overflow-hidden",
                    main { id: "main-content", tabindex: "-1", class: "{workspace.main_class()}",
                        div { class: "{workspace.content_class()}",
                            Outlet::<NextRoute> {}
                        }
                    }
                    NextAgentPanel {
                        hidden: matches!(route, NextRoute::Investigate {}),
                        route: route.clone(),
                    }
                }
            }
        }
    }
}

fn evidence_refresh_key(
    route: &NextRoute,
    investigation: &InvestigationContext,
    training_cluster: bool,
    memory_cluster: bool,
    memory_window: usize,
) -> String {
    format!(
        "{route:?}|{}|training_cluster={training_cluster}|memory_cluster={memory_cluster}|memory_window={memory_window}",
        investigation_context_key(investigation),
    )
}

#[component]
fn NextAgentPanel(hidden: bool, route: NextRoute) -> Element {
    let mut last_fallback_key = use_signal(String::new);
    let panel_open = *AGENT_PANEL_OPEN.read();
    let page = PAGE_CONTEXT.read().clone();
    let fallback_key = format!("{}|{}", page.page_id, page.investigation_key);
    let fallback_route = route.clone();
    use_effect(move || {
        if hidden
            || !panel_open
            || !page.snapshot.is_empty()
            || fallback_route.page_spec().publishes_evidence
        {
            return;
        }
        if *last_fallback_key.read() == fallback_key {
            return;
        }
        *last_fallback_key.write() = fallback_key.clone();
        let route = fallback_route.clone();
        spawn(async move {
            refresh_next_page_snapshot(route).await;
        });
    });

    if hidden || !panel_open {
        return rsx! {};
    }

    let panel_width = *AGENT_PANEL_WIDTH.read();
    let width_class = panel_width.floating_class();
    rsx! {
        div { class: "absolute inset-0 z-40 flex pointer-events-none",
            div {
                class: "flex-1 bg-black/20 pointer-events-auto",
                onclick: move |_| *AGENT_PANEL_OPEN.write() = false,
            }
            aside { class: "flex h-full {width_class} min-w-[22rem] shrink-0 flex-col bg-gray-50 shadow-2xl pointer-events-auto",
                div { class: "flex shrink-0 items-center justify-between border-b border-gray-200 bg-white px-4 py-3",
                    div {
                        div { class: "text-sm font-semibold text-gray-950", "Investigate" }
                        div { class: "text-xs text-gray-500", "Skill-driven diagnostic workspace" }
                    }
                    div { class: "flex items-center gap-1",
                        div { class: "mr-1 inline-flex items-center rounded-md border border-gray-200 bg-gray-100 p-0.5",
                            button {
                                r#type: "button",
                                class: if panel_width == AgentPanelWidth::Third {
                                    "rounded bg-white px-2 py-1 text-xs font-medium text-gray-900 shadow-sm"
                                } else {
                                    "rounded px-2 py-1 text-xs text-gray-500 hover:text-gray-800"
                                },
                                aria_label: "Use one-third Investigate width",
                                onclick: move |_| save_agent_panel_width(AgentPanelWidth::Third),
                                "⅓"
                            }
                            button {
                                r#type: "button",
                                class: if panel_width == AgentPanelWidth::TwoThirds {
                                    "rounded bg-white px-2 py-1 text-xs font-medium text-gray-900 shadow-sm"
                                } else {
                                    "rounded px-2 py-1 text-xs text-gray-500 hover:text-gray-800"
                                },
                                aria_label: "Use two-thirds Investigate width",
                                onclick: move |_| save_agent_panel_width(AgentPanelWidth::TwoThirds),
                                "⅔"
                            }
                        }
                        button {
                            r#type: "button",
                            class: "rounded-md p-2 text-gray-500 hover:bg-gray-100 hover:text-gray-800",
                            aria_label: "Open LLM settings",
                            onclick: move |_| *LLM_SETTINGS_OPEN.write() = true,
                            Icon { icon: &icondata::AiSettingOutlined, class: "h-4 w-4" }
                        }
                        button {
                            r#type: "button",
                            class: "rounded-md p-2 text-gray-500 hover:bg-gray-100 hover:text-gray-800",
                            aria_label: "Close Investigate panel",
                            onclick: move |_| *AGENT_PANEL_OPEN.write() = false,
                            Icon { icon: &icondata::AiCloseOutlined, class: "h-4 w-4" }
                        }
                    }
                }
                div { class: "flex min-h-0 flex-1 flex-col p-3",
                    NextPageEvidence { route }
                    div { class: "min-h-0 flex-1",
                        InvestigateSession { compact: true }
                    }
                }
            }
        }
    }
}

#[component]
fn NextPageEvidence(route: NextRoute) -> Element {
    let page = PAGE_CONTEXT.read().clone();
    let page_owned = route.page_spec().publishes_evidence;
    rsx! {
        div { class: "mb-3 rounded-xl border border-blue-200 bg-blue-50 px-3 py-2 text-xs text-blue-950",
            div { class: "flex items-start justify-between gap-2",
                div { class: "min-w-0",
                    div { class: "font-semibold", "Viewing · {page.title}" }
                    div { class: "truncate font-mono text-xs text-blue-700", "{page.path}" }
                }
                if page_owned {
                    span { class: "shrink-0 rounded-md border border-blue-200 bg-white px-2 py-1 text-xs font-medium text-blue-700",
                        if page.snapshot_loading { "Collecting…" } else { "Page evidence" }
                    }
                } else {
                    button {
                        r#type: "button",
                        class: "shrink-0 rounded-md border border-blue-200 bg-white px-2 py-1 text-xs font-medium text-blue-700 hover:bg-blue-100",
                        disabled: page.snapshot_loading,
                        onclick: move |_| {
                            let route = route.clone();
                            spawn(async move {
                                refresh_next_page_snapshot(route).await;
                            });
                        },
                        if page.snapshot_loading { "Loading…" } else { "Refresh evidence" }
                    }
                }
            }
            if !page.snapshot.is_empty() {
                pre { class: "mt-2 max-h-24 overflow-auto whitespace-pre-wrap rounded-md bg-white/70 p-2 font-mono text-xs text-blue-900",
                    "{page.snapshot}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::next::page_registry::WorkspaceKind;

    #[test]
    fn compact_sidebar_reclaims_detail_panel_width() {
        assert_eq!(sidebar_width_class(false), "w-72");
        assert_eq!(sidebar_width_class(true), "w-14");
    }

    #[test]
    fn visualization_routes_receive_full_height_workspace() {
        assert_eq!(
            NextRoute::ProfileView {
                view: "trace".to_string(),
            }
            .page_spec()
            .workspace,
            WorkspaceKind::FullHeight
        );
        assert_eq!(
            NextRoute::Perfetto {}.page_spec().workspace,
            WorkspaceKind::FullHeight
        );
        assert_eq!(
            NextRoute::Training {}.page_spec().workspace,
            WorkspaceKind::Standard
        );
    }

    #[test]
    fn evidence_refresh_key_changes_only_for_material_request_inputs() {
        let context = InvestigationContext {
            rank: Some(58),
            host: Some("node-07".into()),
            ..Default::default()
        };
        let local = evidence_refresh_key(&NextRoute::Training {}, &context, false, false, 5);
        let same = evidence_refresh_key(&NextRoute::Training {}, &context, false, false, 5);
        let cluster = evidence_refresh_key(&NextRoute::Training {}, &context, true, false, 5);

        assert_eq!(local, same);
        assert_ne!(local, cluster);
    }

    #[test]
    fn page_owned_evidence_avoids_agent_side_duplicate_queries() {
        assert!(NextRoute::Dashboard {}.page_spec().publishes_evidence);
        assert!(NextRoute::Training {}.page_spec().publishes_evidence);
        assert!(NextRoute::Memory {}.page_spec().publishes_evidence);
        assert!(!NextRoute::System {}.page_spec().publishes_evidence);
    }
}
