//! Page-aware tools: describe routes and fetch live snapshots for the Agent / LLM.

use dioxus::prelude::ReadableExt;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::ApiClient;
use crate::app::Route;
use crate::components::flamegraph::FlamegraphPayload;
use crate::state::profiling::{normalize_profiling_view, PROFILING_PPROF_FREQ, PROFILING_TORCH_ENABLED};
use crate::state::ui_tasks::{begin_snapshot_task, end_snapshot_task};
use crate::utils::error::Result;

pub use crate::agent::routing::describe_route;

fn ele_str(ele: &Ele) -> String {
    match ele {
        Ele::Nil => "—".into(),
        Ele::BOOL(x) => x.to_string(),
        Ele::I32(x) => x.to_string(),
        Ele::I64(x) => x.to_string(),
        Ele::F32(x) => format!("{x:.4}"),
        Ele::F64(x) => format!("{x:.4}"),
        Ele::Text(x) => x.clone(),
        Ele::Url(x) => x.clone(),
        Ele::DataTime(x) => x.to_string(),
    }
}

pub fn dataframe_preview(df: &DataFrame, max_rows: usize) -> String {
    let nrows = df.cols.iter().map(|c| c.len()).max().unwrap_or(0);
    if nrows == 0 || df.names.is_empty() {
        return "(empty)".into();
    }
    let take = nrows.min(max_rows);
    let mut lines = vec![df.names.join("\t")];
    for row in 0..take {
        let cells: Vec<String> = df
            .cols
            .iter()
            .map(|col| ele_str(&col.get(row)))
            .collect();
        lines.push(cells.join("\t"));
    }
    if nrows > take {
        lines.push(format!("… +{} rows", nrows - take));
    }
    lines.join("\n")
}

async fn query_preview(client: &ApiClient, sql: &str, max_rows: usize) -> Option<String> {
    match client.execute_query(sql).await {
        Ok(df) => Some(dataframe_preview(&df, max_rows)),
        Err(_) => None,
    }
}

/// Fetch live page snapshot via SQL/API (Agent tool: `get_page_snapshot`).
pub async fn fetch_page_snapshot(route: &Route) -> Result<String> {
    let client = ApiClient::new();
    let mut parts: Vec<String> = Vec::new();

    let cluster = crate::agent::fetch_cluster_snapshot().await;
    parts.push(format!("[cluster]\n{}", cluster.nodes_summary));

    match route {
        Route::DashboardPage {} => {
            if let Some(p) = query_preview(
                &client,
                "SELECT ts, cpu_total_pct, rss_kb, thread_count FROM cpu.utilization WHERE scope='process' ORDER BY ts DESC LIMIT 3",
                3,
            )
            .await
            {
                parts.push(format!("[cpu.utilization]\n{p}"));
            }
            if let Some(p) = query_preview(
                &client,
                "SELECT ts, name, mem_used_pct, gpu_util_pct FROM gpu.utilization ORDER BY ts DESC LIMIT 3",
                3,
            )
            .await
            {
                parts.push(format!("[gpu.utilization]\n{p}"));
            }
            if let Some(p) = query_preview(
                &client,
                "SELECT max(step) AS latest_step FROM python.torch_trace",
                1,
            )
            .await
            {
                parts.push(format!("[torch_trace]\n{p}"));
            }
        }
        Route::ClusterPage {} => {
            if let Ok(nodes) = client.get_nodes().await {
                parts.push(format!("[cluster.nodes] {} registered", nodes.len()));
            }
            if cluster.has_peers() {
                if let Ok(resp) = client
                    .cluster_query(
                        "SELECT rank, op, avg(duration_ms) AS avg_ms, count(*) AS n \
                         FROM global.python.comm_collective \
                         GROUP BY rank, op ORDER BY avg_ms DESC LIMIT 8",
                        true,
                    )
                    .await
                {
                    let preview = dataframe_preview(&resp.dataframe, 8);
                    parts.push(format!(
                        "[global.python.comm_collective · {} nodes]\n{preview}",
                        resp.meta.nodes_queried
                    ));
                }
            }
        }
        Route::StackPage {} => {
            if let Ok(frames) = client.get_callstack_with_mode(None, "mixed").await {
                let preview: String = frames
                    .iter()
                    .take(12)
                    .map(|f| format!("{f}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                parts.push(format!("[callstack top 12]\n{preview}"));
            }
        }
        Route::StackWithTidPage { tid } => {
            if let Ok(frames) = client
                .get_callstack_with_mode(Some(tid.clone()), "mixed")
                .await
            {
                let preview: String = frames
                    .iter()
                    .take(12)
                    .map(|f| format!("{f}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                parts.push(format!("[callstack tid={tid} top 12]\n{preview}"));
            }
        }
        Route::ProfilingViewPage { view } => {
            let v = normalize_profiling_view(view);
            parts.push(format!(
                "UI state: pprof.sample_freq={} torch.profiling={}",
                *PROFILING_PPROF_FREQ.read(),
                *PROFILING_TORCH_ENABLED.read()
            ));
            if let Ok(cfg) = client.get_profiler_config().await {
                for (k, val) in cfg {
                    parts.push(format!("{k}={val}"));
                }
            }
            match v {
                "torch" => {
                    if let Some(p) = query_preview(
                        &client,
                        "SELECT max(step) AS latest_step, count(*) AS rows FROM python.torch_trace",
                        1,
                    )
                    .await
                    {
                        parts.push(format!("[torch_trace]\n{p}"));
                    }
                }
                "pprof" => {
                    let freq = *PROFILING_PPROF_FREQ.read();
                    parts.push(format!("[pprof] probing.pprof.sample_freq={freq}"));
                    if freq > 0 {
                        match client.get_flamegraph_json("pprof").await {
                            Ok(json) => {
                                if let Ok(payload) =
                                    serde_json::from_str::<FlamegraphPayload>(&json)
                                {
                                    parts.push(format!(
                                        "[pprof flamegraph] total_samples={} dropped={}",
                                        payload.total, payload.dropped
                                    ));
                                } else {
                                    parts.push(
                                        "[pprof] sampling enabled; flamegraph JSON not ready yet"
                                            .into(),
                                    );
                                }
                            }
                            Err(_) => parts.push(
                                "[pprof] sampling enabled but no CPU stacks collected yet"
                                    .into(),
                            ),
                        }
                    } else {
                        parts.push(
                            "[pprof] CPU sampling off — enable sample_freq in Profiling sidebar"
                                .into(),
                        );
                    }
                }
                _ => {}
            }
        }
        Route::TrainingPage {} => {
            if cluster.has_peers() {
                if let Ok(resp) = client
                    .cluster_query(
                        "SELECT rank, op, avg(duration_ms) AS avg_ms, count(*) AS n \
                         FROM global.python.comm_collective \
                         GROUP BY rank, op ORDER BY avg_ms DESC LIMIT 8",
                        true,
                    )
                    .await
                {
                    let preview = dataframe_preview(&resp.dataframe, 8);
                    parts.push(format!(
                        "[global comm_collective · {} nodes]\n{preview}",
                        resp.meta.nodes_queried
                    ));
                }
            } else if let Some(p) = query_preview(
                &client,
                "SELECT rank, op, avg(duration_ms) AS avg_ms, count(*) AS n FROM python.comm_collective GROUP BY rank, op ORDER BY avg_ms DESC LIMIT 8",
                8,
            )
            .await
            {
                parts.push(format!("[comm_collective by rank]\n{p}"));
            }
            if let Some(p) = query_preview(
                &client,
                "SELECT max(step) AS latest_step FROM python.torch_trace",
                1,
            )
            .await
            {
                parts.push(format!("[torch_trace]\n{p}"));
            }
        }
        Route::TracesPage {} | Route::SpansPage {} => {
            if let Some(p) = query_preview(
                &client,
                "SELECT record_type, count(*) AS n FROM python.trace_event GROUP BY record_type",
                8,
            )
            .await
            {
                parts.push(format!("[trace_event counts]\n{p}"));
            }
        }
        Route::PulsingPage {} => {
            if let Some(p) = query_preview(&client, "SELECT count(*) AS actors FROM pulsing.actors", 1).await {
                parts.push(format!("[pulsing.actors]\n{p}"));
            }
        }
        Route::AnalyticsPage {} => {
            if let Some(p) = query_preview(
                &client,
                "SELECT table_schema, table_name FROM information_schema.tables WHERE table_schema NOT IN ('information_schema') ORDER BY 1,2 LIMIT 12",
                12,
            )
            .await
            {
                parts.push(format!("[tables]\n{p}"));
            }
        }
        _ => {}
    }

    if parts.is_empty() {
        Ok("(no live snapshot for this page)".into())
    } else {
        Ok(parts.join("\n\n"))
    }
}

pub async fn refresh_page_snapshot_for_route(route: Route) {
    refresh_page_snapshot(route, true).await;
}

/// Refresh without blanking the UI when a snapshot is already shown (e.g. manual Refresh).
pub async fn refresh_page_snapshot_quiet(route: Route) {
    refresh_page_snapshot(route, false).await;
}

async fn refresh_page_snapshot(route: Route, show_loading: bool) {
    let had_snapshot = !crate::state::page_context::PAGE_CONTEXT.read().snapshot.is_empty();
    if show_loading && !had_snapshot {
        crate::state::page_context::set_page_snapshot_loading(true);
    }

    let detail = describe_route(&route).title;
    let task = begin_snapshot_task("Page snapshot", Some(detail));
    let task_id = task.id();

    let result = fetch_page_snapshot(&route).await;

    if task.is_cancelled() {
        task.cancel();
        end_snapshot_task(task_id);
        return;
    }

    match result {
        Ok(s) => {
            task.finish();
            end_snapshot_task(task_id);
            crate::state::page_context::set_page_snapshot(s);
        }
        Err(e) => {
            let msg = e.display_message();
            task.fail(&msg);
            end_snapshot_task(task_id);
            crate::state::page_context::set_page_snapshot(format!("(snapshot error: {msg})"));
        }
    }
}
