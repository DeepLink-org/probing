use dioxus::prelude::*;

/// Server-aligned default: profiling off until `get_profiler_config` runs.
pub static PROFILING_PPROF_FREQ: GlobalSignal<i32> = Signal::global(|| 0);
pub static PROFILING_TORCH_ENABLED: GlobalSignal<bool> = Signal::global(|| false);
/// Set after the first successful profiler config fetch (gates auto flamegraph load).
pub static PROFILING_CONFIG_LOADED: GlobalSignal<bool> = Signal::global(|| false);

/// Normalize URL slug to a canonical profiling view.
pub fn normalize_profiling_view(view: &str) -> &'static str {
    let view = view.trim().trim_matches('/');
    match view {
        "" | "pprof" => "pprof",
        "torch" => "torch",
        "torch-dist" | "distributed-torch" | "torch-distributed" => "torch-dist",
        "trace" | "trace-timeline" => "trace",
        "pytorch" | "pytorch-timeline" => "pytorch",
        "ray" | "ray-timeline" => "ray",
        _ => "pprof",
    }
}

pub fn profiling_view_label(view: &str) -> &'static str {
    profiling_view_spec(view).label
}

/// Apply server `df_settings` rows to global profiling UI state.
pub fn apply_profiler_config(config: &[(String, String)]) {
    *PROFILING_PPROF_FREQ.write() = 0;
    *PROFILING_TORCH_ENABLED.write() = false;

    for (name, value) in config {
        match name.as_str() {
            "probing.pprof.sample_freq" => {
                if let Ok(v) = value.parse::<i32>() {
                    *PROFILING_PPROF_FREQ.write() = v.max(0);
                }
            }
            "probing.torch.profiling" => {
                let lowered = value.trim().to_lowercase();
                let disabled_values = ["", "0", "false", "off", "disable", "disabled"];
                let enabled = !disabled_values.contains(&lowered.as_str());
                *PROFILING_TORCH_ENABLED.write() = enabled;
            }
            _ => {}
        }
    }
    *PROFILING_CONFIG_LOADED.write() = true;
}

pub static PROFILING_CHROME_LIMIT: GlobalSignal<usize> = Signal::global(|| 1000);
/// Row cap for the Spans page tree (`python.trace_event`); independent of Profiling chrome trace.
pub static SPANS_TREE_LIMIT: GlobalSignal<usize> = Signal::global(|| 1000);
pub static PROFILING_PYTORCH_STEPS: GlobalSignal<i32> = Signal::global(|| 5);
pub static PROFILING_PYTORCH_TIMELINE_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);
pub static PROFILING_RAY_TIMELINE_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);
pub static PROFILING_TRACE_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);
/// Optional ``local_step`` for distributed torch flamegraph (``None`` = latest on server).
pub static PROFILING_DIST_STEP: GlobalSignal<Option<i64>> = Signal::global(|| None);
pub static PROFILING_DIST_CLUSTER: GlobalSignal<bool> = Signal::global(|| true);
pub static PROFILING_DIST_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);

#[derive(Clone, Debug, PartialEq)]
pub struct ProfilingFeedback {
    pub message: String,
    pub is_error: bool,
}

pub static PROFILING_FEEDBACK: GlobalSignal<Option<ProfilingFeedback>> = Signal::global(|| None);

pub fn show_profiling_feedback(message: impl Into<String>, is_error: bool) {
    *PROFILING_FEEDBACK.write() = Some(ProfilingFeedback {
        message: message.into(),
        is_error,
    });
    spawn(async {
        gloo_timers::future::TimeoutFuture::new(4_500).await;
        if PROFILING_FEEDBACK.read().is_some() {
            clear_profiling_feedback();
        }
    });
}

pub fn clear_profiling_feedback() {
    *PROFILING_FEEDBACK.write() = None;
}

#[derive(Clone, Copy, Debug)]
pub struct ProfilingViewSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub sidebar_label: &'static str,
    pub tooltip: &'static str,
}

pub const PROFILING_VIEWS: &[ProfilingViewSpec] = &[
    ProfilingViewSpec {
        id: "pprof",
        label: "CPU sampling",
        sidebar_label: "CPU (pprof)",
        tooltip: "SIGPROF stack sampling · statistical flamegraph",
    },
    ProfilingViewSpec {
        id: "torch",
        label: "Torch modules",
        sidebar_label: "Torch flamegraph",
        tooltip: "PyTorch module hook durations · statistical flamegraph (not the profiler timeline)",
    },
    ProfilingViewSpec {
        id: "torch-dist",
        label: "Distributed torch",
        sidebar_label: "Distributed flamegraph",
        tooltip: "SPMD snapshot at one training step — merge identical module stacks across ranks",
    },
    ProfilingViewSpec {
        id: "trace",
        label: "Chrome trace",
        sidebar_label: "Chrome trace",
        tooltip: "Chrome trace event timeline from probing trace buffers (not distributed spans on the Spans page)",
    },
    ProfilingViewSpec {
        id: "pytorch",
        label: "PyTorch profiler",
        sidebar_label: "PyTorch timeline",
        tooltip: "PyTorch profiler chrome trace export",
    },
    ProfilingViewSpec {
        id: "ray",
        label: "Ray timeline",
        sidebar_label: "Ray timeline",
        tooltip: "Ray task and actor timeline",
    },
];

pub fn profiling_view_spec(view: &str) -> &'static ProfilingViewSpec {
    let id = normalize_profiling_view(view);
    PROFILING_VIEWS
        .iter()
        .find(|v| v.id == id)
        .unwrap_or(&PROFILING_VIEWS[0])
}
