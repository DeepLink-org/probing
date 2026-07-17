//! Shared RL observability state (rollout filter persists across RL views).

use dioxus::prelude::*;

pub static ROLLOUT_FILTER: GlobalSignal<String> = Signal::global(|| String::new());
pub static ROLLOUT_FILTER_INPUT: GlobalSignal<String> = Signal::global(|| String::new());

/// Height of the pinned rollout/train detail panel (px). Synced for main content padding.
pub static RL_DETAIL_PANEL_HEIGHT: GlobalSignal<u32> = Signal::global(|| 256);

pub const RL_DETAIL_PANEL_HEIGHT_DEFAULT: u32 = 256;
pub const RL_DETAIL_PANEL_HEIGHT_MIN: u32 = 120;
pub const RL_DETAIL_PANEL_HEIGHT_MAX: u32 = 1200;

pub fn estimate_detail_panel_height(segment_count: usize) -> u32 {
    let estimated = 108u32.saturating_add(segment_count as u32 * 44);
    estimated
        .max(RL_DETAIL_PANEL_HEIGHT_MIN)
        .min(RL_DETAIL_PANEL_HEIGHT_MAX)
}
