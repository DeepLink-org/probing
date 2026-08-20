//! Parse Ascend HCCS (NVLink analogue) counters via `npu-smi -t hccs`.

use super::npu_smi::{list_npu_card_ids, run_npu_smi_text};

/// One chip's HCCS snapshot (cumulative counters + health).
#[derive(Debug, Clone, Default)]
pub struct HccsChipSample {
    pub npu_id: i32,
    pub chip_id: i32,
    /// Logical Phy-ID matching `gpu.utilization.device_id` (NPU*chip_count + chip).
    pub device_id: i32,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub error_count: u64,
    pub retry_count: u64,
    pub health_ok: bool,
}

/// Optional instantaneous bandwidth from `npu-smi -t hccs-bw` (expensive).
#[derive(Debug, Clone, Default)]
pub struct HccsBwSample {
    pub tx_gbs: f32,
    pub rx_gbs: f32,
}

/// Sum a bracket list like `[a b c]` or space-separated numbers on a value line.
fn sum_u64_list(value: &str) -> u64 {
    let cleaned = value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .replace(',', " ");
    cleaned
        .split_whitespace()
        .filter_map(|t| t.parse::<u64>().ok())
        .sum()
}

fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(':')?;
    Some((k.trim(), v.trim()))
}

/// Discover (npu_id, chip_id, device_id) triples once; reuse across ticks.
pub fn discover_hccs_targets() -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for npu_id in list_npu_card_ids() {
        let chip_ids = discover_chip_ids(npu_id);
        let chip_count = chip_ids.len().max(1) as i32;
        for chip_id in chip_ids {
            out.push((npu_id, chip_id, npu_id * chip_count + chip_id));
        }
    }
    out
}

/// Sample HCCS counters for one chip.
pub fn sample_hccs_chip(npu_id: i32, chip_id: i32, device_id: i32) -> Option<HccsChipSample> {
    let text = run_npu_smi_text(&[
        "info",
        "-t",
        "hccs",
        "-i",
        &npu_id.to_string(),
        "-c",
        &chip_id.to_string(),
    ])?;
    let mut sample = HccsChipSample {
        npu_id,
        chip_id,
        device_id,
        health_ok: true,
        ..Default::default()
    };
    let mut saw = false;
    for line in text.lines() {
        let Some((k, v)) = parse_kv(line) else {
            continue;
        };
        let kl = k.to_ascii_lowercase();
        if kl.contains("hccs tx bytes") {
            sample.tx_bytes = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs rx bytes") {
            sample.rx_bytes = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs tx packets") {
            sample.tx_packets = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs rx packets") {
            sample.rx_packets = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs error count") {
            sample.error_count = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs retry count") {
            sample.retry_count = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs health") {
            sample.health_ok = v.to_ascii_uppercase().contains("OK");
            saw = true;
        }
    }
    saw.then_some(sample)
}

/// Parse `hccs-bw` total row (GB/s). Slow (~1s/chip); call sparingly.
pub fn sample_hccs_bw(npu_id: i32, chip_id: i32) -> Option<HccsBwSample> {
    let text = run_npu_smi_text(&[
        "info",
        "-t",
        "hccs-bw",
        "-i",
        &npu_id.to_string(),
        "-c",
        &chip_id.to_string(),
    ])?;
    let mut tx = -1.0_f32;
    let mut rx = -1.0_f32;
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("total") {
            if let (Ok(t), Ok(r)) = (parts[1].parse::<f32>(), parts[2].parse::<f32>()) {
                tx = t;
                rx = r;
            }
        }
    }
    if tx < 0.0 && rx < 0.0 {
        None
    } else {
        Some(HccsBwSample {
            tx_gbs: tx.max(0.0),
            rx_gbs: rx.max(0.0),
        })
    }
}

fn discover_chip_ids(npu_id: i32) -> Vec<i32> {
    let Some(text) = run_npu_smi_text(&["info", "-t", "usages", "-i", &npu_id.to_string()]) else {
        return vec![0];
    };
    let mut chips: Vec<i32> = text
        .lines()
        .filter_map(|line| {
            let (k, v) = parse_kv(line)?;
            if k.eq_ignore_ascii_case("Chip ID") {
                v.split_whitespace().next()?.parse().ok()
            } else {
                None
            }
        })
        .collect();
    chips.sort_unstable();
    chips.dedup();
    if chips.is_empty() {
        vec![0]
    } else {
        chips
    }
}

/// Parse a multi-line HCCS block (unit tests / offline).
pub fn parse_hccs_text(
    text: &str,
    npu_id: i32,
    chip_id: i32,
    device_id: i32,
) -> Option<HccsChipSample> {
    let mut sample = HccsChipSample {
        npu_id,
        chip_id,
        device_id,
        health_ok: true,
        ..Default::default()
    };
    let mut saw = false;
    for line in text.lines() {
        let Some((k, v)) = parse_kv(line) else {
            continue;
        };
        let kl = k.to_ascii_lowercase();
        if kl.contains("hccs tx bytes") {
            sample.tx_bytes = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs rx bytes") {
            sample.rx_bytes = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs tx packets") {
            sample.tx_packets = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs rx packets") {
            sample.rx_packets = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs error count") {
            sample.error_count = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs retry count") {
            sample.retry_count = sum_u64_list(v);
            saw = true;
        } else if kl.contains("hccs health") {
            sample.health_ok = v.to_ascii_uppercase().contains("OK");
            saw = true;
        }
    }
    saw.then_some(sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_bracket_list() {
        assert_eq!(sum_u64_list("[10 20 30]"), 60);
        assert_eq!(sum_u64_list("5"), 5);
    }

    #[test]
    fn parse_real_hccs_block() {
        let text = r#"
	hccs health status             : OK
	hccs tx packets                : [1993002168   1434439505   1588783520]
	hccs tx bytes                  : [39860043360  28688790100  31775670400]
	hccs rx packets                : [2091328898   760745014    4126456722]
	hccs rx bytes                  : [41826577960  15214900280  82529134440]
	hccs retry count               : [0            0            0]
	hccs error count               : [0            0            0]
        "#;
        let s = parse_hccs_text(text, 0, 0, 0).expect("parse");
        assert!(s.health_ok);
        assert_eq!(s.tx_bytes, 39860043360 + 28688790100 + 31775670400);
        assert_eq!(s.rx_packets, 2091328898 + 760745014 + 4126456722);
        assert_eq!(s.error_count, 0);
    }
}
