use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct AppleGpuPerfStats {
    pub device_util_pct: Option<f32>,
    pub renderer_util_pct: Option<f32>,
    pub tiler_util_pct: Option<f32>,
    pub in_use_system_memory: Option<u64>,
    pub alloc_system_memory: Option<u64>,
    pub model: Option<String>,
}

/// Read AGX / IOAccelerator performance counters via `ioreg` (M1–M4).
pub fn read_performance_stats() -> Option<AppleGpuPerfStats> {
    let output = Command::new("ioreg")
        .args(["-r", "-c", "IOAccelerator", "-l"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_ioreg_output(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_ioreg_output(text: &str) -> AppleGpuPerfStats {
    let mut stats = AppleGpuPerfStats::default();

    for line in text.lines() {
        if line.contains("\"PerformanceStatistics\"") {
            stats.device_util_pct = extract_pct(line, "Device Utilization %");
            stats.renderer_util_pct = extract_pct(line, "Renderer Utilization %");
            stats.tiler_util_pct = extract_pct(line, "Tiler Utilization %");
            stats.in_use_system_memory = extract_u64(line, "In use system memory");
            stats.alloc_system_memory = extract_u64(line, "Alloc system memory");
        }
        if stats.model.is_none() && line.contains("\"model\"") {
            stats.model = extract_quoted_value(line, "model");
        }
    }

    stats
}

fn extract_pct(line: &str, key: &str) -> Option<f32> {
    extract_u64(line, key).map(|v| v as f32)
}

fn extract_u64(line: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(',').unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn extract_quoted_value(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\" = ");
    let start = line.find(&needle)? + needle.len();
    let rest = line[start..].trim();
    if !rest.starts_with('"') {
        return None;
    }
    let inner = rest.trim_start_matches('"');
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_performance_statistics_line() {
        let line = r#"|   "PerformanceStatistics" = {"Device Utilization %"=28,"Renderer Utilization %"=27,"Tiler Utilization %"=28,"In use system memory"=688357376,"Alloc system memory"=13377732608}"#;
        let stats = parse_ioreg_output(line);
        assert_eq!(stats.device_util_pct, Some(28.0));
        assert_eq!(stats.renderer_util_pct, Some(27.0));
        assert_eq!(stats.in_use_system_memory, Some(688357376));
    }
}
