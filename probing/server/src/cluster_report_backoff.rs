//! Exponential backoff for cluster heartbeats when the view is stable.

use std::time::Duration;

use probing_proto::prelude::{Node, NodeReportResponse};

use crate::cluster_http::get_i32_env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportOutcome {
    /// Discovery or parent not ready — do not penalize streak.
    Skipped,
    /// PUT failed.
    Failed,
    /// PUT ok; cluster/local view still converging — hold interval at base.
    SuccessConverging,
    /// PUT ok and expected membership present — advance backoff streak.
    SuccessStable,
}

#[derive(Debug, Clone)]
pub struct ReportBackoff {
    streak: u32,
    base_secs: u64,
    max_secs: u64,
    factor: f64,
}

pub fn report_backoff_enabled() -> bool {
    match std::env::var("PROBING_CLUSTER_REPORT_BACKOFF") {
        Ok(val) => {
            let lower = val.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => true,
    }
}

pub fn base_report_interval_secs() -> u64 {
    std::env::var("PROBING_CLUSTER_REPORT_INTERVAL_SEC")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(10)
        .max(1)
}

pub fn stale_threshold_secs() -> u64 {
    std::env::var("PROBING_CLUSTER_STALE_SEC")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(25)
        .max(5)
}

fn configured_max_interval_secs() -> u64 {
    std::env::var("PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(120)
        .max(1)
}

fn backoff_factor() -> f64 {
    std::env::var("PROBING_CLUSTER_REPORT_BACKOFF_FACTOR")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(2.0_f64)
        .max(1.0_f64)
}

/// Cap interval below stale TTL so nodes are not marked dead between beats.
pub fn max_report_interval_secs() -> u64 {
    let base = base_report_interval_secs();
    let configured = configured_max_interval_secs();
    let stale = stale_threshold_secs();
    let safe = stale.saturating_sub(stale / 4 + 1).max(base);
    configured.min(safe).max(base)
}

impl ReportBackoff {
    pub fn new() -> Self {
        Self {
            streak: 0,
            base_secs: base_report_interval_secs(),
            max_secs: max_report_interval_secs(),
            factor: backoff_factor(),
        }
    }

    pub fn reset(&mut self) {
        self.streak = 0;
    }

    pub fn record(&mut self, outcome: ReportOutcome) {
        if !report_backoff_enabled() {
            self.streak = 0;
            return;
        }
        match outcome {
            ReportOutcome::Skipped => {}
            ReportOutcome::Failed | ReportOutcome::SuccessConverging => self.streak = 0,
            ReportOutcome::SuccessStable => self.streak = self.streak.saturating_add(1),
        }
    }

    pub fn sleep_duration(&self) -> Duration {
        if !report_backoff_enabled() {
            return Duration::from_secs(self.base_secs);
        }
        let mult = self.factor.powi(self.streak as i32);
        let secs = ((self.base_secs as f64) * mult)
            .min(self.max_secs as f64)
            .max(self.base_secs as f64) as u64;
        Duration::from_secs(secs)
    }
}

impl Default for ReportBackoff {
    fn default() -> Self {
        Self::new()
    }
}

fn node_is_alive(node: &Node) -> bool {
    node.status.as_deref() != Some("dead")
}

fn local_world_size() -> Option<i32> {
    get_i32_env("LOCAL_WORLD_SIZE")
        .or_else(|| get_i32_env("LOCAL_SIZE"))
        .or_else(|| get_i32_env("NPROC_PER_NODE"))
}

fn node_rank_from_env() -> i32 {
    get_i32_env("GROUP_RANK")
        .or_else(|| get_i32_env("NODE_RANK"))
        .unwrap_or(0)
}

fn local_group_alive_count(nodes: &[Node]) -> usize {
    let grp = node_rank_from_env();
    nodes
        .iter()
        .filter(|n| n.group_rank == Some(grp) && node_is_alive(n))
        .count()
}

fn global_alive_count(nodes: &[Node]) -> usize {
    nodes.iter().filter(|n| node_is_alive(n)).count()
}

/// Classify whether the merged snapshot indicates stable membership for backoff.
pub fn classify_report_outcome(
    put_ok: bool,
    discovered: bool,
    resp: Option<&NodeReportResponse>,
) -> ReportOutcome {
    if !discovered {
        return ReportOutcome::Skipped;
    }
    if !put_ok {
        return ReportOutcome::Failed;
    }
    let Some(resp) = resp else {
        return ReportOutcome::SuccessConverging;
    };

    let local_rank = get_i32_env("LOCAL_RANK").unwrap_or(0);
    if local_rank != 0 {
        if let Some(lws) = local_world_size() {
            if lws > 0 && local_group_alive_count(&resp.nodes) >= lws as usize {
                return ReportOutcome::SuccessStable;
            }
        }
        return ReportOutcome::SuccessConverging;
    }

    if let Some(ws) = get_i32_env("WORLD_SIZE") {
        if ws > 1 && global_alive_count(&resp.nodes) >= ws as usize {
            return ReportOutcome::SuccessStable;
        }
    }

    if let Some(lws) = local_world_size() {
        if lws > 0 && local_group_alive_count(&resp.nodes) >= lws as usize {
            return ReportOutcome::SuccessStable;
        }
    }

    ReportOutcome::SuccessConverging
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn clear_backoff_env() {
        for key in [
            "PROBING_CLUSTER_REPORT_INTERVAL_SEC",
            "PROBING_CLUSTER_STALE_SEC",
            "PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC",
            "PROBING_CLUSTER_REPORT_BACKOFF_FACTOR",
            "PROBING_CLUSTER_REPORT_BACKOFF",
        ] {
            std::env::remove_var(key);
        }
    }

    fn with_env<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_backoff_env();
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        f();
        clear_backoff_env();
    }

    #[test]
    fn interval_doubles_when_stable() {
        with_env(
            &[
                ("PROBING_CLUSTER_REPORT_INTERVAL_SEC", "10"),
                ("PROBING_CLUSTER_STALE_SEC", "90"),
                ("PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC", "120"),
            ],
            || {
                let mut b = ReportBackoff::new();
                assert_eq!(b.sleep_duration(), Duration::from_secs(10));
                b.record(ReportOutcome::SuccessStable);
                assert_eq!(b.sleep_duration(), Duration::from_secs(20));
                b.record(ReportOutcome::SuccessStable);
                assert_eq!(b.sleep_duration(), Duration::from_secs(40));
            },
        );
    }

    #[test]
    fn failure_resets_streak() {
        with_env(&[("PROBING_CLUSTER_STALE_SEC", "90")], || {
            let mut b = ReportBackoff::new();
            b.record(ReportOutcome::SuccessStable);
            b.record(ReportOutcome::SuccessStable);
            b.record(ReportOutcome::Failed);
            assert_eq!(b.sleep_duration(), Duration::from_secs(10));
        });
    }

    #[test]
    fn max_respects_stale_ttl() {
        with_env(
            &[
                ("PROBING_CLUSTER_STALE_SEC", "25"),
                ("PROBING_CLUSTER_REPORT_MAX_INTERVAL_SEC", "120"),
            ],
            || {
                assert!(max_report_interval_secs() <= 25);
            },
        );
    }
}
