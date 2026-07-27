//! Shared Training workspace controls.

use dioxus::prelude::*;

pub static TRAINING_CLUSTER_SCOPE: GlobalSignal<bool> = Signal::global(|| false);
pub static TRAINING_REFRESH: GlobalSignal<u32> = Signal::global(|| 0);
