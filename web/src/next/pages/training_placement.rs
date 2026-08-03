use std::collections::{BTreeMap, BTreeSet};

use dioxus::prelude::*;
use probing_proto::prelude::Node;

use crate::state::investigation::{set_training_rank_context, INVESTIGATION_CONTEXT};

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlacementProcess {
    rank: Option<i32>,
    local_rank: Option<i32>,
    role_label: Option<String>,
    coordinates: Vec<(String, String)>,
    status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlacementHost {
    name: String,
    processes: Vec<PlacementProcess>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PlacementModel {
    hosts: Vec<PlacementHost>,
    observed_ranks: usize,
    expected_ranks: usize,
    has_parallel_coordinates: bool,
    parallel_sizes: Vec<(String, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlacementGroup {
    Focus,
    Tensor,
    Data,
    Pipeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlacementGroupSizes {
    tensor: usize,
    data: usize,
    pipeline: usize,
}

#[component]
pub(super) fn TrainingPlacement(nodes: Vec<Node>, local_step: Option<i64>) -> Element {
    let placement = build_placement(&nodes);
    rsx! { PlacementDiagram { placement, local_step } }
}

#[component]
fn PlacementDiagram(placement: PlacementModel, local_step: Option<i64>) -> Element {
    let missing_ranks = placement
        .expected_ranks
        .saturating_sub(placement.observed_ranks);
    rsx! {
        div { class: "space-y-3",
            div { class: "flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-gray-600",
                span { class: "font-medium text-gray-900", "{placement.hosts.len()} hosts" }
                span { "{placement.observed_ranks} / {placement.expected_ranks} ranks" }
                if placement.has_parallel_coordinates {
                    for (dimension, size) in placement.parallel_sizes.iter() {
                        span { class: "font-mono font-semibold uppercase text-violet-700",
                            "{dimension}{size}"
                        }
                    }
                } else if placement.expected_ranks > 1 {
                    span { class: "text-gray-500", "parallel coordinates not reported" }
                }
                if missing_ranks > 0 {
                    span { class: "rounded-full bg-amber-100 px-2 py-0.5 font-medium text-amber-800",
                        "{missing_ranks} missing"
                    }
                }
            }
            PlacementOverview { placement, local_step }
        }
    }
}

#[component]
fn PlacementOverview(placement: PlacementModel, local_step: Option<i64>) -> Element {
    let mut hovered_rank = use_signal(|| None::<i32>);
    let pinned_rank = INVESTIGATION_CONTEXT.read().rank;
    let active_rank = hovered_rank().or(pinned_rank);
    let active_selection = active_rank.and_then(|rank| {
        placement.hosts.iter().find_map(|host| {
            host.processes
                .iter()
                .find(|process| process.rank == Some(rank))
                .map(|process| (host.name.clone(), process.clone()))
        })
    });
    let active_process = active_selection
        .as_ref()
        .map(|(_, process)| process.clone());
    let group_sizes = active_process
        .as_ref()
        .and_then(|active| placement_group_sizes(&placement, active));
    let host_columns = placement.hosts.len().clamp(1, 8);

    rsx! {
        div {
            class: "rounded-md border border-gray-200 bg-gray-50 px-3 py-2.5",
            onmouseleave: move |_| hovered_rank.set(None),
            div { class: "mb-2 flex flex-wrap items-center justify-between gap-2",
                div { class: "flex items-center gap-2",
                    span { class: "text-xs font-medium uppercase tracking-wide text-gray-500", "Overview" }
                    if let Some(rank) = active_rank {
                        span { class: "font-mono text-xs font-semibold text-blue-700", "R{rank}" }
                        if hovered_rank().is_none() {
                            span { class: "text-xs text-blue-600", "pinned" }
                        }
                    }
                }
                div { class: "flex items-center gap-3 text-xs text-gray-500",
                    GroupLegend { label: "TP", count: group_sizes.map(|sizes| sizes.tensor), class: "border-violet-500 bg-violet-100" }
                    GroupLegend { label: "DP", count: group_sizes.map(|sizes| sizes.data), class: "border-emerald-500 bg-emerald-100" }
                    GroupLegend { label: "PP", count: group_sizes.map(|sizes| sizes.pipeline), class: "border-amber-500 bg-amber-100" }
                    span { class: "text-gray-600", "Focus or hover to preview · click to pin" }
                }
            }
            if let Some((host, process)) = active_selection.as_ref() {
                PlacementSelectionDetail {
                    host: host.clone(),
                    process: process.clone(),
                    group_sizes,
                    pinned: hovered_rank().is_none(),
                }
            }
            div { class: "overflow-x-auto pb-0.5",
                div {
                    class: "grid min-w-max gap-2",
                    style: "grid-template-columns: repeat({host_columns}, 34px);",
                    for (host_index, host) in placement.hosts.iter().enumerate() {
                        div {
                            class: "rounded border border-gray-200 bg-white p-1",
                            aria_label: "Host {host_index}: {host.name}",
                            div { class: "mb-1 truncate text-center font-mono text-xs text-gray-600", "H{host_index}" }
                            div { class: "grid grid-cols-1 justify-items-center gap-0.5",
                                for process in host.processes.iter() {
                                    PlacementCell {
                                        host: host.name.clone(),
                                        process: process.clone(),
                                        active: active_process.clone(),
                                        hovered_rank,
                                        local_step,
                                    }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "mt-2 flex flex-wrap gap-x-3 gap-y-1 border-t border-gray-200 pt-2 text-xs text-gray-600",
                for (host_index, host) in placement.hosts.iter().enumerate() {
                    span { class: "font-mono", "H{host_index} {host.name}" }
                }
            }
        }
    }
}

#[component]
fn PlacementSelectionDetail(
    host: String,
    process: PlacementProcess,
    group_sizes: Option<PlacementGroupSizes>,
    pinned: bool,
) -> Element {
    let rank = process
        .rank
        .map(|value| format!("rank {value}"))
        .unwrap_or_else(|| "rank unknown".to_string());
    let local_rank = process
        .local_rank
        .map(|value| format!("GPU {value}"))
        .unwrap_or_else(|| "GPU unknown".to_string());
    let status = process
        .status
        .unwrap_or_else(|| "unknown status".to_string());
    let role = process
        .role_label
        .unwrap_or_else(|| "role unknown".to_string());
    let coordinates = process
        .coordinates
        .iter()
        .map(|(dimension, value)| format!("{dimension}={value}"))
        .collect::<Vec<_>>()
        .join(" · ");

    rsx! {
        div {
            class: "mb-3 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-blue-200 bg-white px-3 py-2 text-xs text-gray-700",
            aria_live: "polite",
            span { class: "font-semibold text-gray-950", "{rank}" }
            span { "{host}" }
            span { "{local_rank}" }
            span { "{status}" }
            span { class: "font-mono", "{role}" }
            if !coordinates.is_empty() {
                span { class: "font-mono", "{coordinates}" }
            }
            if let Some(sizes) = group_sizes {
                span { class: "font-medium text-violet-800", "TP group {sizes.tensor}" }
                span { class: "font-medium text-emerald-800", "DP group {sizes.data}" }
                span { class: "font-medium text-amber-800", "PP group {sizes.pipeline}" }
            }
            span { class: "ml-auto font-medium text-blue-700", if pinned { "Pinned" } else { "Preview" } }
        }
    }
}

#[component]
fn PlacementCell(
    host: String,
    process: PlacementProcess,
    active: Option<PlacementProcess>,
    mut hovered_rank: Signal<Option<i32>>,
    local_step: Option<i64>,
) -> Element {
    let rank = process.rank;
    let rank_label = rank
        .map(|value| format!("R{value}"))
        .unwrap_or_else(|| "R?".to_string());
    let local_rank = process
        .local_rank
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    let status = process.status.as_deref().unwrap_or("unknown");
    let role = process.role_label.as_deref().unwrap_or("rank");
    let coordinates = process
        .coordinates
        .iter()
        .map(|(dimension, value)| {
            format!(
                "{}{}",
                dimension.chars().next().unwrap_or('?').to_ascii_uppercase(),
                value
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let group = active
        .as_ref()
        .and_then(|active| placement_group_membership(active, &process));
    let cell_class = cell_classes(group, process.status.as_deref());
    let group_name = placement_group_name(group);
    let coordinate_detail = if coordinates.is_empty() {
        String::new()
    } else {
        format!(" · {coordinates}")
    };
    let title = format!(
        "{rank_label} · {host} · GPU{local_rank} · {status} · {role}{}{}",
        coordinate_detail,
        group_name
            .map(|name| format!(" · {name}"))
            .unwrap_or_default(),
    );
    let pinned_host = host.clone();
    let pinned = INVESTIGATION_CONTEXT.read().rank == rank;
    let cell_text = placement_group_code(group).unwrap_or(local_rank.as_str());

    rsx! {
        button {
            r#type: "button",
            class: "flex h-6 w-6 items-center justify-center rounded-[3px] border font-mono text-xs font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-700 focus-visible:ring-offset-1 {cell_class}",
            aria_label: "{title}",
            aria_pressed: pinned.to_string(),
            title: "{title}",
            onmouseover: move |_| hovered_rank.set(rank),
            onfocus: move |_| hovered_rank.set(rank),
            onclick: move |_| {
                hovered_rank.set(rank);
                if let Some(rank) = rank {
                    set_training_rank_context(rank, local_step, Some(&pinned_host));
                }
            },
            onblur: move |_| hovered_rank.set(None),
            "{cell_text}"
        }
    }
}

fn placement_group_code(group: Option<PlacementGroup>) -> Option<&'static str> {
    match group {
        Some(PlacementGroup::Focus) => Some("●"),
        Some(PlacementGroup::Tensor) => Some("T"),
        Some(PlacementGroup::Data) => Some("D"),
        Some(PlacementGroup::Pipeline) => Some("P"),
        None => None,
    }
}

fn placement_group_name(group: Option<PlacementGroup>) -> Option<&'static str> {
    match group {
        Some(PlacementGroup::Focus) => Some("selected GPU"),
        Some(PlacementGroup::Tensor) => Some("tensor-parallel peer"),
        Some(PlacementGroup::Data) => Some("data-parallel peer"),
        Some(PlacementGroup::Pipeline) => Some("pipeline-parallel peer"),
        None => None,
    }
}

#[component]
fn GroupLegend(label: &'static str, count: Option<usize>, class: &'static str) -> Element {
    rsx! {
        span { class: "flex items-center gap-1",
            span { class: "h-3 w-3 rounded-[2px] border border-dashed {class}" }
            "{label}"
            if let Some(count) = count {
                span { class: "font-mono font-semibold text-gray-700", "{count}" }
            }
        }
    }
}

fn coordinate<'a>(process: &'a PlacementProcess, dimension: &str) -> Option<&'a str> {
    process
        .coordinates
        .iter()
        .find_map(|(key, value)| (key == dimension).then_some(value.as_str()))
}

fn placement_group_membership(
    active: &PlacementProcess,
    candidate: &PlacementProcess,
) -> Option<PlacementGroup> {
    if active.rank == candidate.rank {
        return Some(PlacementGroup::Focus);
    }

    let active_dp = coordinate(active, "dp")?;
    let active_pp = coordinate(active, "pp")?;
    let active_tp = coordinate(active, "tp")?;
    let candidate_dp = coordinate(candidate, "dp")?;
    let candidate_pp = coordinate(candidate, "pp")?;
    let candidate_tp = coordinate(candidate, "tp")?;

    if active_dp == candidate_dp && active_pp == candidate_pp {
        Some(PlacementGroup::Tensor)
    } else if active_pp == candidate_pp && active_tp == candidate_tp {
        Some(PlacementGroup::Data)
    } else if active_dp == candidate_dp && active_tp == candidate_tp {
        Some(PlacementGroup::Pipeline)
    } else {
        None
    }
}

fn placement_group_sizes(
    placement: &PlacementModel,
    active: &PlacementProcess,
) -> Option<PlacementGroupSizes> {
    coordinate(active, "dp")?;
    coordinate(active, "pp")?;
    coordinate(active, "tp")?;

    let mut sizes = PlacementGroupSizes {
        tensor: 1,
        data: 1,
        pipeline: 1,
    };
    for candidate in placement
        .hosts
        .iter()
        .flat_map(|host| host.processes.iter())
    {
        match placement_group_membership(active, candidate) {
            Some(PlacementGroup::Tensor) => sizes.tensor += 1,
            Some(PlacementGroup::Data) => sizes.data += 1,
            Some(PlacementGroup::Pipeline) => sizes.pipeline += 1,
            Some(PlacementGroup::Focus) | None => {}
        }
    }
    Some(sizes)
}

fn cell_classes(group: Option<PlacementGroup>, status: Option<&str>) -> &'static str {
    match group {
        Some(PlacementGroup::Focus) => {
            "border-blue-700 bg-blue-600 text-white ring-2 ring-blue-200"
        }
        Some(PlacementGroup::Tensor) => {
            "border-dashed border-violet-500 bg-violet-100 text-violet-900"
        }
        Some(PlacementGroup::Data) => {
            "border-dashed border-emerald-500 bg-emerald-100 text-emerald-900"
        }
        Some(PlacementGroup::Pipeline) => {
            "border-dashed border-amber-500 bg-amber-100 text-amber-900"
        }
        None if matches!(
            status.unwrap_or_default().to_ascii_lowercase().as_str(),
            "failed" | "error" | "offline" | "unhealthy"
        ) =>
        {
            "border-red-400 bg-red-100 text-red-800"
        }
        None => "border-gray-300 bg-gray-100 text-gray-500 hover:border-blue-400 hover:bg-blue-50",
    }
}

fn parse_coordinates(role: Option<&str>) -> Vec<(String, String)> {
    let mut parsed = BTreeMap::new();
    for part in role.unwrap_or_default().split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if matches!(key.as_str(), "dp" | "pp" | "tp" | "sp" | "cp" | "ep") && !value.is_empty() {
            parsed.insert(key, value.to_string());
        }
    }
    ["dp", "pp", "tp", "sp", "cp", "ep"]
        .into_iter()
        .filter_map(|key| parsed.remove(key).map(|value| (key.to_string(), value)))
        .collect()
}

fn build_placement(nodes: &[Node]) -> PlacementModel {
    let mut hosts: BTreeMap<String, Vec<PlacementProcess>> = BTreeMap::new();
    let mut ranks = BTreeSet::new();
    let mut expected_ranks = 0usize;

    for node in nodes {
        if let Some(rank) = node.rank {
            ranks.insert(rank);
        }
        if let Some(world_size) = node.world_size.filter(|size| *size > 0) {
            expected_ranks = expected_ranks.max(world_size as usize);
        }
        let coordinates = parse_coordinates(node.role.as_deref());
        let role_label = node
            .role_name
            .clone()
            .filter(|role| !role.trim().is_empty());
        let host = if node.host.trim().is_empty() {
            "Unknown host".to_string()
        } else {
            node.host.clone()
        };
        hosts.entry(host).or_default().push(PlacementProcess {
            rank: node.rank,
            local_rank: node.local_rank,
            role_label,
            coordinates,
            status: node.status.clone(),
        });
    }

    let mut hosts = hosts
        .into_iter()
        .map(|(name, mut processes)| {
            processes.sort_by_key(|process| {
                (
                    process.rank.unwrap_or(i32::MAX),
                    process.local_rank.unwrap_or(i32::MAX),
                )
            });
            PlacementHost { name, processes }
        })
        .collect::<Vec<_>>();
    hosts.sort_by(|left, right| left.name.cmp(&right.name));

    let observed_ranks = ranks.len();
    if expected_ranks == 0 {
        expected_ranks = observed_ranks;
    }
    let mut values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for process in hosts.iter().flat_map(|host| host.processes.iter()) {
        for (dimension, value) in &process.coordinates {
            values
                .entry(dimension.clone())
                .or_default()
                .insert(value.clone());
        }
    }
    let parallel_sizes = ["dp", "pp", "tp", "sp", "cp", "ep"]
        .into_iter()
        .filter_map(|dimension| {
            values
                .get(dimension)
                .map(|values| (dimension.to_string(), values.len()))
        })
        .collect::<Vec<_>>();
    let has_parallel_coordinates = hosts
        .iter()
        .flat_map(|host| host.processes.iter())
        .any(|process| !process.coordinates.is_empty());

    PlacementModel {
        hosts,
        observed_ranks,
        expected_ranks,
        has_parallel_coordinates,
        parallel_sizes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_uses_reported_megatron_coordinates() {
        let nodes = (0..64)
            .map(|rank| Node {
                host: format!("host-{}", rank / 8),
                rank: Some(rank),
                local_rank: Some(rank % 8),
                world_size: Some(64),
                role: Some(format!(
                    "dp={},pp={},tp={}",
                    rank / 8,
                    (rank / 2) % 4,
                    rank % 2
                )),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let placement = build_placement(&nodes);

        assert_eq!(placement.hosts.len(), 8);
        assert_eq!(placement.observed_ranks, 64);
        assert_eq!(placement.expected_ranks, 64);
        assert_eq!(
            placement.parallel_sizes,
            vec![("dp".into(), 8), ("pp".into(), 4), ("tp".into(), 2)]
        );
    }

    #[test]
    fn communication_groups_have_non_color_labels() {
        assert_eq!(placement_group_code(Some(PlacementGroup::Focus)), Some("●"));
        assert_eq!(
            placement_group_code(Some(PlacementGroup::Tensor)),
            Some("T")
        );
        assert_eq!(placement_group_code(Some(PlacementGroup::Data)), Some("D"));
        assert_eq!(
            placement_group_code(Some(PlacementGroup::Pipeline)),
            Some("P")
        );
        assert_eq!(placement_group_code(None), None);
    }
}
