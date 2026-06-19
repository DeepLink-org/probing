use dioxus::prelude::*;

use crate::components::flamegraph::FlamegraphPayload;

const MAX_SNAPSHOTS: usize = 8;

#[derive(Clone, PartialEq)]
pub struct ProfileSnapshot {
    pub id: u64,
    pub captured_at_ms: u64,
    pub profiler: String,
    pub metric: Option<String>,
    pub title: String,
    pub payload: FlamegraphPayload,
}

pub static PROFILE_SNAPSHOTS: GlobalSignal<Vec<ProfileSnapshot>> = Signal::global(Vec::new);
pub static PROFILE_DIFF_BASELINE: GlobalSignal<Option<u64>> = Signal::global(|| None);

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

pub fn push_profile_snapshot(profiler: &str, metric: Option<&str>, payload: FlamegraphPayload) {
    if payload.frames.is_empty() {
        return;
    }
    let id = now_ms();
    let title = payload.title.clone();
    let snap = ProfileSnapshot {
        id,
        captured_at_ms: id,
        profiler: profiler.to_string(),
        metric: metric.map(str::to_string),
        title,
        payload,
    };
    let mut list = PROFILE_SNAPSHOTS.read().clone();
    list.retain(|s| !(s.profiler == snap.profiler && s.metric == snap.metric));
    list.insert(0, snap);
    list.truncate(MAX_SNAPSHOTS);
    *PROFILE_SNAPSHOTS.write() = list;
}

pub fn snapshot_label(snap: &ProfileSnapshot) -> String {
    let secs_ago = now_ms().saturating_sub(snap.captured_at_ms) / 1000;
    let ago = if secs_ago < 60 {
        format!("{secs_ago}s ago")
    } else {
        format!("{}m ago", secs_ago / 60)
    };
    format!("{} · {ago}", snap.title)
}
