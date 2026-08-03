use dioxus::prelude::*;
use dioxus_router::Link;
use probing_proto::prelude::Process;

use crate::api::{format_cpu_ms, ApiClient, CpuSnapshot, CpuThreadRow, GpuSnapshot};

use super::super::components::{
    EvidenceMetric, LoadingPanel, SectionCard, UnavailablePanel, WorkspacePage,
};
use super::super::routes::NextRoute;

#[component]
pub fn SystemPage() -> Element {
    let process = use_resource(|| async move { ApiClient::new().get_overview().await });
    let cpu = use_resource(|| async move { ApiClient::new().fetch_cpu_latest().await });
    let gpu = use_resource(|| async move { ApiClient::new().fetch_gpu_latest().await });
    let threads = use_resource(|| async move { ApiClient::new().fetch_cpu_top_threads(16).await });
    let process_state = process.read().clone();
    let cpu_state = cpu.read().clone();
    let gpu_state = gpu.read().clone();
    let thread_state = threads.read().clone();

    rsx! {
        WorkspacePage {
            title: "System".to_string(),
            subtitle: "Target process identity and the latest reported CPU, memory, GPU, and thread samples.".to_string(),
            div { class: "grid items-start gap-4 xl:grid-cols-2",
                SectionCard { title: "Target process".to_string(), subtitle: Some("Values come from the current /apis/overview response.".to_string()),
                    match process_state {
                        None => rsx! { LoadingPanel { label: "Loading process identity".to_string() } },
                        Some(Err(error)) => rsx! { UnavailablePanel { label: "Process identity unavailable".to_string(), detail: error.display_message() } },
                        Some(Ok(process)) => rsx! { ProcessEvidence { process } },
                    }
                }
                SectionCard { title: "CPU and memory".to_string(), subtitle: Some("Latest process-scope cpu.utilization row.".to_string()),
                    match cpu_state {
                        None => rsx! { LoadingPanel { label: "Loading CPU sample".to_string() } },
                        Some(Err(error)) => rsx! { UnavailablePanel { label: "CPU sample unavailable".to_string(), detail: error.display_message() } },
                        Some(Ok(None)) => rsx! { UnavailablePanel { label: "No CPU sample".to_string(), detail: "No process-scope utilization row was returned.".to_string() } },
                        Some(Ok(Some(snapshot))) => rsx! { CpuEvidence { snapshot } },
                    }
                }
            }
            SectionCard { title: "Accelerators".to_string(), subtitle: Some("One row per device at the latest shared utilization timestamp.".to_string()), body_class: "p-0".to_string(),
                match gpu_state {
                    None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading accelerator samples".to_string() } } },
                    Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Accelerator samples unavailable".to_string(), detail: error.display_message() } } },
                    Some(Ok(devices)) if devices.is_empty() => rsx! { div { class: "p-4", UnavailablePanel { label: "No accelerator samples".to_string(), detail: "gpu.utilization returned no devices.".to_string() } } },
                    Some(Ok(devices)) => rsx! { GpuEvidence { devices } },
                }
            }
            SectionCard { title: "Top CPU threads".to_string(), subtitle: Some("Latest sampled threads ordered by reported CPU delta; open a thread to capture its current stack.".to_string()), body_class: "p-0".to_string(),
                match thread_state {
                    None => rsx! { div { class: "p-4", LoadingPanel { label: "Loading thread samples".to_string() } } },
                    Some(Err(error)) => rsx! { div { class: "p-4", UnavailablePanel { label: "Thread samples unavailable".to_string(), detail: error.display_message() } } },
                    Some(Ok(rows)) if rows.is_empty() => rsx! { div { class: "p-4", UnavailablePanel { label: "No thread samples".to_string(), detail: "cpu.tasks returned no rows.".to_string() } } },
                    Some(Ok(rows)) => rsx! { ThreadEvidence { rows } },
                }
            }
        }
    }
}

#[component]
fn ProcessEvidence(process: Process) -> Element {
    rsx! {
        div { class: "grid grid-cols-3 divide-x divide-gray-200",
            EvidenceMetric { label: "PID", value: process.pid.to_string() }
            EvidenceMetric { label: "Main thread", value: process.main_thread.to_string() }
            EvidenceMetric { label: "Threads", value: process.threads.len().to_string() }
        }
        dl { class: "mt-4 grid gap-2 text-xs",
            EvidenceLine { label: "Executable", value: process.exe }
            EvidenceLine { label: "Working directory", value: process.cwd }
            EvidenceLine { label: "Command", value: process.cmd }
        }
    }
}

#[component]
fn CpuEvidence(snapshot: CpuSnapshot) -> Element {
    let width = snapshot.cpu_total_pct.clamp(0.0, 100.0);
    rsx! {
        div { class: "grid grid-cols-4 divide-x divide-gray-200",
            EvidenceMetric { label: "CPU total", value: format!("{:.1}%", snapshot.cpu_total_pct) }
            EvidenceMetric { label: "User / system", value: format!("{:.1}% / {:.1}%", snapshot.cpu_user_pct, snapshot.cpu_sys_pct) }
            EvidenceMetric { label: "RSS", value: format_bytes(snapshot.rss_kb.saturating_mul(1024)) }
            EvidenceMetric { label: "Threads", value: snapshot.thread_count.to_string() }
        }
        div { class: "mt-4 h-2 overflow-hidden rounded-full bg-gray-100", div { class: "h-full bg-blue-500", style: "width: {width}%" } }
        p { class: "mt-2 text-xs text-gray-500", "Platform {snapshot.platform} · voluntary / involuntary context switches {snapshot.delta_vol_ctxt} / {snapshot.delta_invol_ctxt}" }
    }
}

#[component]
fn GpuEvidence(devices: Vec<GpuSnapshot>) -> Element {
    rsx! { div { class: "divide-y divide-gray-100",
        for device in devices {
            div { class: "grid grid-cols-[80px_minmax(180px,1fr)_minmax(180px,1fr)_120px] items-center gap-4 px-4 py-3 text-xs",
                div { class: "min-w-0 font-mono font-medium text-gray-800",
                    div { "GPU {device.device_id}" }
                    div { class: "mt-0.5 break-words text-xs font-normal text-gray-500", "{device.name}" }
                }
                UsageBar { label: "Compute", value: device.gpu_util_pct }
                UsageBar { label: "Memory", value: Some(device.mem_used_pct) }
                div { class: "text-right text-xs text-gray-500", "{format_bytes(device.used_bytes)} / {format_bytes(device.total_bytes)}" }
            }
        }
    } }
}

#[component]
fn UsageBar(label: &'static str, value: Option<f32>) -> Element {
    let display = value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "—".to_string());
    let width = value.unwrap_or(0.0).clamp(0.0, 100.0);
    rsx! { div { class: "min-w-0",
        div { class: "mb-1 flex justify-between text-xs text-gray-500", span { "{label}" } span { class: "font-mono", "{display}" } }
        div { class: "h-1.5 overflow-hidden rounded-full bg-gray-100", div { class: "h-full bg-blue-500", style: "width: {width}%" } }
    } }
}

#[component]
fn ThreadEvidence(rows: Vec<CpuThreadRow>) -> Element {
    rsx! { table { class: "w-full text-left text-xs",
        thead { class: "bg-gray-50 text-xs uppercase tracking-wide text-gray-500", tr { th { class: "px-4 py-2", "TID" } th { class: "px-4 py-2", "Name" } th { class: "px-4 py-2", "State / wait" } th { class: "px-4 py-2 text-right", "CPU delta" } th { class: "px-4 py-2" } } }
        tbody { class: "divide-y divide-gray-100",
            for row in rows { tr {
                { let wait = row.wchan.as_deref().unwrap_or(""); rsx! {
                    td { class: "px-4 py-2 font-mono", "{row.tid}" }
                    td { class: "max-w-xs truncate px-4 py-2", "{row.name}" }
                    td { class: "px-4 py-2 text-gray-500", "{row.state} {wait}" }
                    td { class: "px-4 py-2 text-right font-mono", "{format_cpu_ms(row.delta_total_ns)}" }
                    td { class: "px-4 py-2 text-right", Link { to: NextRoute::StackThread { tid: row.tid.to_string() }, class: "text-blue-600 hover:underline", "Open stack →" } }
                } }
            } }
        }
    } }
}

#[component]
fn EvidenceLine(label: &'static str, value: String) -> Element {
    rsx! { div { class: "grid grid-cols-[120px_minmax(0,1fr)] gap-3", dt { class: "text-gray-500", "{label}" } dd { class: "break-all font-mono text-gray-800", "{value}" } } }
}

fn format_bytes(bytes: i64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_units_are_human_readable() {
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }
}
