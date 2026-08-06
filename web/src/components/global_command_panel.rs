//! Global Command Panel (VS Code style) and floating result toast.
//!
//! Open via ⌘K. Select/Enter executes the command via `ApiClient::eval`.

use dioxus::prelude::*;

use crate::api::{ApiClient, MagicGroup, MagicItem};
use crate::hooks::use_api;
use crate::state::commands::{
    Cell, EvalState, FloatingResult, COMMAND_PANEL_OPEN, EVAL_HISTORY, FLOATING_RESULT,
    SHORTCUTS_HELP_OPEN,
};

/// Flatten groups into searchable items
fn flatten_magics(groups: &[MagicGroup]) -> Vec<(String, MagicItem)> {
    groups
        .iter()
        .flat_map(|g: &MagicGroup| {
            g.items
                .iter()
                .map(move |i: &MagicItem| (g.group.clone(), i.clone()))
        })
        .collect()
}

/// Filter by query
fn filter_magics(items: &[(String, MagicItem)], query: &str) -> Vec<(String, MagicItem)> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return items.to_vec();
    }
    items
        .iter()
        .filter(|(_, item)| {
            item.command.to_lowercase().contains(&q)
                || item.label.to_lowercase().contains(&q)
                || item.help.to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}

fn execute_command(code: String) {
    let code = code.trim().to_string();
    if code.is_empty() {
        return;
    }
    *COMMAND_PANEL_OPEN.write() = false;
    spawn(async move {
        let client = ApiClient::new();
        let eval_state = match client.eval(&code).await {
            Ok(resp) => {
                let mut text = resp.output;
                if !resp.traceback.is_empty() {
                    text.push('\n');
                    text.push_str(&resp.traceback.join("\n"));
                }
                EvalState {
                    output: text,
                    is_error: resp.status == "error" || !resp.traceback.is_empty(),
                }
            }
            Err(e) => EvalState {
                output: e.display_message(),
                is_error: true,
            },
        };

        EVAL_HISTORY.write().push(Cell {
            input: code.clone(),
            output: eval_state.clone(),
        });

        *FLOATING_RESULT.write() = Some(FloatingResult {
            command: code,
            output: eval_state.output,
            is_error: eval_state.is_error,
        });
    });
}

#[component]
fn CommandPanelItem(
    cmd: String,
    help: String,
    group: String,
    is_selected: bool,
    on_select: EventHandler<String>,
) -> Element {
    let cmd_clone = cmd.clone();
    rsx! {
        button {
            class: if is_selected {
                "w-full text-left px-4 py-2 bg-blue-50 border-l-2 border-blue-600 flex flex-col gap-0.5"
            } else {
                "w-full text-left px-4 py-2 hover:bg-gray-100 focus:outline-none focus:bg-gray-100 flex flex-col gap-0.5 border-l-2 border-transparent"
            },
            onclick: move |_| on_select.call(cmd_clone.clone()),
            div {
                class: "flex items-center gap-2",
                span { class: "text-sm font-mono font-medium text-gray-800", "{cmd}" }
                span { class: "text-xs text-gray-400", "{group}" }
            }
            if !help.is_empty() {
                div { class: "text-xs text-gray-500 truncate", "{help}" }
            }
        }
    }
}

/// Global Command Panel overlay. Select / Enter runs the command and shows a toast.
#[component]
pub fn GlobalCommandPanel() -> Element {
    let mut panel_query = use_signal(String::new);
    let mut highlight_idx = use_signal(|| 0usize);

    let magics_state = use_api(move || {
        let client = ApiClient::new();
        async move { client.get_magics().await }
    });

    let query = panel_query.read().clone();
    let all_items = match magics_state.data.read().as_ref() {
        Some(Ok(groups)) => flatten_magics(groups),
        _ => vec![],
    };
    let filtered = filter_magics(&all_items, &query);
    let items_to_show: Vec<(String, String, String)> = filtered
        .into_iter()
        .take(50)
        .map(|(g, m)| (g, m.command, m.help))
        .collect();

    let item_count = items_to_show.len();
    if item_count > 0 {
        let idx = *highlight_idx.read();
        if idx >= item_count {
            *highlight_idx.write() = item_count - 1;
        }
    } else {
        *highlight_idx.write() = 0;
    }

    let current_highlight = *highlight_idx.read();
    let history_preview: Vec<(String, bool)> = EVAL_HISTORY
        .read()
        .iter()
        .rev()
        .take(5)
        .map(|h| (h.input.clone(), h.output.is_error))
        .collect();

    let on_select = EventHandler::new(move |selected: String| {
        execute_command(selected);
    });

    rsx! {
        div {
            class: "fixed inset-0 z-[9998] flex items-start justify-center pt-[15vh] bg-black/20",
            onclick: move |_| *COMMAND_PANEL_OPEN.write() = false,
            div {
                class: "w-full max-w-xl mx-4 bg-white rounded-lg shadow-2xl border border-gray-200 overflow-hidden",
                onclick: move |e| { e.stop_propagation(); },
                input {
                    r#type: "text",
                    autofocus: true,
                    class: "w-full px-4 py-3 text-sm font-mono border-b border-gray-200 focus:outline-none focus:ring-0",
                    placeholder: "Type to search… ↑↓ navigate, Enter runs the command",
                    value: "{query}",
                    oninput: move |e| {
                        *panel_query.write() = e.value();
                        *highlight_idx.write() = 0;
                    },
                    onkeydown: move |e: dioxus::html::events::KeyboardEvent| {
                        use dioxus::html::input_data::keyboard_types::Key;
                        if e.key() == Key::Escape {
                            *COMMAND_PANEL_OPEN.write() = false;
                        } else if e.key() == Key::Enter {
                            if !items_to_show.is_empty() {
                                let idx = current_highlight.min(items_to_show.len() - 1);
                                let cmd = items_to_show[idx].1.clone();
                                execute_command(cmd);
                            } else {
                                let typed = panel_query.read().trim().to_string();
                                if !typed.is_empty() {
                                    execute_command(typed);
                                }
                            }
                        } else if e.key() == Key::ArrowDown {
                            e.prevent_default();
                            if !items_to_show.is_empty() {
                                let idx = current_highlight.min(items_to_show.len() - 1);
                                *highlight_idx.write() = (idx + 1).min(items_to_show.len() - 1);
                            }
                        } else if e.key() == Key::ArrowUp {
                            e.prevent_default();
                            if current_highlight > 0 {
                                *highlight_idx.write() = current_highlight - 1;
                            }
                        }
                    },
                }
                div {
                    class: "max-h-96 overflow-y-auto py-1",
                    if magics_state.is_loading() {
                        div { class: "px-4 py-6 text-sm text-gray-500", "Loading..." }
                    } else if all_items.is_empty() {
                        div { class: "px-4 py-6 text-sm text-gray-500", "No magics (REPL not ready)" }
                    } else if items_to_show.is_empty() {
                        div { class: "px-4 py-6 text-sm text-gray-500", "No matching commands — press Enter to run typed text" }
                    } else {
                        for (i, row) in items_to_show.iter().enumerate() {
                            CommandPanelItem {
                                cmd: row.1.clone(),
                                help: row.2.clone(),
                                group: row.0.clone(),
                                is_selected: i == current_highlight,
                                on_select,
                            }
                        }
                    }
                }
                if !history_preview.is_empty() {
                    div {
                        class: "border-t border-gray-100 bg-gray-50 px-4 py-2",
                        div { class: "mb-1 text-[11px] font-semibold uppercase tracking-wide text-gray-400", "Recent" }
                        div { class: "flex flex-col gap-0.5",
                            for (cmd, is_error) in history_preview {
                                button {
                                    class: if is_error {
                                        "w-full truncate text-left font-mono text-xs text-red-600 hover:underline"
                                    } else {
                                        "w-full truncate text-left font-mono text-xs text-gray-600 hover:underline"
                                    },
                                    onclick: {
                                        let cmd = cmd.clone();
                                        move |_| execute_command(cmd.clone())
                                    },
                                    "{cmd}"
                                }
                            }
                        }
                    }
                }
                div {
                    class: "flex items-center justify-between border-t border-gray-100 px-4 py-2 text-xs text-gray-400",
                    span { "Enter runs · Esc closes" }
                    button {
                        class: "hover:text-gray-600",
                        onclick: move |_| {
                            *COMMAND_PANEL_OPEN.write() = false;
                            *SHORTCUTS_HELP_OPEN.write() = true;
                        },
                        "Shortcuts ?"
                    }
                }
            }
        }
    }
}

/// Centered modal showing execution result. Mounted from NextShell so it outlives the panel.
#[component]
pub fn FloatingResultToast() -> Element {
    let opt = FLOATING_RESULT.read().clone();
    if let Some(ref fr) = opt {
        let output = fr.output.clone();
        let is_error = fr.is_error;
        let command = fr.command.clone();
        rsx! {
            div {
                class: "fixed inset-0 z-[9999] flex items-start justify-center pt-[10vh] bg-black/20",
                onclick: move |_| *FLOATING_RESULT.write() = None,
                div {
                    class: "w-full max-w-2xl mx-4 max-h-[80vh] overflow-hidden rounded-lg shadow-2xl border border-gray-200 bg-white flex flex-col",
                    onclick: move |e| { e.stop_propagation(); },
                    div {
                        class: if is_error { "px-4 py-3 bg-red-50 border-b border-red-100 text-red-800 font-medium text-sm" } else { "px-4 py-3 bg-gray-50 border-b border-gray-200 text-gray-800 font-medium text-sm" },
                        "{command}"
                    }
                    div {
                        class: "p-4 overflow-y-auto flex-1 text-sm font-mono whitespace-pre-wrap min-h-[200px]",
                        class: if is_error { "text-red-700" } else { "text-gray-800" },
                        if output.is_empty() {
                            "(no output)"
                        } else {
                            "{output}"
                        }
                    }
                    div {
                        class: "px-4 py-2 border-t border-gray-200 flex justify-end",
                        button {
                            class: "px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 hover:bg-gray-200 rounded-lg",
                            onclick: move |_| *FLOATING_RESULT.write() = None,
                            "Close"
                        }
                    }
                }
            }
        }
    } else {
        rsx! { div {} }
    }
}
