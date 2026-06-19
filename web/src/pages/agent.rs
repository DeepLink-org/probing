//! Investigation Agent — full-page workspace view.

use dioxus::prelude::*;

use crate::components::agent::chat::{AgentChat, AgentChatVariant};
use crate::components::page::PageContainer;
use crate::state::agent::AGENT_PANEL_OPEN;

#[component]
pub fn Agent() -> Element {
    use_effect(move || {
        // Dedicated page replaces the floating overlay.
        *AGENT_PANEL_OPEN.write() = false;
    });

    rsx! {
        PageContainer {
            div { class: "flex flex-col min-h-[calc(100vh-11rem)]",
                AgentChat { variant: AgentChatVariant::Page }
            }
        }
    }
}
