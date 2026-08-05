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

pub static MEMORY_CLUSTER_SCOPE: GlobalSignal<bool> = Signal::global(|| false);
pub static MEMORY_WINDOW_MINUTES: GlobalSignal<usize> = Signal::global(|| 5);
pub static MEMORY_REFRESH: GlobalSignal<u32> = Signal::global(|| 0);

const NEXT_SIDEBAR_COMPACT_KEY: &str = "probing_next_sidebar_compact";

pub static NEXT_SIDEBAR_COMPACT: GlobalSignal<bool> = Signal::global(|| false);

pub fn load_next_shell_settings() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    if let Ok(Some(value)) = storage.get_item(NEXT_SIDEBAR_COMPACT_KEY) {
        *NEXT_SIDEBAR_COMPACT.write() = value == "true";
    }
}

pub fn save_next_sidebar_compact(compact: bool) {
    *NEXT_SIDEBAR_COMPACT.write() = compact;
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(storage) = window.local_storage().ok().flatten() else {
        return;
    };
    let _ = storage.set_item(
        NEXT_SIDEBAR_COMPACT_KEY,
        if compact { "true" } else { "false" },
    );
}
