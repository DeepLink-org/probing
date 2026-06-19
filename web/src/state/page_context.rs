//! Current workspace page — route, description, and fetched snapshot for the Agent.

use dioxus::prelude::*;

use crate::app::Route;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageContext {
    pub page_id: String,
    pub title: String,
    pub path: String,
    pub description: String,
    pub suggested_playbooks: Vec<String>,
    pub investigation_summary: String,
    /// Extra hints pushed by the active page component (optional).
    pub local_hints: Vec<String>,
    /// Text snapshot from page tools (SQL/API).
    pub snapshot: String,
    pub snapshot_loading: bool,
}

impl PageContext {
    pub fn llm_block(&self) -> String {
        let mut lines = vec![
            format!("Current page: {} ({})", self.title, self.page_id),
            format!("Path: {}", self.path),
            self.description.clone(),
        ];
        if !self.investigation_summary.is_empty() && self.investigation_summary != "No context" {
            lines.push(format!(
                "Investigation context: {}",
                self.investigation_summary
            ));
        }
        if !self.local_hints.is_empty() {
            lines.push(format!("Page hints: {}", self.local_hints.join("; ")));
        }
        if !self.suggested_playbooks.is_empty() {
            lines.push(format!(
                "Suggested playbooks on this page: {}",
                self.suggested_playbooks.join(", ")
            ));
        }
        if self.snapshot_loading {
            lines.push("Page snapshot: (loading…)".to_string());
        } else if !self.snapshot.is_empty() {
            lines.push(format!("Page snapshot:\n{}", self.snapshot));
        } else {
            lines.push("Page snapshot: (none)".to_string());
        }
        lines.join("\n")
    }
}

pub static PAGE_CONTEXT: GlobalSignal<PageContext> = Signal::global(PageContext::default);

/// Active route (for snapshot refresh before Agent LLM calls).
pub static CURRENT_ROUTE: GlobalSignal<Option<Route>> = Signal::global(|| None);

fn commit_page_context(ctx: PageContext) {
    if ctx == *PAGE_CONTEXT.read() {
        return;
    }
    *PAGE_CONTEXT.write() = ctx;
}

pub fn set_page_local_hints(hints: Vec<String>) {
    let mut ctx = PAGE_CONTEXT.read().clone();
    ctx.local_hints = hints;
    commit_page_context(ctx);
}

pub fn apply_page_descriptor(
    page_id: String,
    title: String,
    path: String,
    description: String,
    suggested_playbooks: Vec<String>,
    investigation_summary: String,
) {
    let mut ctx = PAGE_CONTEXT.read().clone();
    let route_changed = ctx.page_id != page_id;
    ctx.page_id = page_id;
    ctx.title = title;
    ctx.path = path;
    ctx.description = description;
    ctx.suggested_playbooks = suggested_playbooks;
    ctx.investigation_summary = investigation_summary;
    if route_changed {
        ctx.local_hints.clear();
        ctx.snapshot.clear();
        ctx.snapshot_loading = true;
    }
    commit_page_context(ctx);
}

pub fn set_page_snapshot(snapshot: String) {
    let mut ctx = PAGE_CONTEXT.read().clone();
    ctx.snapshot = snapshot;
    ctx.snapshot_loading = false;
    commit_page_context(ctx);
}

pub fn set_page_snapshot_loading(loading: bool) {
    let mut ctx = PAGE_CONTEXT.read().clone();
    if ctx.snapshot_loading == loading {
        return;
    }
    ctx.snapshot_loading = loading;
    commit_page_context(ctx);
}
