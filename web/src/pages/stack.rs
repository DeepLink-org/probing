use dioxus::prelude::*;
use probing_proto::prelude::CallFrame;

use crate::api::ApiClient;
use crate::components::callstack_view::CallStackView;
use crate::components::common::{AsyncBoundary, EmptyState, ErrorState};
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_app_resource;
use crate::state::stack::{
    stack_tid_label, StackSnapshot, STACK_MODE, STACK_REFRESH, STACK_SNAPSHOT,
};
use crate::utils::callframe::{count_by_kind, matches_mode};
use crate::utils::error::AppError;

#[component]
pub fn Stack(tid: Option<String>) -> Element {
    let tid_for_api = tid.clone();
    let tid_label = stack_tid_label(tid.as_deref());
    let refresh_tick = STACK_REFRESH();

    rsx! {
        PageContainer {
            PageTitle {
                title: "Stacks".to_string(),
                subtitle: if tid.is_some() {
                    Some(format!("Thread {tid_label}"))
                } else {
                    None
                },
                icon: Some(&icondata::AiApartmentOutlined),
            }

            AsyncBoundary {
                message: Some("Loading call stack…".to_string()),
                StackLoaded {
                    tid: tid_for_api,
                    tid_label: tid_label,
                    refresh_tick: refresh_tick,
                }
            }
        }
    }
}

#[component]
fn StackLoaded(tid: Option<String>, tid_label: String, refresh_tick: u32) -> Element {
    let mode = STACK_MODE();
    let filter_mode = mode.clone();
    let stack = use_app_resource(move || {
        let _ = refresh_tick;
        let tid_arg = tid.clone();
        async move {
            ApiClient::new()
                .get_callstack_with_mode(tid_arg, "mixed")
                .await
        }
    });

    let stack_peek = stack.read().clone();
    let tid_for_effect = tid_label.clone();

    use_effect(use_reactive!(|(
        mode,
        refresh_tick,
        stack_peek,
        tid_for_effect,
    )| {
        let _ = refresh_tick;
        let Some(result) = stack_peek.as_ref() else {
            return;
        };
        *STACK_SNAPSHOT.write() = stack_snapshot_for(&tid_for_effect, result, &mode);
    }));

    match stack.suspend()?().as_ref() {
        Err(err) => rsx! {
            ErrorState {
                title: Some("Failed to load stack".to_string()),
                error: err.display_message(),
            }
        },
        Ok(callframes) if callframes.is_empty() => rsx! {
            EmptyState {
                message: format!(
                    "No stack frames for thread {tid_label}. The thread may be idle or not yet sampled."
                )
            }
        },
        Ok(callframes) => {
            let current_mode = filter_mode.clone();
            let filtered: Vec<_> = callframes
                .iter()
                .filter(|cf| matches_mode(cf, current_mode.as_str()))
                .cloned()
                .collect();
            let shown = filtered.len();

            if filtered.is_empty() {
                rsx! {
                    EmptyState {
                        message: format!(
                            "No frames match the \"{}\" filter",
                            mode_label(&current_mode)
                        )
                    }
                }
            } else {
                rsx! {
                    div { class: "space-y-0",
                        for (idx, cf) in filtered.iter().enumerate() {
                            CallStackView {
                                key: "{refresh_tick}-{idx}",
                                callstack: cf.clone(),
                                index: idx,
                                is_last: idx + 1 == shown,
                                default_open: idx == 0,
                            }
                        }
                    }
                }
            }
        }
    }
}

fn stack_snapshot_for(
    tid_label: &str,
    result: &Result<Vec<CallFrame>, AppError>,
    mode: &str,
) -> StackSnapshot {
    match result {
        Err(_) => StackSnapshot::default(),
        Ok(frames) if frames.is_empty() => StackSnapshot {
            tid_label: tid_label.to_string(),
            loaded: true,
            ..StackSnapshot::default()
        },
        Ok(frames) => {
            let (py_count, rust_count, cpp_count) = count_by_kind(frames);
            let shown = frames.iter().filter(|cf| matches_mode(cf, mode)).count();
            StackSnapshot {
                tid_label: tid_label.to_string(),
                total: frames.len(),
                py: py_count,
                rust: rust_count,
                cpp: cpp_count,
                shown,
                loaded: true,
            }
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
