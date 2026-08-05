//! Keeps PAGE_CONTEXT in sync with the active route and refreshes snapshots.

use dioxus::prelude::*;
use dioxus_router::use_route;

use crate::agent::page_tools::{
    describe_route, refresh_page_snapshot_for_route, refresh_page_snapshot_quiet,
};
use crate::app::Route;
use crate::state::investigation::investigation_context_key;
use crate::state::investigation::INVESTIGATION_CONTEXT;
use crate::state::page_context::{
    apply_page_descriptor, PageContextDescriptor, CURRENT_ROUTE, PAGE_CONTEXT,
};

#[component]
pub fn PageContextSync() -> Element {
    let route = use_route::<Route>();
    let investigation = INVESTIGATION_CONTEXT.read().clone();

    use_effect(move || {
        let route = route.clone();
        let investigation = investigation.clone();
        *CURRENT_ROUTE.write() = Some(route.clone());
        let desc = describe_route(&route);
        let old_page_id = PAGE_CONTEXT.read().page_id.clone();
        apply_page_descriptor(PageContextDescriptor {
            page_id: desc.page_id,
            title: desc.title,
            path: desc.path,
            description: desc.description,
            suggested_skills: desc.suggested_skills,
            investigation_summary: investigation.summary(),
            investigation_coordinates: investigation.coordinates_summary(),
            investigation_key: investigation_context_key(&investigation),
        });
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
