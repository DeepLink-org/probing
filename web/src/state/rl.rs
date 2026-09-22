//! Shared RL observability state (rollout filter persists across RL views).

use dioxus::prelude::*;

pub static ROLLOUT_FILTER: GlobalSignal<String> = Signal::global(String::new);
pub static ROLLOUT_FILTER_INPUT: GlobalSignal<String> = Signal::global(String::new);
pub static RL_EVENT_LIMIT: GlobalSignal<usize> = Signal::global(|| 400);
pub static SAMPLE_STEP_FILTER: GlobalSignal<Option<i64>> = Signal::global(|| None);
