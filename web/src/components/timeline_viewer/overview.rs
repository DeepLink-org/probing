use super::model::{SliceKey, TimelineModel, TimelineSlice, TimelineTrack};

pub const OVERVIEW_BUCKET_COUNT: usize = 56;

#[derive(Clone, Debug, PartialEq)]
pub struct TimeRange {
    pub start_us: f64,
    pub end_us: f64,
}

impl TimeRange {
    pub fn overlaps(&self, start_us: f64, end_us: f64) -> bool {
        end_us > self.start_us && start_us < self.end_us
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TimelineBarItem {
    OverviewBucket {
        start_us: f64,
        end_us: f64,
        count: usize,
        label: String,
        cat: String,
    },
    Slice {
        slice: TimelineSlice,
        expandable: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TimelineViewState {
    pub expanded_range: Option<TimeRange>,
    pub drill_path: Vec<SliceKey>,
}

impl TimelineViewState {
    pub fn is_overview(&self) -> bool {
        self.expanded_range.is_none() && self.drill_path.is_empty()
    }

    pub fn reset(&mut self) {
        self.expanded_range = None;
        self.drill_path.clear();
    }

    pub fn pop_drill(&mut self) {
        if !self.drill_path.is_empty() {
            self.drill_path.pop();
            return;
        }
        self.expanded_range = None;
    }
}

pub fn build_track_bars(
    track: &TimelineTrack,
    state: &TimelineViewState,
    model: &TimelineModel,
    force_detail: bool,
) -> Vec<TimelineBarItem> {
    let level = visible_level_slices(track, &state.drill_path);
    if !force_detail && state.is_overview() {
        return bucket_slices(level, model.min_ts_us, model.max_ts_us, OVERVIEW_BUCKET_COUNT);
    }

    let filtered: Vec<TimelineSlice> = if let Some(range) = &state.expanded_range {
        level
            .iter()
            .filter(|s| range.overlaps(s.start_us, s.end_us()))
            .cloned()
            .collect()
    } else {
        level.to_vec()
    };

    filtered
        .into_iter()
        .map(|slice| TimelineBarItem::Slice {
            expandable: slice.has_children(),
            slice,
        })
        .collect()
}

fn visible_level_slices<'a>(track: &'a TimelineTrack, drill_path: &[SliceKey]) -> &'a [TimelineSlice] {
    let mut level = track.slices.as_slice();
    for key in drill_path {
        let Some(slice) = level.iter().find(|s| SliceKey::from_slice(s) == *key) else {
            return &[];
        };
        level = &slice.children;
    }
    level
}

fn bucket_slices(
    slices: &[TimelineSlice],
    min_ts: f64,
    max_ts: f64,
    bucket_count: usize,
) -> Vec<TimelineBarItem> {
    if slices.is_empty() {
        return Vec::new();
    }

    let range = (max_ts - min_ts).max(1.0);
    let bucket_width = range / bucket_count as f64;
    let mut buckets: Vec<(f64, f64, Vec<&TimelineSlice>)> = (0..bucket_count)
        .map(|i| {
            let start = min_ts + bucket_width * i as f64;
            let end = if i + 1 == bucket_count {
                max_ts
            } else {
                min_ts + bucket_width * (i + 1) as f64
            };
            (start, end, Vec::new())
        })
        .collect();

    for slice in slices {
        let mid = slice.start_us + slice.dur_us * 0.5;
        let idx = ((mid - min_ts) / bucket_width).floor() as usize;
        let idx = idx.min(bucket_count - 1);
        buckets[idx].2.push(slice);
    }

    buckets
        .into_iter()
        .filter(|(_, _, items)| !items.is_empty())
        .map(|(start_us, end_us, items)| {
            let count = items.len();
            let dominant = items
                .iter()
                .copied()
                .max_by(|a, b| a.dur_us.partial_cmp(&b.dur_us).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(items[0]);
            let label = if count == 1 {
                dominant.name.clone()
            } else {
                format!("{count} slices")
            };
            TimelineBarItem::OverviewBucket {
                start_us,
                end_us,
                count,
                label,
                cat: dominant.cat.clone(),
            }
        })
        .collect()
}

pub fn zoom_fracs(model: &TimelineModel, range: &TimeRange) -> (f64, f64) {
    let full_range = model.range_us();
    let lo = (range.start_us - model.min_ts_us) / full_range;
    let hi = (range.end_us - model.min_ts_us) / full_range;
    (lo, hi)
}

pub fn find_slice_in_model<'a>(model: &'a TimelineModel, key: SliceKey) -> Option<&'a TimelineSlice> {
    model
        .tracks
        .iter()
        .find_map(|track| find_slice_in_list(&track.slices, key))
}

fn find_slice_in_list<'a>(slices: &'a [TimelineSlice], key: SliceKey) -> Option<&'a TimelineSlice> {
    for slice in slices {
        if SliceKey::from_slice(slice) == key {
            return Some(slice);
        }
        if let Some(found) = find_slice_in_list(&slice.children, key) {
            return Some(found);
        }
    }
    None
}
