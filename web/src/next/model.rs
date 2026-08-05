//! Pure view models for the next diagnostics UI.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::api::{GpuSnapshot, StepDurationSample, StepMatrixResponse};
use probing_proto::prelude::Node;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistryHealth {
    pub processes: usize,
    pub hosts: usize,
    pub observed_ranks: usize,
    pub expected_ranks: Option<usize>,
    pub world_sizes: Vec<i32>,
}

impl RegistryHealth {
    pub fn from_nodes(nodes: &[Node]) -> Self {
        let hosts = nodes
            .iter()
            .map(|node| node.host.as_str())
            .filter(|host| !host.trim().is_empty())
            .collect::<BTreeSet<_>>()
            .len();
        let observed_ranks = nodes
            .iter()
            .filter_map(|node| node.rank)
            .collect::<BTreeSet<_>>()
            .len();
        let world_sizes = nodes
            .iter()
            .filter_map(|node| node.world_size)
            .filter(|world_size| *world_size > 0)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let expected_ranks = if world_sizes.len() == 1 {
            Some(world_sizes[0] as usize)
        } else {
            None
        };

        Self {
            processes: nodes.len(),
            hosts,
            observed_ranks,
            expected_ranks,
            world_sizes,
        }
    }

    pub fn rank_coverage(&self) -> String {
        self.expected_ranks
            .map(|expected| format!("{} / {expected}", self.observed_ranks))
            .unwrap_or_else(|| self.observed_ranks.to_string())
    }

    pub fn world_size_label(&self) -> String {
        if self.world_sizes.is_empty() {
            "—".to_string()
        } else {
            self.world_sizes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepHealth {
    pub latest_step: Option<i64>,
    pub median_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub slowest_rank: Option<i32>,
    pub slowest_ms: Option<f64>,
    pub observed_ranks: usize,
    pub expected_ranks: usize,
    pub nodes_failed: Vec<String>,
    pub rank_durations: Vec<(i32, f64)>,
    pub trend: Vec<StepTrendPoint>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepTrendPoint {
    pub step: i64,
    pub median_ms: f64,
    pub p95_ms: f64,
}

impl StepHealth {
    pub fn from_matrix(matrix: &StepMatrixResponse) -> Self {
        let mut latest_by_rank: HashMap<i32, &StepDurationSample> = HashMap::new();
        for sample in &matrix.samples {
            let coordinate = sample_coordinate(sample);
            latest_by_rank
                .entry(sample.rank)
                .and_modify(|current| {
                    if coordinate > sample_coordinate(current) {
                        *current = sample;
                    }
                })
                .or_insert(sample);
        }

        let mut durations = latest_by_rank
            .values()
            .map(|sample| sample.duration_ms)
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .collect::<Vec<_>>();
        durations.sort_by(f64::total_cmp);

        let median_ms = percentile(&durations, 0.5);
        let p95_ms = percentile(&durations, 0.95);
        let slowest = latest_by_rank
            .values()
            .filter(|sample| sample.duration_ms.is_finite())
            .max_by(|a, b| a.duration_ms.total_cmp(&b.duration_ms));

        let mut rank_durations = latest_by_rank
            .values()
            .filter(|sample| sample.duration_ms.is_finite())
            .map(|sample| (sample.rank, sample.duration_ms))
            .collect::<Vec<_>>();
        rank_durations.sort_by(|a, b| b.1.total_cmp(&a.1));

        let mut durations_by_step: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
        for sample in &matrix.samples {
            if sample.duration_ms.is_finite() && sample.duration_ms >= 0.0 {
                durations_by_step
                    .entry(sample_coordinate(sample))
                    .or_default()
                    .push(sample.duration_ms);
            }
        }
        let mut trend = durations_by_step
            .into_iter()
            .filter_map(|(step, mut durations)| {
                durations.sort_by(f64::total_cmp);
                Some(StepTrendPoint {
                    step,
                    median_ms: percentile(&durations, 0.5)?,
                    p95_ms: percentile(&durations, 0.95)?,
                })
            })
            .collect::<Vec<_>>();
        if trend.len() > 40 {
            trend = trend.split_off(trend.len() - 40);
        }

        Self {
            latest_step: latest_by_rank
                .values()
                .map(|sample| sample_coordinate(sample))
                .max(),
            median_ms,
            p95_ms,
            slowest_rank: slowest.map(|sample| sample.rank),
            slowest_ms: slowest.map(|sample| sample.duration_ms),
            observed_ranks: latest_by_rank.len(),
            expected_ranks: matrix.rank_count.max(latest_by_rank.len()),
            nodes_failed: matrix.nodes_failed.clone(),
            rank_durations,
            trend,
        }
    }

    pub fn slowest_ratio(&self) -> Option<f64> {
        let median = self.median_ms?;
        let slowest = self.slowest_ms?;
        (median > 0.0).then_some(slowest / median)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuHealth {
    pub device_count: usize,
    pub average_util_pct: Option<f64>,
    pub average_memory_pct: Option<f64>,
}

impl GpuHealth {
    pub fn from_snapshots(snapshots: &[GpuSnapshot]) -> Self {
        let util = snapshots
            .iter()
            .filter_map(|snapshot| snapshot.gpu_util_pct.map(f64::from))
            .collect::<Vec<_>>();
        let memory = snapshots
            .iter()
            .map(|snapshot| f64::from(snapshot.mem_used_pct))
            .collect::<Vec<_>>();
        Self {
            device_count: snapshots.len(),
            average_util_pct: mean(&util),
            average_memory_pct: mean(&memory),
        }
    }
}

fn sample_coordinate(sample: &StepDurationSample) -> i64 {
    if sample.coord_step != 0 {
        sample.coord_step
    } else {
        sample.local_step
    }
}

fn percentile(sorted: &[f64], percentile: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = ((sorted.len() as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted.get(index).copied()
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

pub fn format_duration(ms: Option<f64>) -> String {
    match ms {
        Some(value) if value >= 1000.0 => format!("{:.2} s", value / 1000.0),
        Some(value) => format!("{value:.1} ms"),
        None => "—".to_string(),
    }
}

pub fn format_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_node(host: &str, rank: Option<i32>, world_size: Option<i32>) -> Node {
        Node {
            host: host.to_string(),
            rank,
            world_size,
            ..Default::default()
        }
    }

    #[test]
    fn registry_health_distinguishes_hosts_processes_and_ranks() {
        let nodes = vec![
            registry_node("node-a", Some(0), Some(4)),
            registry_node("node-a", Some(1), Some(4)),
            registry_node("node-b", Some(2), Some(4)),
            registry_node("node-b", Some(3), Some(4)),
        ];
        let health = RegistryHealth::from_nodes(&nodes);
        assert_eq!(health.hosts, 2);
        assert_eq!(health.processes, 4);
        assert_eq!(health.rank_coverage(), "4 / 4");
    }

    #[test]
    fn registry_health_does_not_index_missing_world_size() {
        let nodes = vec![registry_node("node-a", Some(0), None)];
        let health = RegistryHealth::from_nodes(&nodes);
        assert_eq!(health.expected_ranks, None);
        assert_eq!(health.rank_coverage(), "1");
        assert_eq!(health.world_size_label(), "—");
    }

    fn sample(rank: i32, step: i64, duration_ms: f64) -> StepDurationSample {
        StepDurationSample {
            rank,
            local_step: step,
            coord_step: step,
            duration_ms,
            host: format!("node-{rank}"),
            addr: String::new(),
        }
    }

    #[test]
    fn step_health_uses_latest_sample_per_rank() {
        let matrix = StepMatrixResponse {
            samples: vec![
                sample(0, 10, 100.0),
                sample(0, 11, 120.0),
                sample(1, 11, 180.0),
                sample(2, 11, 140.0),
            ],
            rank_count: 4,
            step_count: 2,
            cluster: true,
            partial: true,
            nodes_queried: 3,
            nodes_failed: vec!["node-4".into()],
        };
        let health = StepHealth::from_matrix(&matrix);
        assert_eq!(health.latest_step, Some(11));
        assert_eq!(health.median_ms, Some(140.0));
        assert_eq!(health.p95_ms, Some(180.0));
        assert_eq!(health.slowest_rank, Some(1));
        assert_eq!(health.observed_ranks, 3);
        assert_eq!(health.expected_ranks, 4);
        assert_eq!(health.rank_durations[0], (1, 180.0));
        assert_eq!(
            health.trend,
            vec![
                StepTrendPoint {
                    step: 10,
                    median_ms: 100.0,
                    p95_ms: 100.0,
                },
                StepTrendPoint {
                    step: 11,
                    median_ms: 140.0,
                    p95_ms: 180.0,
                },
            ]
        );
    }

    #[test]
    fn gpu_health_ignores_unknown_utilization() {
        let snapshots = vec![
            GpuSnapshot {
                gpu_util_pct: Some(50.0),
                mem_used_pct: 25.0,
                ..Default::default()
            },
            GpuSnapshot {
                gpu_util_pct: None,
                mem_used_pct: 75.0,
                ..Default::default()
            },
        ];
        let health = GpuHealth::from_snapshots(&snapshots);
        assert_eq!(health.device_count, 2);
        assert_eq!(health.average_util_pct, Some(50.0));
        assert_eq!(health.average_memory_pct, Some(50.0));
    }
}
