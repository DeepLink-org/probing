//! Bridge source preview → Investigation Agent (context + prompts).

use dioxus::prelude::*;

use crate::agent::load_playbook;
use crate::state::agent::{open_agent_prefill, queue_agent_action, AgentPendingAction};
use crate::state::page_context::{set_page_local_hints, PAGE_CONTEXT};
use crate::utils::source_ref::{language_class, SourceSlice};

const DEFAULT_SOURCE_PLAYBOOKS: &[&str] = &["training_hang", "health_overview", "module_bottleneck"];

/// Attach the visible source slice to page hints so LLM / playbooks see it.
pub fn attach_source_focus(path: &str, line: Option<i64>, slice: &SourceSlice) {
    set_page_local_hints(vec![build_source_hint(path, line, slice)]);
}

pub fn suggested_playbooks_for_source() -> Vec<String> {
    let page = PAGE_CONTEXT.read();
    let from_page: Vec<String> = page
        .suggested_playbooks
        .iter()
        .filter(|id| load_playbook(id).is_some())
        .cloned()
        .collect();
    if !from_page.is_empty() {
        return from_page.into_iter().take(4).collect();
    }
    DEFAULT_SOURCE_PLAYBOOKS
        .iter()
        .filter(|id| load_playbook(id).is_some())
        .map(|s| (*s).to_string())
        .collect()
}

/// Open Investigate with a pre-filled question about the focused source.
pub fn ask_agent_about_source(path: &str, line: Option<i64>, slice: &SourceSlice) {
    attach_source_focus(path, line, slice);
    open_agent_prefill(build_user_prompt(path, line, slice));
}

/// Open Investigate and immediately submit the source-focused question.
pub fn ask_and_run_agent_about_source(path: &str, line: Option<i64>, slice: &SourceSlice) {
    attach_source_focus(path, line, slice);
    queue_agent_action(AgentPendingAction::SubmitText(build_user_prompt(
        path, line, slice,
    )));
}

/// Run a playbook with the current source slice attached as page context.
pub fn run_playbook_with_source(playbook_id: &str, path: &str, line: Option<i64>, slice: &SourceSlice) {
    if load_playbook(playbook_id).is_none() {
        return;
    }
    attach_source_focus(path, line, slice);
    queue_agent_action(AgentPendingAction::RunPlaybook(playbook_id.to_string()));
}

fn build_user_prompt(path: &str, line: Option<i64>, slice: &SourceSlice) -> String {
    let loc = match line {
        Some(l) => format!("{path}:{l}"),
        None => path.to_string(),
    };
    let highlight = highlighted_line_text(slice)
        .map(|t| format!("\nHighlighted line:\n{t}\n"))
        .unwrap_or_default();
    format!(
        "I'm inspecting source at {loc} while debugging.\n\
         {highlight}\n\
         Source excerpt ({lang}, lines {start}–{end}):\n\
         ```\n{body}\n```\n\
         What might cause a stall or slowness here? Recommend playbooks and next checks.",
        lang = language_class(path),
        start = slice.start_line,
        end = slice.end_line,
        body = slice.text.trim(),
    )
}

fn build_source_hint(path: &str, line: Option<i64>, slice: &SourceSlice) -> String {
    let loc = match line {
        Some(l) => format!("{path}:{l}"),
        None => path.to_string(),
    };
    format!(
        "Source focus: {loc} (lines {}–{} of {})\n```\n{}\n```",
        slice.start_line,
        slice.end_line,
        slice.total_lines,
        slice.text.trim()
    )
}

fn highlighted_line_text(slice: &SourceSlice) -> Option<String> {
    let hl = slice.highlight_line? as usize;
    if hl < slice.start_line || hl > slice.end_line {
        return None;
    }
    slice
        .text
        .lines()
        .nth(hl - slice.start_line)
        .map(str::to_string)
}
