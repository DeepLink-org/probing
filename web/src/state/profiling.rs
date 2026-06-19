use dioxus::prelude::*;

pub static PROFILING_VIEW: GlobalSignal<String> = Signal::global(|| "pprof".to_string());
/// Server-aligned default: profiling off until `get_profiler_config` runs.
pub static PROFILING_PPROF_FREQ: GlobalSignal<i32> = Signal::global(|| 0);
pub static PROFILING_TORCH_ENABLED: GlobalSignal<bool> = Signal::global(|| false);
/// Set after the first successful profiler config fetch (gates auto flamegraph load).
pub static PROFILING_CONFIG_LOADED: GlobalSignal<bool> = Signal::global(|| false);

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
#[allow(dead_code)]
pub static PROFILING_CHROME_DATA_SOURCE: GlobalSignal<String> =
    Signal::global(|| "trace".to_string());
pub static PROFILING_CHROME_LIMIT: GlobalSignal<usize> = Signal::global(|| 1000);
pub static PROFILING_PYTORCH_STEPS: GlobalSignal<i32> = Signal::global(|| 5);
pub static PROFILING_PYTORCH_TIMELINE_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);
pub static PROFILING_RAY_TIMELINE_RELOAD: GlobalSignal<i32> = Signal::global(|| 0);
