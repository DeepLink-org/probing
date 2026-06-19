//! Keeps PAGE_CONTEXT in sync with the active route and refreshes snapshots.

use dioxus::prelude::*;
use dioxus_router::use_route;

use crate::agent::page_tools::{
    describe_route, refresh_page_snapshot_for_route, refresh_page_snapshot_quiet,
};
use crate::app::Route;
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::page_context::{apply_page_descriptor, CURRENT_ROUTE, PAGE_CONTEXT};

#[component]
pub fn PageContextSync() -> Element {
    let route = use_route::<Route>();
    let inv_summary = INVESTIGATION_CONTEXT.read().summary();

    use_effect(move || {
        let route = route.clone();
        let inv_summary = inv_summary.clone();
        *CURRENT_ROUTE.write() = Some(route.clone());
        let desc = describe_route(&route);
        let old_page_id = PAGE_CONTEXT.read().page_id.clone();
        apply_page_descriptor(
            desc.page_id,
            desc.title,
            desc.path,
            desc.description,
            desc.suggested_playbooks,
            inv_summary,
        );
        let route_changed = old_page_id != PAGE_CONTEXT.read().page_id;
        if route_changed {
            spawn(async move {
                refresh_page_snapshot_for_route(route).await;
            });
        } else {
            spawn(async move {
                refresh_page_snapshot_quiet(route).await;
            });
        }
    });

    rsx! {}
}
