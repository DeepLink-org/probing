use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InvestigationContext {
    pub pid: Option<i32>,
    pub tid: Option<i32>,
    pub trace_id: Option<i64>,
    pub span_name: Option<String>,
    /// Training coordinate from step matrix / heatmap (filters span attributes on Spans page).
    pub local_step: Option<i64>,
    pub label: Option<String>,
}

impl InvestigationContext {
    pub fn is_empty(&self) -> bool {
        self.pid.is_none()
            && self.tid.is_none()
            && self.trace_id.is_none()
            && self.span_name.is_none()
            && self.local_step.is_none()
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

/// Thread id to filter pprof flamegraph (set from Dashboard CPU thread actions).
pub static PROFILING_THREAD_FILTER: GlobalSignal<Option<i32>> = Signal::global(|| None);

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
    crate::state::investigation_url::sync_investigation_context_to_url();
    clear_profiling_thread_filter();
}

pub fn clear_profiling_thread_filter() {
    *PROFILING_THREAD_FILTER.write() = None;
}

/// Clear span-tree filters while keeping process context (pid).
pub fn clear_spans_investigation_filters() {
    let pid = INVESTIGATION_CONTEXT.read().pid;
    clear_investigation_context();
    if let Some(pid) = pid {
        update_investigation_context(|ctx| {
            ctx.pid = Some(pid);
            ctx.label = Some(format!("pid {pid}"));
        });
    }
}

/// Stable key for detecting external investigation context changes.
pub fn investigation_context_key(ctx: &InvestigationContext) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        ctx.pid.unwrap_or(-1),
        ctx.tid.unwrap_or(-1),
        ctx.trace_id.unwrap_or(-1),
        ctx.span_name.as_deref().unwrap_or(""),
        ctx.local_step.unwrap_or(-1),
    )
}

/// Write Spans page filters back into global context (and URL).
pub fn sync_spans_filters_to_context(
    name_filter: &str,
    thread_filter: &str,
    trace_id_filter: &str,
) {
    update_investigation_context(|ctx| {
        let name = name_filter.trim();
        ctx.span_name = if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        };

        let thread = thread_filter.trim();
        if thread.is_empty() {
            ctx.tid = None;
        } else if let Ok(tid) = thread.parse::<i32>() {
            ctx.tid = Some(tid);
        }

        let trace = trace_id_filter.trim();
        if trace.is_empty() {
            ctx.trace_id = None;
        } else if let Ok(trace_id) = trace.parse::<i64>() {
            ctx.trace_id = Some(trace_id);
        }

        ctx.label = if ctx.is_empty() {
            None
        } else {
            Some(ctx.summary())
        };
    });
}

fn column_index(names: &[String], candidates: &[&str]) -> Option<usize> {
    names.iter().position(|name| {
        let lower = name.to_lowercase();
        candidates
            .iter()
            .any(|c| lower == *c || lower.ends_with(&format!("_{c}")))
    })
}

fn cell_i32(df: &probing_proto::prelude::DataFrame, row: usize, col: usize) -> Option<i32> {
    use probing_proto::prelude::Ele;
    match df.cols.get(col)?.get(row) {
        Ele::I32(v) => Some(v),
        Ele::I64(v) => i32::try_from(v).ok(),
        Ele::Text(s) => s.parse().ok(),
        _ => None,
    }
}

fn cell_i64(df: &probing_proto::prelude::DataFrame, row: usize, col: usize) -> Option<i64> {
    use probing_proto::prelude::Ele;
    match df.cols.get(col)?.get(row) {
        Ele::I64(v) => Some(v),
        Ele::I32(v) => Some(v as i64),
        Ele::Text(s) => s.parse().ok(),
        _ => None,
    }
}

fn cell_text(df: &probing_proto::prelude::DataFrame, row: usize, col: usize) -> Option<String> {
    use probing_proto::prelude::Ele;
    match df.cols.get(col)?.get(row) {
        Ele::Text(s) if !s.is_empty() => Some(s.clone()),
        Ele::I32(v) => Some(v.to_string()),
        Ele::I64(v) => Some(v.to_string()),
        _ => None,
    }
}

/// Apply investigation context from an Agent/SQL result row (tid, trace_id, span name columns).
pub fn apply_context_from_dataframe_row(df: &probing_proto::prelude::DataFrame, row: usize) {
    let tid = column_index(&df.names, &["tid", "thread_id", "thread"])
        .and_then(|c| cell_i32(df, row, c));
    let trace_id = column_index(&df.names, &["trace_id", "trace"])
        .and_then(|c| cell_i64(df, row, c));
    let span_name = column_index(
        &df.names,
        &["span_name", "span", "name", "operation", "op"],
    )
    .and_then(|c| cell_text(df, row, c));
    let pid = column_index(&df.names, &["pid", "process_id"]).and_then(|c| cell_i32(df, row, c));
    let rank = column_index(&df.names, &["rank", "_rank"]).and_then(|c| cell_i32(df, row, c));
    let local_step = column_index(&df.names, &["local_step", "step"])
        .and_then(|c| cell_i64(df, row, c));

    if tid.is_none()
        && trace_id.is_none()
        && span_name.is_none()
        && pid.is_none()
        && rank.is_none()
    {
        return;
    }

    update_investigation_context(|ctx| {
        if let Some(p) = pid {
            ctx.pid = Some(p);
        }
        if let Some(t) = tid {
            ctx.tid = Some(t);
        }
        if let Some(id) = trace_id {
            ctx.trace_id = Some(id);
        }
        if let Some(name) = span_name {
            ctx.span_name = Some(name);
        }
        if let Some(r) = rank {
            let mut label = format!("rank {r}");
            if let Some(step) = local_step {
                label.push_str(&format!(" · step {step}"));
            }
            if let Some(ref op) = ctx.span_name {
                label.push_str(&format!(" · {op}"));
            }
            ctx.label = Some(label);
        } else {
            ctx.label = Some(ctx.summary());
        }
    });
}

/// Pin investigation context to a train.step heatmap cell (rank + optional step).
pub fn set_training_step_context(rank: i32, local_step: Option<i64>, host: Option<&str>) {
    let mut label = format!("rank {rank}");
    if let Some(step) = local_step {
        label.push_str(&format!(" · step {step}"));
    }
    if let Some(h) = host {
        if !h.is_empty() {
            label.push_str(&format!(" · {h}"));
        }
    }
    update_investigation_context(|ctx| {
        ctx.tid = None;
        ctx.trace_id = None;
        ctx.span_name = Some("train.step".to_string());
        ctx.local_step = local_step;
        ctx.label = Some(label);
    });
    clear_profiling_thread_filter();
}

pub fn set_thread_context(tid: i32, thread_name: Option<&str>, pid: Option<i32>) {
    let label = match thread_name {
        Some(name) if !name.is_empty() => format!("thread {tid} · {name}"),
        _ => format!("thread {tid}"),
    };
    update_investigation_context(|ctx| {
        ctx.tid = Some(tid);
        ctx.pid = pid.or(ctx.pid);
        ctx.trace_id = None;
        ctx.span_name = None;
        ctx.local_step = None;
        ctx.label = Some(label);
    });
    *PROFILING_THREAD_FILTER.write() = Some(tid);
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
        ctx.local_step = None;
        ctx.label = Some(label);
    });
}

#[allow(dead_code)]
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
