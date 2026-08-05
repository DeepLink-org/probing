//! Current workspace page — route, description, and fetched snapshot for the Agent.

use dioxus::prelude::*;

use crate::app::Route;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageContext {
    pub page_id: String,
    pub title: String,
    pub path: String,
    pub description: String,
    pub suggested_skills: Vec<String>,
    pub investigation_summary: String,
    pub investigation_coordinates: String,
    pub investigation_key: String,
    /// Extra hints pushed by the active page component (optional).
    pub local_hints: Vec<String>,
    /// Text snapshot from page tools (SQL/API).
    pub snapshot: String,
    pub snapshot_loading: bool,
    /// Request time of evidence published by the visible Next page.
    pub evidence_requested_at_ms: Option<u64>,
}

pub struct PageContextDescriptor {
    pub page_id: String,
    pub title: String,
    pub path: String,
    pub description: String,
    pub suggested_skills: Vec<String>,
    pub investigation_summary: String,
    pub investigation_coordinates: String,
    pub investigation_key: String,
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
        if !self.investigation_coordinates.is_empty()
            && self.investigation_coordinates != "No context"
            && self.investigation_coordinates != self.investigation_summary
        {
            lines.push(format!(
                "Investigation coordinates: {}",
                self.investigation_coordinates
            ));
        }
        if !self.local_hints.is_empty() {
            lines.push(format!("Page hints: {}", self.local_hints.join("; ")));
        }
        if !self.suggested_skills.is_empty() {
            lines.push(format!(
                "Suggested skills on this page: {}",
                self.suggested_skills.join(", ")
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

pub fn apply_page_descriptor(descriptor: PageContextDescriptor) {
    let PageContextDescriptor {
        page_id,
        title,
        path,
        description,
        suggested_skills,
        investigation_summary,
        investigation_coordinates,
        investigation_key,
    } = descriptor;
    let mut ctx = PAGE_CONTEXT.read().clone();
    let route_changed = ctx.page_id != page_id;
    let investigation_changed = ctx.investigation_key != investigation_key;
    ctx.page_id = page_id;
    ctx.title = title;
    ctx.path = path;
    ctx.description = description;
    ctx.suggested_skills = suggested_skills;
    ctx.investigation_summary = investigation_summary;
    ctx.investigation_coordinates = investigation_coordinates;
    ctx.investigation_key = investigation_key;
    if route_changed || investigation_changed {
        ctx.local_hints.clear();
        ctx.snapshot.clear();
        ctx.snapshot_loading = true;
        ctx.evidence_requested_at_ms = None;
    }
    commit_page_context(ctx);
}

pub fn set_page_snapshot(snapshot: String) {
    let mut ctx = PAGE_CONTEXT.read().clone();
    ctx.snapshot = snapshot;
    ctx.snapshot_loading = false;
    ctx.evidence_requested_at_ms = None;
    commit_page_context(ctx);
}

/// Publish the exact evidence bundle already used by the visible Next page.
///
/// Late responses from an old route, investigation coordinate, or refresh are
/// ignored so Agent context cannot move backwards while the UI moves forward.
pub fn publish_page_evidence(
    page_id: &str,
    investigation_key: &str,
    requested_at_ms: u64,
    snapshot: String,
) -> bool {
    let mut ctx = PAGE_CONTEXT.read().clone();
    if !page_evidence_is_current(&ctx, page_id, investigation_key, requested_at_ms) {
        return false;
    }
    ctx.snapshot = snapshot;
    ctx.snapshot_loading = false;
    ctx.evidence_requested_at_ms = Some(requested_at_ms);
    commit_page_context(ctx);
    true
}

fn page_evidence_is_current(
    ctx: &PageContext,
    page_id: &str,
    investigation_key: &str,
    requested_at_ms: u64,
) -> bool {
    ctx.page_id == page_id
        && ctx.investigation_key == investigation_key
        && ctx
            .evidence_requested_at_ms
            .is_none_or(|current| current <= requested_at_ms)
}

pub fn set_page_snapshot_loading(loading: bool) {
    let mut ctx = PAGE_CONTEXT.read().clone();
    if ctx.snapshot_loading == loading {
        return;
    }
    ctx.snapshot_loading = loading;
    commit_page_context(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_label_cannot_make_different_coordinates_share_evidence() {
        let ctx = PageContext {
            page_id: "next_training".into(),
            investigation_summary: "selected rank".into(),
            investigation_key: "rank=58".into(),
            evidence_requested_at_ms: Some(100),
            ..Default::default()
        };

        assert!(page_evidence_is_current(
            &ctx,
            "next_training",
            "rank=58",
            101
        ));
        assert!(!page_evidence_is_current(
            &ctx,
            "next_training",
            "rank=57",
            101
        ));
        assert!(!page_evidence_is_current(
            &ctx,
            "next_training",
            "rank=58",
            99
        ));
    }
}
