//! Hint when profiling controls live in the (possibly hidden) sidebar.

use dioxus::prelude::*;

use crate::state::sidebar::SIDEBAR_HIDDEN;

#[component]
pub fn ProfilingSidebarHint() -> Element {
    if !*SIDEBAR_HIDDEN.read() {
        return rsx! {};
    }

    rsx! {
        div {
            class: "mb-3 px-3 py-2 text-xs rounded-md border border-amber-200 bg-amber-50 text-amber-900",
            "Profiling controls (sampling frequency, timeline reload), background tasks, and Torch overhead are in the left sidebar Monitor section — use the menu button (top-left) to show it."
        }
    }
}
