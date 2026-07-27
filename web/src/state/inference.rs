//! Shared inference-page refresh controls.

use dioxus::prelude::*;

pub static INFERENCE_REFRESH: GlobalSignal<u64> = Signal::global(|| 0);
