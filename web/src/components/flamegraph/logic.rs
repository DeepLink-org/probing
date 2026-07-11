use std::collections::{HashMap, HashSet};

use super::model::FlameFrame;

pub fn index_frames(frames: &[FlameFrame]) -> HashMap<usize, FlameFrame> {
    frames.iter().map(|f| (f.id, f.clone())).collect()
}

pub fn child_map(frames: &[FlameFrame]) -> HashMap<usize, Vec<usize>> {
    let mut children = HashMap::new();
    for f in frames {
        if let Some(parent) = f.parent {
            children.entry(parent).or_insert_with(Vec::new).push(f.id);
        }
    }
    children
}

pub fn descendants(children: &HashMap<usize, Vec<usize>>, id: usize) -> HashSet<usize> {
    let mut out = HashSet::new();
    out.insert(id);
    if let Some(kids) = children.get(&id) {
        for child in kids {
            for x in descendants(children, *child) {
                out.insert(x);
            }
        }
    }
    out
}

pub fn ancestor_ids(by_id: &HashMap<usize, FlameFrame>, id: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut cur = Some(id);
    while let Some(frame_id) = cur {
        out.push(frame_id);
        cur = by_id.get(&frame_id).and_then(|f| f.parent);
    }
    out.reverse();
    out
}

pub fn classic_color(name: &str, depth: usize) -> String {
    let mut h = 0u32;
    for b in name.bytes() {
        h = h.wrapping_mul(37).wrapping_add(b as u32);
    }
    let r = (205 + (h % 55)) % 256;
    let g = (40 + ((h >> 8) % 120)) % 256;
    let depth = depth as u32;
    let b = (30 + ((h >> 16) % 80) + depth * 4) % 256;
    format!("rgb({r},{g},{b})")
}

pub fn phase_color(phase: &str, depth: usize) -> String {
    let base: [u8; 3] = match phase {
        "forward" => [59, 130, 246],
        "step" => [245, 158, 11],
        "backward" => [168, 85, 247],
        _ => [100, 116, 139],
    };
    let fade = depth.min(40) as u8;
    format!(
        "rgb({}, {}, {})",
        base[0] + fade,
        base[1] + fade,
        base[2] + fade
    )
}

pub fn format_duration_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2} s", ns as f64 / 1e9)
    } else if ns >= 1_000_000 {
        format!("{:.2} ms", ns as f64 / 1e6)
    } else if ns >= 1_000 {
        format!("{:.1} µs", ns as f64 / 1e3)
    } else {
        format!("{ns} ns")
    }
}

pub fn format_pct(value: u64, total: u64) -> String {
    if total == 0 {
        "0".to_string()
    } else {
        format!("{:.2}", 100.0 * value as f64 / total as f64)
    }
}

pub fn phase_label(phase: &str) -> &str {
    match phase {
        "forward" => "Forward",
        "step" => "Optimizer",
        "backward" => "Backward",
        "all" => "All phases",
        _ => phase,
    }
}

pub fn label_for_frame(frame: &FlameFrame) -> String {
    if frame.depth >= 2 {
        frame
            .module_path
            .clone()
            .unwrap_or_else(|| frame.name.clone())
    } else if frame.depth == 1 {
        match frame.name.as_str() {
            "forward" => "Forward pass".to_string(),
            "backward" => "Backward pass".to_string(),
            "step" => "Optimizer step".to_string(),
            other => other.to_string(),
        }
    } else {
        frame.name.clone()
    }
}

pub fn matches_search(frame: &FlameFrame, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    frame.name.to_lowercase().contains(&q)
        || frame
            .module_path
            .as_ref()
            .is_some_and(|p| p.to_lowercase().contains(&q))
}

pub fn format_frame_value(value: u64, count_name: &str) -> String {
    if count_name == "ns" {
        format_duration_ns(value)
    } else if count_name == "MB" {
        format_memory_mb(value)
    } else {
        format!("{value} {count_name}")
    }
}

/// Values are stored as micro-MB (MB × 1e6) for torch memory metrics.
pub fn format_memory_mb(micro_mb: u64) -> String {
    let mb = micro_mb as f64 / 1_000_000.0;
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else if mb >= 1.0 {
        format!("{:.2} MB", mb)
    } else if mb >= 0.001 {
        format!("{:.1} KB", mb * 1024.0)
    } else {
        format!("{:.3} MB", mb)
    }
}

pub fn metric_value_label(metric: Option<&str>, count_name: &str) -> &'static str {
    match metric {
        Some("delta_mb") => "Δ Memory",
        Some("peak_mb") => "Peak Δ",
        Some("duration") => "Duration",
        _ if count_name == "ns" => "Duration",
        _ if count_name == "MB" => "Memory",
        _ => "Value",
    }
}

pub const TORCH_METRICS: [(&str, &str); 3] = [
    ("duration", "Time"),
    ("delta_mb", "Δ Memory"),
    ("peak_mb", "Peak"),
];

pub fn is_torch_profile(profile: &str) -> bool {
    profile == "torch-module"
}

pub fn frame_fill_color(profile: &str, frame: &FlameFrame) -> String {
    if is_torch_profile(profile) {
        let phase = frame.phase.as_deref().unwrap_or("other");
        phase_color(phase, frame.depth)
    } else {
        classic_color(&frame.name, frame.depth)
    }
}

pub fn search_placeholder(profile: &str) -> &'static str {
    if is_torch_profile(profile) {
        "Filter modules…"
    } else {
        "Filter stacks…"
    }
}

pub fn leaf_count_label(profile: &str) -> &'static str {
    if is_torch_profile(profile) {
        "Modules"
    } else {
        "Frames"
    }
}

pub fn list_phases(frames: &[FlameFrame]) -> Vec<String> {
    let mut phases = vec!["all".to_string()];
    for f in frames {
        if f.depth == 1 && f.name != "all" && !phases.iter().any(|p| p == &f.name) {
            phases.push(f.name.clone());
        }
    }
    phases
}

/// Whether a pprof depth-1 frame name matches an OS thread id (`thread-{tid}` or `thread-{tid} (name)`).
pub fn frame_matches_thread_tid(name: &str, tid: i32) -> bool {
    let prefix = format!("thread-{tid}");
    if !name.starts_with(&prefix) {
        return false;
    }
    name.len() == prefix.len()
        || name.as_bytes().get(prefix.len()) == Some(&b' ')
        || name.as_bytes().get(prefix.len()) == Some(&b'(')
}

pub fn thread_root_for_frame(by_id: &HashMap<usize, FlameFrame>, id: usize) -> Option<usize> {
    let mut cur = Some(id);
    while let Some(frame_id) = cur {
        let frame = by_id.get(&frame_id)?;
        if frame.depth == 1 {
            return Some(frame_id);
        }
        if frame.depth == 0 {
            return None;
        }
        cur = frame.parent;
    }
    None
}

pub fn frame_visible_for_thread(
    by_id: &HashMap<usize, FlameFrame>,
    frame: &FlameFrame,
    tid: i32,
) -> bool {
    if frame.depth == 0 {
        return true;
    }
    thread_root_for_frame(by_id, frame.id)
        .and_then(|root_id| by_id.get(&root_id))
        .is_some_and(|root| frame_matches_thread_tid(&root.name, tid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_tid_frame_names() {
        assert!(frame_matches_thread_tid("thread-42", 42));
        assert!(frame_matches_thread_tid("thread-42 (main)", 42));
        assert!(!frame_matches_thread_tid("thread-420", 42));
        assert!(!frame_matches_thread_tid("thread-4", 42));
    }
}
