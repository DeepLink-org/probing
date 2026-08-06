use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InvestigationContext {
    pub pid: Option<i32>,
    pub tid: Option<i32>,
    pub rank: Option<i32>,
    pub host: Option<String>,
    pub device_id: Option<i32>,
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
            && self.rank.is_none()
            && self.host.is_none()
            && self.device_id.is_none()
            && self.trace_id.is_none()
            && self.span_name.is_none()
            && self.local_step.is_none()
            && self.label.is_none()
    }

    pub fn summary(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        self.derived_summary()
    }

    /// Exact coordinates used to filter evidence, independent of a friendly
    /// presentation label such as a thread name.
    pub fn coordinates_summary(&self) -> String {
        self.derived_summary()
    }

    fn derived_summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(pid) = self.pid {
            parts.push(format!("pid {pid}"));
        }
        if let Some(tid) = self.tid {
            parts.push(format!("tid {tid}"));
        }
        if let Some(rank) = self.rank {
            parts.push(format!("rank {rank}"));
        }
        if let Some(host) = &self.host {
            parts.push(host.clone());
        }
        if let Some(device_id) = self.device_id {
            parts.push(format!("GPU {device_id}"));
        }
        if let Some(step) = self.local_step {
            parts.push(format!("step {step}"));
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

/// Stable key for detecting external investigation context changes.
pub fn investigation_context_key(ctx: &InvestigationContext) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        ctx.pid.unwrap_or(-1),
        ctx.tid.unwrap_or(-1),
        ctx.rank.unwrap_or(-1),
        ctx.host.as_deref().unwrap_or(""),
        ctx.device_id.unwrap_or(-1),
        ctx.trace_id.unwrap_or(-1),
        ctx.span_name.as_deref().unwrap_or(""),
        ctx.local_step.unwrap_or(-1),
    )
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
        ctx.rank = Some(rank);
        ctx.host = host.filter(|value| !value.is_empty()).map(str::to_string);
        ctx.device_id = None;
        ctx.tid = None;
        ctx.trace_id = None;
        ctx.span_name = Some("train.step".to_string());
        ctx.local_step = local_step;
        ctx.label = Some(label);
    });
    clear_profiling_thread_filter();
}

/// Pin a reported rank while preserving the currently selected training step.
pub fn set_training_rank_context(rank: i32, fallback_step: Option<i64>, host: Option<&str>) {
    let local_step = INVESTIGATION_CONTEXT.read().local_step.or(fallback_step);
    set_training_step_context(rank, local_step, host);
}

/// Pin a physical accelerator while keeping any step/span coordinate available
/// for cross-page investigation.
pub fn set_memory_device_context(rank: Option<i32>, host: Option<&str>, device_id: i32) {
    update_investigation_context(|ctx| {
        ctx.rank = rank;
        ctx.host = host.filter(|value| !value.is_empty()).map(str::to_string);
        ctx.device_id = Some(device_id);
        ctx.label = None;
        ctx.label = Some(ctx.derived_summary());
    });
}

/// Pin a reported process/accelerator without discarding the current step.
/// Cluster, Training, and Memory use this common coordinate set.
pub fn set_node_context(rank: Option<i32>, host: Option<&str>, device_id: Option<i32>) {
    update_investigation_context(|ctx| {
        ctx.rank = rank;
        ctx.host = host.filter(|value| !value.is_empty()).map(str::to_string);
        ctx.device_id = device_id;
        ctx.label = None;
        ctx.label = Some(ctx.derived_summary());
    });
}

pub fn set_trace_context(trace_id: i64, span_name: Option<&str>, tid: Option<i32>) {
    update_investigation_context(|ctx| {
        ctx.trace_id = Some(trace_id);
        ctx.span_name = span_name.map(str::to_string);
        ctx.tid = tid.or(ctx.tid);
        ctx.label = None;
        ctx.label = Some(ctx.derived_summary());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_label_does_not_hide_exact_evidence_coordinates() {
        let context = InvestigationContext {
            pid: Some(42),
            tid: Some(7),
            rank: Some(58),
            host: Some("node-07".into()),
            device_id: Some(2),
            label: Some("trainer thread".into()),
            ..Default::default()
        };

        assert_eq!(context.summary(), "trainer thread");
        assert_eq!(
            context.coordinates_summary(),
            "pid 42 · tid 7 · rank 58 · node-07 · GPU 2"
        );
    }

    #[test]
    fn investigation_identity_ignores_friendly_label_but_tracks_coordinates() {
        let first = InvestigationContext {
            rank: Some(1),
            label: Some("selected rank".into()),
            ..Default::default()
        };
        let second = InvestigationContext {
            rank: Some(2),
            label: Some("selected rank".into()),
            ..Default::default()
        };

        assert_ne!(
            investigation_context_key(&first),
            investigation_context_key(&second)
        );
    }
}
