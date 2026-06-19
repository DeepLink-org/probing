use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InvestigationContext {
    pub pid: Option<i32>,
    pub tid: Option<i32>,
    pub trace_id: Option<i64>,
    pub span_name: Option<String>,
    pub label: Option<String>,
}

impl InvestigationContext {
    pub fn is_empty(&self) -> bool {
        self.pid.is_none()
            && self.tid.is_none()
            && self.trace_id.is_none()
            && self.span_name.is_none()
            && self.label.is_none()
    }

    pub fn summary(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        let mut parts = Vec::new();
        if let Some(pid) = self.pid {
            parts.push(format!("pid {pid}"));
        }
        if let Some(tid) = self.tid {
            parts.push(format!("tid {tid}"));
        }
        if let Some(trace_id) = self.trace_id {
            parts.push(format!("trace {trace_id}"));
        }
        if let Some(name) = &self.span_name {
            parts.push(name.clone());
        }
        if parts.is_empty() {
            "No context".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

pub static INVESTIGATION_CONTEXT: GlobalSignal<InvestigationContext> =
    Signal::global(InvestigationContext::default);

const STORAGE_KEY: &str = "probing_investigation_context";

pub fn load_investigation_context() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    let Ok(Some(raw)) = storage.get_item(STORAGE_KEY) else {
        crate::state::investigation_url::apply_investigation_context_from_url();
        return;
    };
    if let Ok(ctx) = serde_json::from_str::<InvestigationContext>(&raw) {
        *INVESTIGATION_CONTEXT.write() = ctx;
    }
    crate::state::investigation_url::apply_investigation_context_from_url();
}

fn save_investigation_context(ctx: &InvestigationContext) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    if ctx.is_empty() {
        let _ = storage.remove_item(STORAGE_KEY);
        return;
    }
    if let Ok(raw) = serde_json::to_string(ctx) {
        let _ = storage.set_item(STORAGE_KEY, &raw);
    }
}

pub fn update_investigation_context(mutator: impl FnOnce(&mut InvestigationContext)) {
    let previous = INVESTIGATION_CONTEXT.read().clone();
    let mut ctx = previous.clone();
    mutator(&mut ctx);
    if ctx == previous {
        return;
    }
    *INVESTIGATION_CONTEXT.write() = ctx.clone();
    save_investigation_context(&ctx);
    crate::state::investigation_url::sync_investigation_context_to_url();
}

pub fn clear_investigation_context() {
    *INVESTIGATION_CONTEXT.write() = InvestigationContext::default();
    save_investigation_context(&InvestigationContext::default());
}

pub fn set_thread_context(tid: i32, thread_name: Option<&str>, pid: Option<i32>) {
    let label = match thread_name {
        Some(name) if !name.is_empty() => format!("thread {tid} · {name}"),
        _ => format!("thread {tid}"),
    };
    update_investigation_context(|ctx| {
        ctx.tid = Some(tid);
        ctx.pid = pid.or(ctx.pid);
        ctx.label = Some(label);
    });
}

pub fn set_trace_context(trace_id: i64, span_name: Option<&str>, tid: Option<i32>) {
    let label = match span_name {
        Some(name) => format!("trace {trace_id} · {name}"),
        None => format!("trace {trace_id}"),
    };
    update_investigation_context(|ctx| {
        ctx.trace_id = Some(trace_id);
        ctx.span_name = span_name.map(str::to_string);
        ctx.tid = tid.or(ctx.tid);
        ctx.label = Some(label);
    });
}

pub fn set_process_context(pid: i32, label: Option<&str>) {
    update_investigation_context(|ctx| {
        ctx.pid = Some(pid);
        if let Some(text) = label {
            ctx.label = Some(text.to_string());
        } else if ctx.label.is_none() {
            ctx.label = Some(format!("pid {pid}"));
        }
    });
}

/// Sync pid from Dashboard overview without overwriting thread/trace context.
pub fn sync_overview_process_context(pid: i32, exe: &str) {
    update_investigation_context(|ctx| {
        ctx.pid = Some(pid);
        if ctx.tid.is_none() && ctx.trace_id.is_none() && ctx.label.is_none() {
            ctx.label = Some(format!("{exe} · pid {pid}"));
        }
    });
}
