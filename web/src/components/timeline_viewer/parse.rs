use std::collections::HashMap;

use serde_json::Value;

use super::model::{count_slices_in_tracks, TimelineModel, TimelineSlice, TimelineTrack};

struct RawEvent {
    name: String,
    cat: String,
    ph: String,
    ts: f64,
    dur: Option<f64>,
    pid: u32,
    tid: u32,
    args: Option<Value>,
}

struct OpenSlice {
    name: String,
    cat: String,
    ts: f64,
    pid: u32,
    tid: u32,
    args: Option<Value>,
    children: Vec<TimelineSlice>,
}

pub fn parse_chrome_trace(json: &str) -> Result<TimelineModel, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("Invalid trace JSON: {e}"))?;
    let events = extract_trace_events(&root)?;
    if events.is_empty() {
        return Ok(TimelineModel::default());
    }

    let mut process_names: HashMap<u32, String> = HashMap::new();
    let mut thread_names: HashMap<(u32, u32), String> = HashMap::new();
    let mut raw: Vec<RawEvent> = Vec::new();

    for ev in events {
        let Some(obj) = ev.as_object() else {
            continue;
        };
        let ph = json_str(obj, "ph");
        if ph.is_empty() {
            continue;
        }

        if ph == "M" {
            let name = json_str(obj, "name");
            let pid = json_u32(obj, "pid");
            let tid = json_u32(obj, "tid");
            if let Some(args) = obj.get("args").and_then(|v| v.as_object()) {
                if name == "process_name" {
                    if let Some(label) = args.get("name").and_then(|v| v.as_str()) {
                        process_names.insert(pid, label.to_string());
                    }
                } else if name == "thread_name" {
                    if let Some(label) = args.get("name").and_then(|v| v.as_str()) {
                        thread_names.insert((pid, tid), label.to_string());
                    }
                }
            }
            continue;
        }

        raw.push(RawEvent {
            name: json_str(obj, "name"),
            cat: json_str(obj, "cat"),
            ph,
            ts: json_f64(obj, "ts"),
            dur: obj.get("dur").and_then(|v| v.as_f64()),
            pid: json_u32(obj, "pid"),
            tid: json_u32(obj, "tid"),
            args: obj.get("args").cloned(),
        });
    }

    raw.sort_by(|a, b| {
        a.ts.partial_cmp(&b.ts)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.ph.cmp(&b.ph))
    });

    let mut roots_by_track: HashMap<(u32, u32), Vec<TimelineSlice>> = HashMap::new();
    let mut stacks: HashMap<(u32, u32), Vec<OpenSlice>> = HashMap::new();

    for ev in raw {
        let track_key = (ev.pid, ev.tid);
        match ev.ph.as_str() {
            "X" | "x" => {
                let dur = ev.dur.filter(|d| *d > 0.0).unwrap_or(0.0);
                if dur > 0.0 {
                    push_root_slice(
                        &mut roots_by_track,
                        track_key,
                        make_leaf_slice(&ev, ev.ts, dur),
                    );
                }
            }
            "B" | "b" => {
                stacks.entry(track_key).or_default().push(OpenSlice {
                    name: ev.name,
                    cat: ev.cat,
                    ts: ev.ts,
                    pid: ev.pid,
                    tid: ev.tid,
                    args: ev.args,
                    children: Vec::new(),
                });
            }
            "E" | "e" => {
                let Some(open) = stacks.get_mut(&track_key).and_then(|s| s.pop()) else {
                    continue;
                };
                let dur = ev.dur.filter(|d| *d > 0.0).unwrap_or(ev.ts - open.ts);
                if dur <= 0.0 {
                    continue;
                }
                let slice = TimelineSlice {
                    name: open.name,
                    cat: open.cat,
                    start_us: open.ts,
                    dur_us: dur,
                    pid: open.pid,
                    tid: open.tid,
                    args: open.args,
                    children: open.children,
                };
                if let Some(parent) = stacks.get_mut(&track_key).and_then(|s| s.last_mut()) {
                    parent.children.push(slice);
                } else {
                    push_root_slice(&mut roots_by_track, track_key, slice);
                }
            }
            "i" | "I" => {
                push_root_slice(
                    &mut roots_by_track,
                    track_key,
                    make_leaf_slice(&ev, ev.ts, 0.0),
                );
            }
            _ => {}
        }
    }

    if roots_by_track.is_empty() {
        return Ok(TimelineModel::default());
    }

    let min_ts_us = roots_by_track
        .values()
        .flat_map(|slices| slices.iter())
        .flat_map(flatten_slice_times)
        .fold(f64::INFINITY, f64::min);
    let max_ts_us = roots_by_track
        .values()
        .flat_map(|slices| slices.iter())
        .flat_map(flatten_slice_times)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut tracks: Vec<TimelineTrack> = roots_by_track
        .into_iter()
        .map(|((pid, tid), mut track_slices)| {
            sort_slice_tree(&mut track_slices);
            let proc = process_names
                .get(&pid)
                .map(String::as_str)
                .unwrap_or("process");
            let thread = thread_names
                .get(&(pid, tid))
                .map(String::as_str)
                .unwrap_or("");
            let label = if thread.is_empty() {
                format!("{proc} · tid {tid}")
            } else {
                format!("{proc} · {thread}")
            };
            TimelineTrack {
                pid,
                tid,
                label,
                slices: track_slices,
            }
        })
        .collect();

    tracks.sort_by(|a, b| a.label.cmp(&b.label));
    let event_count = count_slices_in_tracks(&tracks);

    Ok(TimelineModel {
        tracks,
        min_ts_us,
        max_ts_us,
        event_count,
    })
}

fn make_leaf_slice(ev: &RawEvent, start_us: f64, dur_us: f64) -> TimelineSlice {
    TimelineSlice {
        name: ev.name.clone(),
        cat: ev.cat.clone(),
        start_us,
        dur_us,
        pid: ev.pid,
        tid: ev.tid,
        args: ev.args.clone(),
        children: Vec::new(),
    }
}

fn push_root_slice(
    roots: &mut HashMap<(u32, u32), Vec<TimelineSlice>>,
    key: (u32, u32),
    slice: TimelineSlice,
) {
    roots.entry(key).or_default().push(slice);
}

fn flatten_slice_times(slice: &TimelineSlice) -> Vec<f64> {
    let mut times = vec![slice.start_us, slice.end_us()];
    for child in &slice.children {
        times.extend(flatten_slice_times(child));
    }
    times
}

fn sort_slice_tree(slices: &mut [TimelineSlice]) {
    slices.sort_by(|a, b| {
        a.start_us
            .partial_cmp(&b.start_us)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for slice in slices.iter_mut() {
        sort_slice_tree(&mut slice.children);
    }
}

fn extract_trace_events(root: &Value) -> Result<Vec<Value>, String> {
    if let Some(arr) = root.as_array() {
        return Ok(arr.clone());
    }
    if let Some(arr) = root.get("traceEvents").and_then(|v| v.as_array()) {
        return Ok(arr.clone());
    }
    Err("Trace JSON must be an array or contain traceEvents".to_string())
}

fn json_str(obj: &serde_json::Map<String, Value>, key: &str) -> String {
    obj.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_u32(obj: &serde_json::Map<String, Value>, key: &str) -> u32 {
    obj.get(key).and_then(|v| v.as_u64()).unwrap_or(0) as u32
}

fn json_f64(obj: &serde_json::Map<String, Value>, key: &str) -> f64 {
    obj.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_begin_end_events() {
        let json = r#"{
            "traceEvents": [
                {"name": "a", "cat": "cat", "ph": "B", "ts": 100, "pid": 1, "tid": 1},
                {"name": "a", "cat": "cat", "ph": "E", "ts": 250, "pid": 1, "tid": 1}
            ]
        }"#;
        let model = parse_chrome_trace(json).unwrap();
        assert_eq!(model.event_count, 1);
        assert!((model.tracks[0].slices[0].dur_us - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn nests_begin_end_hierarchy() {
        let json = r#"{
            "traceEvents": [
                {"name": "outer", "cat": "span", "ph": "B", "ts": 0, "pid": 1, "tid": 1},
                {"name": "inner", "cat": "span", "ph": "B", "ts": 50, "pid": 1, "tid": 1},
                {"name": "inner", "cat": "span", "ph": "E", "ts": 150, "pid": 1, "tid": 1},
                {"name": "outer", "cat": "span", "ph": "E", "ts": 200, "pid": 1, "tid": 1}
            ]
        }"#;
        let model = parse_chrome_trace(json).unwrap();
        assert_eq!(model.event_count, 2);
        let outer = &model.tracks[0].slices[0];
        assert_eq!(outer.name, "outer");
        assert_eq!(outer.children.len(), 1);
        assert_eq!(outer.children[0].name, "inner");
    }
}
