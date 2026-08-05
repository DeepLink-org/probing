//! Next-generation diagnostics-first web interface.

pub(crate) mod capabilities;
mod components;
pub(crate) mod evidence;
mod model;
mod page_registry;
mod page_snapshot;
mod pages;
mod routes;
mod settings;
mod shell;
mod sidebar;

use dioxus::prelude::*;
use dioxus_router::Router;

pub use routes::NextRoute;

#[component]
pub fn NextApp() -> Element {
    // Seed URL-backed investigation coordinates before the router mounts the
    // shell and its URL synchronization component.
    use_hook(crate::state::investigation::load_investigation_context);
    rsx! {
        crate::components::app_overlays::AppOverlays {}
        Router::<NextRoute> {}
    }
}
