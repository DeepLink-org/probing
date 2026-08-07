//! Native Next-page evidence snapshots consumed by the Investigate panel.
//!
//! This module deliberately speaks in Next routes and evidence requests. It
//! must not translate through removed legacy routes: page UI and Agent context should
//! use the same scope and investigation coordinates.

use dioxus::prelude::ReadableExt;
use probing_proto::prelude::{Node, Process};

use crate::api::{ApiClient, CpuSnapshot, CpuThreadRow, GpuSnapshot, RuntimeDebugResponse};
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::training::TRAINING_CLUSTER_SCOPE;
use crate::state::ui_tasks::{begin_snapshot_task, end_snapshot_task};
use crate::utils::error::Result;

use super::evidence::{dataframe_preview, EvidenceRequest, EvidenceScope};
use super::routes::NextRoute;
use super::settings::{MEMORY_CLUSTER_SCOPE, MEMORY_WINDOW_MINUTES};

pub async fn refresh_next_page_snapshot(route: NextRoute) {
    // Keep the previous text visible while marking it stale. Agent calls wait
    // for this flag, and the refresh button cannot start overlapping snapshots.
    crate::state::page_context::set_page_snapshot_loading(true);

    let detail = route.snapshot_id();
    let task = begin_snapshot_task("Next page evidence", Some(detail.to_string()));
    let task_id = task.id();
    let request = request_for_route(&route, task_id);
    let result = fetch_next_page_snapshot(&route, &request).await;

    if task.is_cancelled() {
        task.cancel();
        end_snapshot_task(task_id);
        return;
    }

    match result {
        Ok(snapshot) => {
            task.finish();
            end_snapshot_task(task_id);
            crate::state::page_context::set_page_snapshot(snapshot);
        }
        Err(error) => {
            let message = error.display_message();
            task.fail(&message);
            end_snapshot_task(task_id);
            crate::state::page_context::set_page_snapshot(format!(
                "[next evidence]\nsnapshot={} · {}\n(snapshot unavailable: {message})",
                request.refresh_epoch,
                request.scope.label(),
            ));
        }
    }
}

fn request_for_route(route: &NextRoute, refresh_epoch: u64) -> EvidenceRequest {
    let scope = match route {
        route if route.uses_cluster_scope() => EvidenceScope::ClusterFanout,
        NextRoute::Training {} if *TRAINING_CLUSTER_SCOPE.read() => EvidenceScope::ClusterFanout,
        NextRoute::Memory {} if *MEMORY_CLUSTER_SCOPE.read() => EvidenceScope::ClusterFanout,
        _ => EvidenceScope::LocalProcess,
    };
    let window_us = matches!(route, NextRoute::Memory {})
        .then(|| (*MEMORY_WINDOW_MINUTES.read() as u64).saturating_mul(60_000_000));
    EvidenceRequest::new(
        refresh_epoch,
        scope,
        window_us,
        INVESTIGATION_CONTEXT.read().clone(),
    )
}

async fn fetch_next_page_snapshot(route: &NextRoute, request: &EvidenceRequest) -> Result<String> {
    let client = ApiClient::new();
    let context = request.context.coordinates_summary();
    let mut parts = vec![format!(
        "[next evidence]\npage={} · snapshot={} · scope={} · context={context}",
        route.snapshot_id(),
        request.refresh_epoch,
        request.scope.label(),
    )];

    match route {
        NextRoute::Dashboard {} => {
            let cluster = request.scope == EvidenceScope::ClusterFanout;
            match client.fetch_step_matrix(40, cluster).await {
                Ok(matrix) => parts.push(format_step_matrix(&matrix, request)),
                Err(error) => parts.push(unavailable("train.step", &error.display_message())),
            }
            match client.fetch_gpu_latest().await {
                Ok(devices) => parts.push(format_gpu_devices(&devices, request)),
                Err(error) => parts.push(unavailable("gpu.utilization", &error.display_message())),
            }
        }
        NextRoute::System {} => {
            match client.get_overview().await {
                Ok(process) => parts.push(format_system_process(&process)),
                Err(error) => parts.push(unavailable("process overview", &error.display_message())),
            }
            match client.fetch_cpu_latest().await {
                Ok(Some(snapshot)) => parts.push(format_cpu_snapshot(&snapshot)),
                Ok(None) => parts.push("[cpu.utilization]\n(no process-scope row)".into()),
                Err(error) => parts.push(unavailable("cpu.utilization", &error.display_message())),
            }
            match client.fetch_gpu_latest().await {
                Ok(devices) => parts.push(format_gpu_devices(&devices, request)),
                Err(error) => parts.push(unavailable("gpu.utilization", &error.display_message())),
            }
            match client.fetch_cpu_top_threads(16).await {
                Ok(threads) => parts.push(format_cpu_threads(&threads)),
                Err(error) => parts.push(unavailable("cpu.tasks", &error.display_message())),
            }
        }
        NextRoute::Training {} => {
            let cluster = request.scope == EvidenceScope::ClusterFanout;
            match client.fetch_step_matrix(120, cluster).await {
                Ok(matrix) => parts.push(format_step_matrix(&matrix, request)),
                Err(error) => parts.push(unavailable("train.step", &error.display_message())),
            }
            parts.push(nodes_snapshot(&client).await);
            parts.push(
                scoped_query_preview(
                    &client,
                    request,
                    "python.comm_collective",
                    "SELECT rank, op, group_size, round(avg(duration_ms), 3) AS avg_ms, round(max(duration_ms), 3) AS max_ms FROM python.comm_collective GROUP BY rank, op, group_size ORDER BY max_ms DESC LIMIT 12",
                    12,
                )
                .await,
            );
        }
        NextRoute::Memory {} => {
            let device_filter = request
                .context
                .device_id
                .map(|device| format!(" AND device_id = {device}"))
                .unwrap_or_default();
            let window = request.window_us.unwrap_or(300_000_000);
            let sql = format!(
                "SELECT device_id, used_bytes, total_bytes, round(used_bytes * 100.0 / NULLIF(total_bytes, 0), 1) AS used_pct, ts FROM gpu.utilization WHERE ts >= GREATEST(COALESCE((SELECT MAX(ts) FROM gpu.utilization), 0) - {window}, 0){device_filter} ORDER BY ts DESC LIMIT 16"
            );
            parts.push(scoped_query_preview(&client, request, "gpu.utilization", &sql, 16).await);
            parts.push(
                scoped_query_preview(
                    &client,
                    request,
                    "python.torch_trace allocator",
                    "SELECT rank, local_step, allocated, max_allocated, cached FROM python.torch_trace WHERE rank >= 0 AND allocated >= 0 AND stage LIKE 'post %' ORDER BY local_step DESC, seq DESC LIMIT 8",
                    8,
                )
                .await,
            );
        }
        NextRoute::Distributed {} | NextRoute::Cluster {} => {
            parts.push(nodes_snapshot(&client).await);
            parts.push(
                scoped_query_preview(
                    &client,
                    request,
                    "python.comm_collective",
                    "SELECT rank, op, count(*) AS calls, round(avg(duration_ms), 3) AS avg_ms, round(max(duration_ms), 3) AS max_ms FROM python.comm_collective GROUP BY rank, op ORDER BY max_ms DESC LIMIT 12",
                    12,
                )
                .await,
            );
        }
        NextRoute::DistributedStatus {} => match client.get_pytorch_runtime_debug().await {
            Ok(snapshot) => parts.push(format_runtime_debug(&snapshot)),
            Err(error) => parts.push(unavailable(
                "pytorch runtime debug",
                &error.display_message(),
            )),
        },
        NextRoute::Stack {} | NextRoute::StackThread { .. } => {
            let tid = match route {
                NextRoute::StackThread { tid } => Some(tid.clone()),
                _ => request.context.tid.map(|tid| tid.to_string()),
            };
            match client.get_callstack_with_mode(tid, "mixed").await {
                Ok(frames) => parts.push(format!(
                    "[callstack]\n{}",
                    empty_if_needed(frames.iter().take(16).map(ToString::to_string).collect())
                )),
                Err(error) => parts.push(unavailable("callstack", &error.display_message())),
            }
        }
        NextRoute::Spans {} | NextRoute::TracesLegacy {} | NextRoute::RlSpans {} => {
            let mut filters = Vec::new();
            if let Some(tid) = request.context.tid {
                filters.push(format!("thread_id = {tid}"));
            }
            if let Some(trace_id) = request.context.trace_id {
                filters.push(format!("trace_id = {trace_id}"));
            }
            if let Some(name) = request.context.span_name.as_ref() {
                filters.push(format!("name = '{}'", name.replace('\'', "''")));
            }
            let where_clause = if filters.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", filters.join(" AND "))
            };
            let sql = format!(
                "SELECT record_type, name, thread_id, trace_id, count(*) AS events FROM python.trace_event{where_clause} GROUP BY record_type, name, thread_id, trace_id ORDER BY events DESC LIMIT 16"
            );
            parts
                .push(scoped_query_preview(&client, request, "python.trace_event", &sql, 16).await);
        }
        NextRoute::Profiles {}
        | NextRoute::ProfilingLegacy {}
        | NextRoute::ProfileView { .. }
        | NextRoute::ChromeTrace {} => match client.get_profiler_config().await {
            Ok(config) => parts.push(format!(
                "[profiler config]\n{}",
                empty_if_needed(
                    config
                        .into_iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect()
                )
            )),
            Err(error) => parts.push(unavailable("profiler config", &error.display_message())),
        },
        NextRoute::DistributedStack {} | NextRoute::DistributedPythonStack {} => {
            parts.push(nodes_snapshot(&client).await);
        }
        _ => parts.push("[page evidence]\nNo page-specific live source is registered.".into()),
    }

    Ok(parts.join("\n\n"))
}

async fn scoped_query_preview(
    client: &ApiClient,
    request: &EvidenceRequest,
    source: &'static str,
    sql: &str,
    max_rows: usize,
) -> String {
    if request.scope == EvidenceScope::ClusterFanout {
        match client.cluster_query(sql, true).await {
            Ok(response) => format!(
                "[{source} · {} peers · {} failed · partial={}]\n{}",
                response.meta.nodes_queried,
                response.meta.nodes_failed.len(),
                response.meta.partial,
                dataframe_preview(&response.dataframe, max_rows),
            ),
            Err(error) => unavailable(source, &error.display_message()),
        }
    } else {
        match client.execute_query(sql).await {
            Ok(dataframe) => format!("[{source}]\n{}", dataframe_preview(&dataframe, max_rows)),
            Err(error) => unavailable(source, &error.display_message()),
        }
    }
}

async fn nodes_snapshot(client: &ApiClient) -> String {
    match client.get_nodes().await {
        Ok(nodes) => format_nodes(&nodes),
        Err(error) => unavailable("cluster.nodes", &error.display_message()),
    }
}

fn format_system_process(process: &Process) -> String {
    format!(
        "[process overview]\npid {} · main thread {} · {} threads\nexecutable={}\nworking directory={}\ncommand={}",
        process.pid,
        process.main_thread,
        process.threads.len(),
        process.exe,
        process.cwd,
        process.cmd,
    )
}

fn format_cpu_snapshot(snapshot: &CpuSnapshot) -> String {
    format!(
        "[cpu.utilization · process]\nCPU {:.1}% · user {:.1}% · system {:.1}% · RSS {} KiB · {} threads\ncontext switches voluntary={} · involuntary={} · platform={}",
        snapshot.cpu_total_pct,
        snapshot.cpu_user_pct,
        snapshot.cpu_sys_pct,
        snapshot.rss_kb,
        snapshot.thread_count,
        snapshot.delta_vol_ctxt,
        snapshot.delta_invol_ctxt,
        snapshot.platform,
    )
}

fn format_gpu_devices(devices: &[GpuSnapshot], request: &EvidenceRequest) -> String {
    let rows = devices
        .iter()
        .filter(|device| {
            request
                .context
                .device_id
                .is_none_or(|id| id == device.device_id)
        })
        .map(|device| {
            format!(
                "GPU {} · {} · memory {:.1}% ({}/{}) · compute {}",
                device.device_id,
                device.name,
                device.mem_used_pct,
                device.used_bytes,
                device.total_bytes,
                device
                    .gpu_util_pct
                    .map(|value| format!("{value:.1}%"))
                    .unwrap_or_else(|| "not reported".into()),
            )
        })
        .collect::<Vec<_>>();
    format!("[gpu.utilization]\n{}", empty_if_needed(rows))
}

fn format_cpu_threads(threads: &[CpuThreadRow]) -> String {
    let rows = threads
        .iter()
        .map(|thread| {
            format!(
                "tid {} · {} · state={} · wait={} · cpu_delta_ns={}",
                thread.tid,
                thread.name,
                thread.state,
                thread.wchan.as_deref().unwrap_or("not reported"),
                thread.delta_total_ns,
            )
        })
        .collect::<Vec<_>>();
    format!("[cpu.tasks · top threads]\n{}", empty_if_needed(rows))
}

fn format_runtime_debug(snapshot: &RuntimeDebugResponse) -> String {
    let wait = if snapshot.wait_counters.available {
        let active = snapshot
            .wait_counters
            .counters
            .iter()
            .filter(|counter| counter.active_count > 0)
            .count();
        let calls: i64 = snapshot
            .wait_counters
            .counters
            .iter()
            .map(|counter| counter.total_calls)
            .sum();
        let mut rows = vec![format!(
            "rank {} · source={} · {} counters · {} active · {} total calls",
            snapshot.wait_counters.rank,
            snapshot.wait_counters.source,
            snapshot.wait_counters.counters.len(),
            active,
            calls,
        )];
        rows.extend(
            snapshot
                .wait_counters
                .counters
                .iter()
                .take(12)
                .map(|counter| {
                    format!(
                        "{} · category={} · active={} · calls={} · avg_us={:.1} · max_us={}",
                        counter.name,
                        counter.category,
                        counter.active_count,
                        counter.total_calls,
                        counter.avg_time_us,
                        counter.max_time_us,
                    )
                }),
        );
        format!("[pytorch wait counters]\n{}", rows.join("\n"))
    } else {
        unavailable(
            "pytorch wait counters",
            snapshot
                .wait_counters
                .error
                .as_deref()
                .unwrap_or("capability not reported"),
        )
    };

    let store = if snapshot.tcpstore.available {
        let mut rows = vec![format!(
            "{} keys · {} identified · catalog={} · values={}",
            snapshot.tcpstore.total_keys,
            snapshot.tcpstore.identified_keys,
            snapshot.tcpstore.catalog_mode,
            snapshot.tcpstore.values_enabled,
        )];
        rows.extend(
            snapshot
                .tcpstore
                .facts
                .iter()
                .map(|fact| format!("{}={}", fact.label, fact.value)),
        );
        rows.extend(snapshot.tcpstore.entries.iter().take(12).map(|entry| {
            format!(
                "{} · category={} · {} bytes · redacted={}",
                entry.key, entry.category, entry.value_size, entry.redacted,
            )
        }));
        format!("[pytorch TCPStore]\n{}", rows.join("\n"))
    } else {
        unavailable(
            "pytorch TCPStore",
            snapshot
                .tcpstore
                .error
                .as_deref()
                .unwrap_or("capability not reported"),
        )
    };

    format!("{wait}\n\n{store}")
}

pub(crate) fn format_nodes(nodes: &[Node]) -> String {
    let mut lines = vec![format!("{} registered node(s)", nodes.len())];
    lines.extend(nodes.iter().take(12).map(|node| {
        format!(
            "rank {} · {} · {} · {}",
            node.rank
                .map(|rank| rank.to_string())
                .unwrap_or_else(|| "—".into()),
            node.host,
            node.addr,
            node.status.as_deref().unwrap_or("status not reported"),
        )
    }));
    if nodes.len() > 12 {
        lines.push(format!("… +{} nodes", nodes.len() - 12));
    }
    format!("[cluster.nodes]\n{}", lines.join("\n"))
}

pub(crate) fn format_step_matrix(
    matrix: &probing_proto::protocol::training::StepMatrixResponse,
    request: &EvidenceRequest,
) -> String {
    let samples = matrix
        .samples
        .iter()
        .filter(|sample| request.context.rank.is_none_or(|rank| rank == sample.rank))
        .rev()
        .take(12)
        .map(|sample| {
            format!(
                "rank {} · step {} · {:.3}ms · {}",
                sample.rank, sample.coord_step, sample.duration_ms, sample.host
            )
        })
        .collect::<Vec<_>>();
    format!(
        "[train.step · {} ranks · {} steps · {} peers · {} failed · partial={}]\n{}",
        matrix.rank_count,
        matrix.step_count,
        matrix.nodes_queried,
        matrix.nodes_failed.len(),
        matrix.partial,
        empty_if_needed(samples),
    )
}

fn empty_if_needed(lines: Vec<String>) -> String {
    if lines.is_empty() {
        "(no matching rows)".into()
    } else {
        lines.join("\n")
    }
}

fn unavailable(source: &str, detail: &str) -> String {
    format!("[{source}]\n(unavailable: {detail})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_ids_follow_canonical_routes() {
        assert_eq!(NextRoute::Memory {}.snapshot_id(), "memory");
        assert_eq!(NextRoute::Training {}.snapshot_id(), "training");
        assert_eq!(NextRoute::Cluster {}.snapshot_id(), "cluster-nodes");
    }

    #[test]
    fn distributed_runtime_status_is_local_process_evidence() {
        assert!(!NextRoute::DistributedStatus {}.uses_cluster_scope());
        assert!(NextRoute::Distributed {}.uses_cluster_scope());
    }

    #[test]
    fn pinned_rank_filters_step_snapshot_without_relabelling_other_ranks() {
        let matrix = probing_proto::protocol::training::StepMatrixResponse {
            samples: vec![
                probing_proto::protocol::training::StepDurationSample {
                    rank: 0,
                    local_step: 0,
                    coord_step: 10,
                    duration_ms: 8.0,
                    host: "node-0".into(),
                    addr: "a".into(),
                },
                probing_proto::protocol::training::StepDurationSample {
                    rank: 1,
                    local_step: 0,
                    coord_step: 10,
                    duration_ms: 12.0,
                    host: "node-1".into(),
                    addr: "b".into(),
                },
            ],
            rank_count: 2,
            step_count: 1,
            cluster: true,
            partial: false,
            nodes_queried: 2,
            nodes_failed: Vec::new(),
        };
        let request = EvidenceRequest::new(
            7,
            EvidenceScope::ClusterFanout,
            None,
            crate::state::investigation::InvestigationContext {
                rank: Some(1),
                ..Default::default()
            },
        );

        let preview = format_step_matrix(&matrix, &request);
        assert!(preview.contains("rank 1 · step 10 · 12.000ms"));
        assert!(!preview.contains("rank 0 · step 10"));
    }

    #[test]
    fn system_snapshot_describes_the_evidence_visible_on_the_system_page() {
        let process = Process {
            pid: 42,
            exe: "/usr/bin/python".into(),
            cmd: "python train.py".into(),
            cwd: "/workspace".into(),
            main_thread: 42,
            threads: vec![42, 43],
            ..Default::default()
        };
        let cpu = CpuSnapshot {
            platform: "linux".into(),
            cpu_total_pct: 75.0,
            cpu_user_pct: 60.0,
            cpu_sys_pct: 15.0,
            rss_kb: 1024,
            thread_count: 2,
            ..Default::default()
        };
        let threads = vec![CpuThreadRow {
            tid: 43,
            name: "worker".into(),
            state: "R".into(),
            wchan: None,
            delta_user_ns: 7,
            delta_sys_ns: 3,
            delta_total_ns: 10,
        }];

        assert!(format_system_process(&process).contains("pid 42 · main thread 42 · 2 threads"));
        assert!(format_cpu_snapshot(&cpu).contains("CPU 75.0% · user 60.0% · system 15.0%"));
        assert!(format_cpu_threads(&threads).contains("tid 43 · worker · state=R"));
    }

    #[test]
    fn distributed_status_snapshot_keeps_capability_failures_independent() {
        let snapshot = RuntimeDebugResponse {
            wait_counters: crate::api::WaitCounterSnapshot {
                available: false,
                error: Some("wait handler missing".into()),
                source: String::new(),
                rank: 0,
                counters: Vec::new(),
            },
            tcpstore: crate::api::TcpStoreSnapshot {
                available: true,
                error: None,
                values_enabled: false,
                catalog_available: true,
                catalog_mode: "complete".into(),
                total_keys: 8,
                identified_keys: 8,
                facts: Vec::new(),
                entries: Vec::new(),
            },
        };

        let preview = format_runtime_debug(&snapshot);
        assert!(preview.contains("[pytorch wait counters]\n(unavailable: wait handler missing)"));
        assert!(preview.contains("[pytorch TCPStore]\n8 keys · 8 identified"));
    }
}
