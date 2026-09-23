//! Shared RL run selection helpers for Overview / Metrics / Samples pages.

use probing_proto::prelude::RlRunSummary;

const STORAGE_KEY: &str = "probing.rl.selected_run_id";

pub fn read_selected_run_id() -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(STORAGE_KEY)
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
}

pub fn write_selected_run_id(run_id: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(Some(storage)) = window.local_storage() else {
        return;
    };
    let _ = storage.set_item(STORAGE_KEY, run_id);
}

pub fn resolve_run<'a>(
    runs: &'a [RlRunSummary],
    preferred: Option<&str>,
) -> Option<&'a RlRunSummary> {
    if let Some(preferred) = preferred.filter(|value| !value.trim().is_empty()) {
        if let Some(run) = runs.iter().find(|run| run.run_id == preferred) {
            return Some(run);
        }
    }
    runs.first()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str) -> RlRunSummary {
        RlRunSummary {
            run_id: id.into(),
            framework: "demo".into(),
            phase: "training".into(),
            global_step: 1,
            timestamp_ns: 1,
            start_time_ns: 1,
            end_time_ns: 0,
            samples_total: 0,
            tokens_total: 0,
            job_id: String::new(),
            config_hash: String::new(),
            metadata_json: String::new(),
        }
    }

    #[test]
    fn prefers_matching_run_id() {
        let runs = vec![run("a"), run("b")];
        assert_eq!(resolve_run(&runs, Some("b")).unwrap().run_id, "b");
    }

    #[test]
    fn falls_back_to_first_run() {
        let runs = vec![run("a"), run("b")];
        assert_eq!(resolve_run(&runs, Some("missing")).unwrap().run_id, "a");
    }
}
