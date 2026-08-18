use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

/// Per-logical-device stats (Phy-ID / torch device ordinal on Ascend A3).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct NpuDeviceStats {
    /// Prefer `NPU Utilization(%)`, else `Aicore Usage Rate(%)`.
    pub ai_core_util_pct: f32,
    pub hbm_util_pct: f32,
    pub hbm_total_bytes: u64,
    pub hbm_used_bytes: u64,
    /// `Aivector Usage Rate(%)` — closest Ascend proxy for Minder "tensor activity".
    pub aivector_util_pct: Option<f32>,
    /// `HBM Bandwidth Usage Rate(%)`.
    pub hbm_bw_util_pct: Option<f32>,
    pub temp_c: Option<f32>,
    pub power_w: Option<f32>,
    /// `"dcmi"` or `"smi"`.
    pub source: &'static str,
}

impl Default for NpuDeviceStats {
    fn default() -> Self {
        Self {
            ai_core_util_pct: 0.0,
            hbm_util_pct: 0.0,
            hbm_total_bytes: 0,
            hbm_used_bytes: 0,
            aivector_util_pct: None,
            hbm_bw_util_pct: None,
            temp_c: None,
            power_w: None,
            source: "smi",
        }
    }
}

/// `npu-smi info -t usages` is ~0.5–1s per card. Without caching, one collector
/// tick calls it once in `sample_all` plus again per `sample_memory` → tens of
/// seconds before the first `gpu.utilization` row appears.
const UTIL_CACHE_TTL: Duration = Duration::from_millis(500);
/// Per-invocation timeout so a stuck `npu-smi` cannot block the collector forever.
const NPU_SMI_TIMEOUT: Duration = Duration::from_secs(5);

struct UtilCache {
    at: Instant,
    map: HashMap<i32, NpuDeviceStats>,
}

static UTIL_CACHE: Mutex<Option<UtilCache>> = Mutex::new(None);

#[derive(Debug, Clone, Default)]
pub struct NpuDeviceDetail {
    pub chip_name: Option<String>,
    pub total_mem_bytes: u64,
    pub free_mem_bytes: u64,
}

fn npu_smi_cmd() -> Command {
    let mut cmd = Command::new("npu-smi");
    // Ascend images often need driver libs on LD_LIBRARY_PATH for npu-smi.
    let extra = "/usr/local/Ascend/driver/lib64/common:/usr/local/Ascend/driver/lib64/driver:/usr/local/Ascend/driver/lib64";
    let ld = match std::env::var("LD_LIBRARY_PATH") {
        Ok(v) if v.contains("Ascend/driver") => v,
        Ok(v) if !v.is_empty() => format!("{extra}:{v}"),
        _ => extra.to_string(),
    };
    cmd.env("LD_LIBRARY_PATH", ld);
    cmd
}

/// Public wrapper for sibling modules (`hccs`).
pub(crate) fn run_npu_smi_text(args: &[&str]) -> Option<String> {
    let output = run_npu_smi(args)?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_npu_smi(args: &[&str]) -> Option<std::process::Output> {
    let mut child = npu_smi_cmd()
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let stderr = child.stderr.take()?;
    // Drain both pipes while the process runs. Waiting for exit before reading
    // can deadlock when npu-smi emits more than the OS pipe buffer.
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stdout).read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stderr).read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + NPU_SMI_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    status.map(|status| std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Return visible logical device ordinals (Phy-ID style: NPU×chip).
pub fn list_device_ids() -> Vec<i32> {
    let util = read_utilization_by_index();
    if !util.is_empty() {
        let mut ids: Vec<i32> = util.keys().copied().collect();
        ids.sort_unstable();
        return ids;
    }
    // Fallback: NPU ID list only (single-chip cards).
    if !npu_smi_available() {
        return Vec::new();
    }
    let Some(output) = run_npu_smi(&["info", "-l"]) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut ids = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(id) = parse_kv_i32(line, &["NPU ID", "NpuID", "Device ID"]) else {
            continue;
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids.sort_unstable();
    ids
}

pub fn read_device_detail(ordinal: i32) -> NpuDeviceDetail {
    let mut detail = NpuDeviceDetail {
        chip_name: Some("Ascend910".to_string()),
        ..Default::default()
    };
    if let Some(stats) = read_utilization_by_index().get(&ordinal) {
        detail.total_mem_bytes = stats.hbm_total_bytes;
        detail.free_mem_bytes = stats.hbm_total_bytes.saturating_sub(stats.hbm_used_bytes);
        return detail;
    }
    // Fallback: query by assuming ordinal == NPU ID (single-chip).
    if !npu_smi_available() {
        return detail;
    }
    if let Some(output) = run_npu_smi(&["info", "-t", "board", "-i", &ordinal.to_string()]) {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some(name) = parse_kv_str(
                    line,
                    &["Chip Name", "Product Name", "Product Type", "Model"],
                ) {
                    if name != "NA" {
                        detail.chip_name = Some(name);
                        break;
                    }
                }
            }
        }
    }
    if let Some(output) = run_npu_smi(&["info", "-t", "memory", "-i", &ordinal.to_string()]) {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let total_mb = parse_memory_mb(&text, &["HBM Capacity", "Total Capacity", "HBM Total"]);
            if total_mb > 0 {
                detail.total_mem_bytes = total_mb.saturating_mul(1024 * 1024);
                // memory -t often has no Used; leave free==total if unknown.
                detail.free_mem_bytes = detail.total_mem_bytes;
            }
        }
    }
    detail
}

/// Batch-read utilization keyed by logical Phy-ID (NPU_ID * chips + Chip_ID).
///
/// Prefers in-process DCMI; falls back to `npu-smi`. Cached + single-flight so
/// collector and `sample_memory` share one sweep per TTL window.
pub fn read_utilization_by_index() -> HashMap<i32, NpuDeviceStats> {
    // Hold the lock for the whole refresh (single-flight). Concurrent
    // collector + devices/SQL probes used to launch overlapping `npu-smi`
    // sweeps; on Ascend that can hang and leave `gpu.utilization` empty forever.
    let Ok(mut guard) = UTIL_CACHE.lock() else {
        return HashMap::new();
    };
    if let Some(cache) = guard.as_ref() {
        if cache.at.elapsed() < UTIL_CACHE_TTL {
            return cache.map.clone();
        }
    }

    let map = super::dcmi::read_utilization_by_index()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(read_utilization_by_index_uncached);
    *guard = Some(UtilCache {
        at: Instant::now(),
        map: map.clone(),
    });
    map
}

fn read_utilization_by_index_uncached() -> HashMap<i32, NpuDeviceStats> {
    if !npu_smi_available() {
        return HashMap::new();
    }

    let npu_ids = list_npu_card_ids();
    let mut map = HashMap::new();
    for npu_id in npu_ids {
        let Some(output) = run_npu_smi(&["info", "-t", "usages", "-i", &npu_id.to_string()]) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let chip_count = parse_kv_i32_from_text(&text, &["Chip Count"])
            .unwrap_or(1)
            .max(1);
        for chip in parse_usage_chips(&text) {
            let phy_id = npu_id * chip_count + chip.chip_id;
            let hbm_total = chip.hbm_capacity_mb.saturating_mul(1024 * 1024);
            let hbm_used = if chip.hbm_usage_rate >= 0.0 {
                ((chip.hbm_capacity_mb as f32) * chip.hbm_usage_rate / 100.0).round() as u64
                    * 1024
                    * 1024
            } else {
                0
            };
            let util = if chip.npu_util_pct >= 0.0 {
                chip.npu_util_pct
            } else if chip.aicore_util_pct >= 0.0 {
                chip.aicore_util_pct
            } else {
                0.0
            };
            map.insert(
                phy_id,
                NpuDeviceStats {
                    ai_core_util_pct: util,
                    hbm_util_pct: chip.hbm_usage_rate.max(0.0),
                    hbm_total_bytes: hbm_total,
                    hbm_used_bytes: hbm_used,
                    aivector_util_pct: (chip.aivector_util_pct >= 0.0)
                        .then_some(chip.aivector_util_pct),
                    hbm_bw_util_pct: (chip.hbm_bw_util_pct >= 0.0).then_some(chip.hbm_bw_util_pct),
                    temp_c: None,
                    power_w: None,
                    source: "smi",
                },
            );
        }
        // Power is a separate `-t power` call; fill chips belonging to this card.
        if let Some(power_text) =
            run_npu_smi_text(&["info", "-t", "power", "-i", &npu_id.to_string()])
        {
            let powers = parse_power_by_chip(&power_text);
            let card_power = powers
                .iter()
                .find(|(chip, _)| *chip < 0)
                .map(|(_, w)| *w)
                .or_else(|| powers.first().map(|(_, w)| *w));
            for (phy_id, stats) in map.iter_mut() {
                if *phy_id / chip_count != npu_id {
                    continue;
                }
                let chip_id = *phy_id % chip_count;
                stats.power_w = powers
                    .iter()
                    .find(|(c, _)| *c == chip_id)
                    .map(|(_, w)| *w)
                    .or(card_power);
            }
        }
    }
    map
}

/// Parse `npu-smi info -t power`. Chip ID `< 0` means card-level aggregate.
fn parse_power_by_chip(text: &str) -> Vec<(i32, f32)> {
    let mut out = Vec::new();
    let mut cur_chip: Option<i32> = None;
    let mut saw_chip = false;
    for line in text.lines() {
        if let Some(id) = parse_kv_i32(line, &["Chip ID"]) {
            cur_chip = Some(id);
            saw_chip = true;
            continue;
        }
        if let Some(w) = parse_power_watts_line(line) {
            let chip = if saw_chip { cur_chip.unwrap_or(0) } else { -1 };
            out.push((chip, w));
        }
    }
    out
}

fn parse_power_watts_line(line: &str) -> Option<f32> {
    let (key, value) = split_kv(line)?;
    if !key_matches(
        key,
        &[
            "NPU Real-time Power",
            "NPU Power",
            "Power Dissipation",
            "Power",
        ],
    ) {
        return None;
    }
    // Avoid matching "Power Limit" style keys when possible.
    let kl = key.to_ascii_lowercase();
    if kl.contains("limit") || kl.contains("cap") {
        return None;
    }
    value
        .split_whitespace()
        .next()?
        .trim_end_matches('W')
        .parse::<f32>()
        .ok()
        .filter(|v| *v >= 0.0)
}

#[derive(Debug, Default)]
struct ChipUsage {
    chip_id: i32,
    aicore_util_pct: f32,
    npu_util_pct: f32,
    aivector_util_pct: f32,
    hbm_usage_rate: f32,
    hbm_bw_util_pct: f32,
    hbm_capacity_mb: u64,
}

fn parse_usage_chips(text: &str) -> Vec<ChipUsage> {
    let mut chips = Vec::new();
    let mut cur = ChipUsage {
        aicore_util_pct: -1.0,
        npu_util_pct: -1.0,
        aivector_util_pct: -1.0,
        hbm_usage_rate: -1.0,
        hbm_bw_util_pct: -1.0,
        ..Default::default()
    };
    let mut saw = false;

    for line in text.lines() {
        if let Some(v) = parse_percent_line(
            line,
            &[
                "Aicore Usage Rate",
                "Ai Core Usage Rate",
                "AICore Usage Rate",
            ],
        ) {
            cur.aicore_util_pct = v;
            saw = true;
        } else if let Some(v) = parse_percent_line(line, &["NPU Utilization"]) {
            cur.npu_util_pct = v;
            saw = true;
        } else if let Some(v) =
            parse_percent_line(line, &["Aivector Usage Rate", "AI Vector Usage Rate"])
        {
            cur.aivector_util_pct = v;
            saw = true;
        } else if let Some(v) = parse_percent_line(line, &["HBM Bandwidth Usage Rate"]) {
            cur.hbm_bw_util_pct = v;
            saw = true;
        } else if let Some(v) = parse_percent_line(line, &["HBM Usage Rate", "Memory Usage Rate"]) {
            // Prefer the capacity fill %; do not overwrite with bandwidth %.
            if !key_matches(
                split_kv(line).map(|(k, _)| k).unwrap_or(""),
                &["HBM Bandwidth Usage Rate"],
            ) {
                cur.hbm_usage_rate = v;
                saw = true;
            }
        } else if let Some(mb) = parse_memory_mb_line(line, &["HBM Capacity"]) {
            cur.hbm_capacity_mb = mb;
            saw = true;
        } else if let Some(id) = parse_kv_i32(line, &["Chip ID"]) {
            cur.chip_id = id;
            if saw {
                chips.push(std::mem::take(&mut cur));
                cur = ChipUsage {
                    aicore_util_pct: -1.0,
                    npu_util_pct: -1.0,
                    aivector_util_pct: -1.0,
                    hbm_usage_rate: -1.0,
                    hbm_bw_util_pct: -1.0,
                    ..Default::default()
                };
                saw = false;
            }
        }
    }
    if saw {
        chips.push(cur);
    }
    chips
}

pub(super) fn list_npu_card_ids() -> Vec<i32> {
    let Some(output) = run_npu_smi(&["info", "-l"]) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut ids = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(id) = parse_kv_i32(line, &["NPU ID", "NpuID", "Device ID"]) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();
    ids
}

fn npu_smi_available() -> bool {
    run_npu_smi(&["--help"])
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false)
}

fn normalize_key(key: &str) -> String {
    key.trim()
        .trim_end_matches("(%)")
        .trim_end_matches("(MB)")
        .trim_end_matches("(page)")
        .trim_end_matches("(C)")
        .trim_end_matches("(MHz)")
        .trim()
        .to_string()
}

fn key_matches(key: &str, patterns: &[&str]) -> bool {
    let norm = normalize_key(key);
    patterns.iter().any(|p| {
        norm.eq_ignore_ascii_case(p) || norm.to_ascii_lowercase().contains(&p.to_ascii_lowercase())
    })
}

fn parse_kv_i32(line: &str, keys: &[&str]) -> Option<i32> {
    let (key, value) = split_kv(line)?;
    if !key_matches(key, keys) {
        return None;
    }
    value.split_whitespace().next()?.parse().ok()
}

fn parse_kv_i32_from_text(text: &str, keys: &[&str]) -> Option<i32> {
    text.lines().find_map(|line| parse_kv_i32(line, keys))
}

fn parse_kv_str(line: &str, keys: &[&str]) -> Option<String> {
    let (key, value) = split_kv(line)?;
    if !key_matches(key, keys) {
        return None;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim()))
}

fn parse_percent_line(line: &str, keys: &[&str]) -> Option<f32> {
    let (key, value) = split_kv(line)?;
    if !key_matches(key, keys) {
        return None;
    }
    let num = value.split_whitespace().next()?;
    num.trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .map(|v| v.clamp(0.0, 100.0))
}

fn parse_memory_mb_line(line: &str, keys: &[&str]) -> Option<u64> {
    let (key, value) = split_kv(line)?;
    if !key_matches(key, keys) {
        return None;
    }
    let num = value.split_whitespace().next()?;
    num.parse::<f64>().ok().map(|v| v.round() as u64)
}

fn parse_memory_mb(text: &str, keys: &[&str]) -> u64 {
    text.lines()
        .find_map(|line| parse_memory_mb_line(line, keys))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_real_a3_usages_block() {
        let text = r#"
	NPU ID                         : 0
	Chip Count                     : 2

	DDR Capacity(MB)               : 0
	HBM Capacity(MB)               : 65536
	HBM Usage Rate(%)              : 16
	HBM Bandwidth Usage Rate(%)    : 27
	Aicore Usage Rate(%)           : 2
	Aivector Usage Rate(%)         : 31
	NPU Utilization(%)             : 65
	Chip ID                        : 0

	HBM Capacity(MB)               : 65536
	HBM Usage Rate(%)              : 16
	Aicore Usage Rate(%)           : 12
	NPU Utilization(%)             : 58
	Chip ID                        : 1
        "#;
        let chips = parse_usage_chips(text);
        assert_eq!(chips.len(), 2);
        assert_eq!(chips[0].chip_id, 0);
        assert_eq!(chips[0].npu_util_pct, 65.0);
        assert_eq!(chips[0].aicore_util_pct, 2.0);
        assert_eq!(chips[0].hbm_usage_rate, 16.0);
        assert_eq!(chips[0].hbm_bw_util_pct, 27.0);
        assert_eq!(chips[0].aivector_util_pct, 31.0);
        assert_eq!(chips[0].hbm_capacity_mb, 65536);
        assert_eq!(chips[1].chip_id, 1);
        assert_eq!(chips[1].npu_util_pct, 58.0);
    }

    #[test]
    fn key_matches_strips_units() {
        assert!(key_matches("Aicore Usage Rate(%)", &["Aicore Usage Rate"]));
        assert!(key_matches("HBM Capacity(MB)", &["HBM Capacity"]));
        assert!(key_matches("NPU Utilization(%)", &["NPU Utilization"]));
    }

    #[test]
    fn parse_npu_id_line() {
        assert_eq!(
            parse_kv_i32("        NPU ID                         : 3", &["NPU ID"]),
            Some(3)
        );
    }

    #[test]
    fn parse_power_ignores_limit() {
        assert_eq!(
            parse_power_watts_line("NPU Real-time Power : 164.5 W"),
            Some(164.5)
        );
        assert_eq!(parse_power_watts_line("Power Limit : 300 W"), None);
    }
}
