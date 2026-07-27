//! Next-generation diagnostics-first web interface.

mod components;
mod model;
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
    rsx! {
        crate::components::app_overlays::AppOverlays {}
        Router::<NextRoute> {}
    }
}
