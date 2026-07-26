//! Shared controls owned by the Next diagnostics shell.
//!
//! These signals keep route-level controls in the sidebar while the active page
//! remains responsible for rendering evidence.

use dioxus::prelude::*;

pub static DASHBOARD_AUTO_REFRESH: GlobalSignal<bool> = Signal::global(|| true);
pub static DASHBOARD_MANUAL_REFRESH: GlobalSignal<u32> = Signal::global(|| 0);

pub static DISTRIBUTED_CLUSTER_SCOPE: GlobalSignal<bool> = Signal::global(|| true);
pub static DISTRIBUTED_STEP_LIMIT: GlobalSignal<usize> = Signal::global(|| 256);
pub static DISTRIBUTED_REFRESH: GlobalSignal<u32> = Signal::global(|| 0);
