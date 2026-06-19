use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SliceKey {
    pub pid: u32,
    pub tid: u32,
    pub start_bits: u64,
}

impl SliceKey {
    pub fn from_slice(s: &TimelineSlice) -> Self {
        Self {
            pid: s.pid,
            tid: s.tid,
            start_bits: s.start_us.to_bits(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineSlice {
    pub name: String,
    pub cat: String,
    pub start_us: f64,
    pub dur_us: f64,
    pub pid: u32,
    pub tid: u32,
    pub args: Option<Value>,
    pub children: Vec<TimelineSlice>,
}

impl TimelineSlice {
    pub fn end_us(&self) -> f64 {
        self.start_us + self.dur_us
    }

    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    pub fn descendant_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(TimelineSlice::descendant_count)
            .sum::<usize>()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineTrack {
    pub pid: u32,
    pub tid: u32,
    pub label: String,
    /// Root-level slices; nested spans live in `children`.
    pub slices: Vec<TimelineSlice>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TimelineModel {
    pub tracks: Vec<TimelineTrack>,
    pub min_ts_us: f64,
    pub max_ts_us: f64,
    pub event_count: usize,
}

impl TimelineModel {
    pub fn range_us(&self) -> f64 {
        (self.max_ts_us - self.min_ts_us).max(1.0)
    }

    pub fn is_empty(&self) -> bool {
        self.event_count == 0
    }
}

pub fn count_slices_in_tracks(tracks: &[TimelineTrack]) -> usize {
    tracks
        .iter()
        .flat_map(|t| t.slices.iter())
        .map(TimelineSlice::descendant_count)
        .sum()
}
