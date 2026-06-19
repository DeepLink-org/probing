//! Map playbook `ui.view` targets to app routes (shared with sidebar navigation).

use crate::app::Route;
use crate::state::profiling::{normalize_profiling_view, profiling_view_label};

pub fn agent_view_to_route(view: &str) -> Route {
    let v = view.trim().trim_start_matches("profiling/");
    match v {
        "analytics" => Route::AnalyticsPage {},
        "pprof" => Route::ProfilingViewPage {
            view: "pprof".to_string(),
        },
        "torch" => Route::ProfilingViewPage {
            view: "torch".to_string(),
        },
        "traces" | "spans" => Route::SpansPage {},
        "trace" | "chrome-trace" => Route::ProfilingViewPage {
            view: "trace".to_string(),
        },
        "python" => Route::PythonPage {},
        "training" => Route::TrainingPage {},
        "cluster" => Route::ClusterPage {},
        other if other.starts_with("profiling/") => Route::ProfilingViewPage {
            view: other.replace("profiling/", ""),
        },
        other => Route::ProfilingViewPage {
            view: normalize_profiling_view(other).to_string(),
        },
    }
}

pub fn agent_view_label(view: &str) -> String {
    let v = view.trim().trim_start_matches("profiling/");
    match v {
        "analytics" => "Analytics".to_string(),
        "traces" | "spans" => "Spans".to_string(),
        "trace" | "chrome-trace" => "Chrome trace".to_string(),
        "python" => "Python".to_string(),
        "training" => "Training".to_string(),
        "cluster" => "Cluster".to_string(),
        other => profiling_view_label(other).to_string(),
    }
}
