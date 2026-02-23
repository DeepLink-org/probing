use dioxus::prelude::*;

pub static PROFILING_VIEW: GlobalSignal<String> = Signal::global(|| "pprof".to_string());
pub static PROFILING_PPROF_FREQ: GlobalSignal<i32> = Signal::global(|| 99);
pub static PROFILING_TORCH_ENABLED: GlobalSignal<bool> = Signal::global(|| false);
#[allow(dead_code)]
pub static PROFILING_CHROME_DATA_SOURCE: GlobalSignal<String> =
    Signal::global(|| "trace".to_string());
pub static PROFILING_CHROME_LIMIT: GlobalSignal<usize> = Signal::global(|| 1000);
pub static PROFILING_PYTORCH_STEPS: GlobalSignal<i32> = Signal::global(|| 5);
pub static PROFILING_PYTORCH_TIMELINE_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);
pub static PROFILING_RAY_TIMELINE_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);
