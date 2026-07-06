//! Host / GPU / torch memory snapshot at crash time.

use nix::libc;
use probing_core::runtime::block_on;
use probing_core::ENGINE;
use probing_proto::prelude::{DataFrame, Ele, Seq};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GpuDeviceMemory {
    pub device_id: i32,
    pub name: String,
    pub used_bytes: i64,
    pub total_bytes: i64,
    pub mem_used_pct: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MemorySnapshot {
    /// Fresh process RSS from `/proc` or `getrusage` (bytes). `-1` if unknown.
    pub host_rss_bytes: i64,
    /// Process thread count from the same direct read. `-1` if unknown.
    pub thread_count: i32,
    /// Latest `cpu.utilization` sample (KiB). `-1` if table empty / unavailable.
    pub sampled_rss_kb: i64,
    /// Per-device rows from latest `gpu.utilization` sample.
    pub gpu_devices: Vec<GpuDeviceMemory>,
    /// Latest `max(allocated)` from `python.torch_trace` (bytes). `-1` if unavailable.
    pub torch_allocated_bytes: i64,
    /// `true` when live table queries (GPU/torch hints) could not be fetched.
    pub table_hints_unavailable: bool,
}

impl MemorySnapshot {
    pub fn capture() -> Self {
        let direct = read_host_direct();
        let mut snap = MemorySnapshot {
            host_rss_bytes: direct.rss_bytes,
            thread_count: direct.thread_count,
            ..Default::default()
        };
        match block_on(fetch_table_hints()) {
            Ok(Ok(hints)) => snap.merge_table_hints(hints),
            Ok(Err(e)) => {
                log::warn!("memory snapshot: table hints query failed: {e}");
                snap.table_hints_unavailable = true;
            }
            Err(e) => {
                log::warn!("memory snapshot: async bridge unavailable for table hints: {e}");
                snap.table_hints_unavailable = true;
            }
        }
        snap
    }

    fn merge_table_hints(&mut self, hints: TableMemoryHints) {
        if hints.sampled_rss_kb >= 0 {
            self.sampled_rss_kb = hints.sampled_rss_kb;
        }
        if hints.torch_allocated_bytes >= 0 {
            self.torch_allocated_bytes = hints.torch_allocated_bytes;
        }
        if !hints.gpu_devices.is_empty() {
            self.gpu_devices = hints.gpu_devices;
        }
    }
}

struct HostDirect {
    rss_bytes: i64,
    thread_count: i32,
}

struct TableMemoryHints {
    sampled_rss_kb: i64,
    torch_allocated_bytes: i64,
    gpu_devices: Vec<GpuDeviceMemory>,
}

async fn fetch_table_hints() -> Result<TableMemoryHints, probing_core::runtime::RuntimeError> {
    let engine = ENGINE.read().await;
    let mut hints = TableMemoryHints {
        sampled_rss_kb: -1,
        torch_allocated_bytes: -1,
        gpu_devices: Vec::new(),
    };

    if let Ok(Some(df)) = engine
        .async_query(
            "SELECT rss_kb FROM cpu.utilization \
             WHERE scope = 'process' ORDER BY ts DESC LIMIT 1",
        )
        .await
    {
        hints.sampled_rss_kb = col_i64_first(&df, "rss_kb").unwrap_or(-1);
    }

    if let Ok(Some(df)) = engine
        .async_query(
            "SELECT device_id, name, used_bytes, total_bytes, mem_used_pct \
             FROM gpu.utilization ORDER BY ts DESC LIMIT 32",
        )
        .await
    {
        hints.gpu_devices = latest_gpu_devices(&df);
    }

    if let Ok(Some(df)) = engine
        .async_query(
            "SELECT max(allocated) AS torch_allocated FROM python.torch_trace \
             WHERE allocated IS NOT NULL",
        )
        .await
    {
        hints.torch_allocated_bytes = col_i64_first(&df, "torch_allocated").unwrap_or(-1);
    }

    Ok(hints)
}

fn latest_gpu_devices(df: &DataFrame) -> Vec<GpuDeviceMemory> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in df.iter() {
        let device_id = row_value_i32(&row, df, "device_id").unwrap_or(-1);
        if device_id < 0 || !seen.insert(device_id) {
            continue;
        }
        out.push(GpuDeviceMemory {
            device_id,
            name: row_value_string(&row, df, "name").unwrap_or_default(),
            used_bytes: row_value_i64(&row, df, "used_bytes").unwrap_or(-1),
            total_bytes: row_value_i64(&row, df, "total_bytes").unwrap_or(-1),
            mem_used_pct: row_value_f32(&row, df, "mem_used_pct").unwrap_or(0.0),
        });
    }
    out.sort_by_key(|d| d.device_id);
    out
}

fn read_host_direct() -> HostDirect {
    HostDirect {
        rss_bytes: read_host_rss_bytes_impl(),
        thread_count: read_thread_count(),
    }
}

/// Signal-safe RSS read for fatal-signal spill (Linux best-effort).
pub fn read_host_rss_bytes_signal_safe() -> i64 {
    #[cfg(target_os = "linux")]
    {
        parse_proc_vmrss_signal_safe()
    }
    #[cfg(not(target_os = "linux"))]
    {
        read_host_rss_bytes_impl()
    }
}

fn read_thread_count() -> i32 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(val) = line.strip_prefix("Threads:") {
                    if let Ok(n) = val.trim().parse::<i32>() {
                        return n;
                    }
                }
            }
        }
    }
    -1
}

#[cfg(target_os = "linux")]
fn parse_proc_status_vmrss(content: &str) -> i64 {
    for line in content.lines() {
        if let Some(kb) = line.strip_prefix("VmRSS:").map(str::trim) {
            if let Ok(kib) = kb.split_whitespace().next().unwrap_or("").parse::<i64>() {
                return kib.saturating_mul(1024);
            }
        }
    }
    -1
}

#[cfg(target_os = "linux")]
fn read_host_rss_bytes_impl() -> i64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .map(|s| parse_proc_status_vmrss(&s))
        .unwrap_or(-1)
}

#[cfg(target_os = "macos")]
fn read_host_rss_bytes_impl() -> i64 {
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
            return usage.ru_maxrss as i64;
        }
    }
    -1
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_host_rss_bytes_impl() -> i64 {
    -1
}

#[cfg(target_os = "linux")]
fn parse_proc_vmrss_signal_safe() -> i64 {
    let fd = unsafe { libc::open(c"/proc/self/status".as_ptr(), libc::O_RDONLY) };
    if fd < 0 {
        return -1;
    }
    let mut buf = [0u8; 4096];
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    unsafe { libc::close(fd) };
    if n <= 0 {
        return -1;
    }
    let text = std::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    parse_proc_status_vmrss(text)
}

fn col_index(df: &DataFrame, name: &str) -> Option<usize> {
    df.names.iter().position(|n| n == name)
}

fn col_i64_first(df: &DataFrame, name: &str) -> Option<i64> {
    let idx = col_index(df, name)?;
    match &df.cols[idx] {
        Seq::SeqI64(v) => v.first().copied(),
        Seq::SeqI32(v) => v.first().map(|x| i64::from(*x)),
        Seq::SeqF64(v) => v.first().map(|x| *x as i64),
        _ => None,
    }
}

fn row_value_i64(row: &[Ele], df: &DataFrame, name: &str) -> Option<i64> {
    let idx = col_index(df, name)?;
    match &row[idx] {
        Ele::I64(v) => Some(*v),
        Ele::I32(v) => Some(i64::from(*v)),
        Ele::F64(v) => Some(*v as i64),
        _ => None,
    }
}

fn row_value_i32(row: &[Ele], df: &DataFrame, name: &str) -> Option<i32> {
    let idx = col_index(df, name)?;
    match &row[idx] {
        Ele::I32(v) => Some(*v),
        Ele::I64(v) => i32::try_from(*v).ok(),
        _ => None,
    }
}

fn row_value_f32(row: &[Ele], df: &DataFrame, name: &str) -> Option<f32> {
    let idx = col_index(df, name)?;
    match &row[idx] {
        Ele::F32(v) => Some(*v),
        Ele::F64(v) => Some(*v as f32),
        _ => None,
    }
}

fn row_value_string(row: &[Ele], df: &DataFrame, name: &str) -> Option<String> {
    let idx = col_index(df, name)?;
    match &row[idx] {
        Ele::Text(v) => Some(v.clone()),
        _ => None,
    }
}

pub fn format_bytes(bytes: i64) -> String {
    if bytes < 0 {
        return "?".into();
    }
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.0} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn report_lines(snap: &MemorySnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    if snap.host_rss_bytes < 0
        && snap.sampled_rss_kb < 0
        && snap.gpu_devices.is_empty()
        && snap.torch_allocated_bytes < 0
    {
        return lines;
    }

    let mut parts = Vec::new();
    if snap.host_rss_bytes >= 0 {
        parts.push(format!("host_rss={}", format_bytes(snap.host_rss_bytes)));
    }
    if snap.sampled_rss_kb >= 0 {
        parts.push(format!(
            "sampled_rss={}",
            format_bytes(snap.sampled_rss_kb.saturating_mul(1024))
        ));
    }
    if snap.thread_count >= 0 {
        parts.push(format!("threads={}", snap.thread_count));
    }
    if snap.torch_allocated_bytes >= 0 {
        parts.push(format!(
            "torch_allocated={}",
            format_bytes(snap.torch_allocated_bytes)
        ));
    }
    if !parts.is_empty() {
        lines.push(format!("  memory     {}", parts.join("  ")));
    }

    if snap.table_hints_unavailable {
        lines.push("  memory     table_hints=unavailable (gpu/torch samples not merged)".into());
    }

    for gpu in &snap.gpu_devices {
        let used = format_bytes(gpu.used_bytes);
        let total = format_bytes(gpu.total_bytes);
        let pct = if gpu.mem_used_pct > 0.0 {
            format!(" ({:.0}%)", gpu.mem_used_pct)
        } else {
            String::new()
        };
        lines.push(format!(
            "  memory     gpu{} {} used={}/{}{pct}",
            gpu.device_id,
            if gpu.name.is_empty() {
                "device".into()
            } else {
                gpu.name.clone()
            },
            used,
            total
        ));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_scales() {
        assert_eq!(format_bytes(-1), "?");
        assert_eq!(format_bytes(512), "512 B");
        assert!(format_bytes(5 * 1024 * 1024).contains("MiB"));
        assert!(format_bytes(3 * 1024 * 1024 * 1024).contains("GiB"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_vmrss_parse() {
        let sample = "Name:\tpython\nVmRSS:\t  12345 kB\n";
        assert_eq!(parse_proc_status_vmrss(sample), 12345 * 1024);
    }

    #[test]
    fn report_lines_note_unavailable_table_hints() {
        let snap = MemorySnapshot {
            host_rss_bytes: 1024,
            table_hints_unavailable: true,
            ..Default::default()
        };
        let lines = report_lines(&snap);
        assert!(lines.iter().any(|l| l.contains("table_hints=unavailable")));
    }

    #[test]
    fn report_lines_include_host_and_gpu() {
        let snap = MemorySnapshot {
            host_rss_bytes: 8 * 1024 * 1024 * 1024,
            thread_count: 4,
            torch_allocated_bytes: 2 * 1024 * 1024 * 1024,
            gpu_devices: vec![GpuDeviceMemory {
                device_id: 0,
                name: "GPU".into(),
                used_bytes: 70 * 1024 * 1024 * 1024,
                total_bytes: 80 * 1024 * 1024 * 1024,
                mem_used_pct: 87.5,
            }],
            ..Default::default()
        };
        let lines = report_lines(&snap);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("host_rss="));
        assert!(lines[0].contains("torch_allocated="));
        assert!(lines[1].contains("gpu0"));
    }
}
