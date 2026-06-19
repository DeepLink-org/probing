use std::collections::HashMap;

use super::logic::index_frames;
use super::model::{FlameFrame, FlamegraphPayload};

#[derive(Clone, Debug, PartialEq)]
pub struct FrameDelta {
    pub path: String,
    pub current: u64,
    pub baseline: u64,
    pub delta: i64,
}

pub fn frame_path(frame: &FlameFrame, by_id: &HashMap<usize, FlameFrame>) -> String {
    let mut parts = vec![frame.name.clone()];
    let mut parent = frame.parent;
    while let Some(pid) = parent {
        if let Some(p) = by_id.get(&pid) {
            parts.push(p.name.clone());
            parent = p.parent;
        } else {
            break;
        }
    }
    parts.reverse();
    parts.join(" › ")
}

pub fn value_map(payload: &FlamegraphPayload) -> HashMap<String, u64> {
    let by_id = index_frames(&payload.frames);
    payload
        .frames
        .iter()
        .filter(|f| f.depth > 0)
        .map(|f| (frame_path(f, &by_id), f.value))
        .fold(HashMap::new(), |mut acc, (path, value)| {
            acc.entry(path)
                .and_modify(|v| *v = v.saturating_add(value))
                .or_insert(value);
            acc
        })
}

pub fn compute_frame_deltas(
    current: &FlamegraphPayload,
    baseline: &FlamegraphPayload,
) -> Vec<FrameDelta> {
    let cur = value_map(current);
    let base = value_map(baseline);
    let mut keys: HashMap<String, ()> = HashMap::new();
    for k in cur.keys().chain(base.keys()) {
        keys.insert(k.clone(), ());
    }

    let mut deltas: Vec<FrameDelta> = keys
        .keys()
        .map(|path| {
            let c = *cur.get(path).unwrap_or(&0);
            let b = *base.get(path).unwrap_or(&0);
            FrameDelta {
                path: path.clone(),
                current: c,
                baseline: b,
                delta: c as i64 - b as i64,
            }
        })
        .filter(|d| d.delta != 0)
        .collect();

    deltas.sort_by_key(|b| std::cmp::Reverse(b.delta.unsigned_abs()));
    deltas.truncate(30);
    deltas
}
