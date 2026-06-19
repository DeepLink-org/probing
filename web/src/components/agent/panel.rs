//! Floating Investigation Agent overlay (does not resize main content).

use dioxus::prelude::*;
use dioxus_router::use_route;

use crate::app::Route;
use crate::components::agent::chat::{AgentChat, AgentChatVariant};
use crate::state::agent::{AGENT_PANEL_OPEN, AGENT_PANEL_WIDTH};

#[component]
pub fn AgentPanel() -> Element {
    let route = use_route::<Route>();
    if !*AGENT_PANEL_OPEN.read() || matches!(route, Route::AgentPage {}) {
        return rsx! {};
    }

    let width_class = AGENT_PANEL_WIDTH.read().floating_class();

    rsx! {
        div { class: "absolute inset-0 z-40 flex pointer-events-none",
            div {
                class: "flex-1 bg-black/20 pointer-events-auto",
                onclick: move |_| *AGENT_PANEL_OPEN.write() = false,
            }
            div {
                class: "h-full {width_class} shrink-0 shadow-2xl pointer-events-auto",
                AgentChat { variant: AgentChatVariant::Floating }
            }
        }
    }
}
