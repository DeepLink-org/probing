use dioxus::prelude::*;
use dioxus_router::use_navigator;

use crate::state::agent::AGENT_PANEL_OPEN;

use super::super::components::InlineNotice;
use super::super::routes::NextRoute;

#[component]
pub fn TrainingPage() -> Element {
    let navigator = use_navigator();
    let agent_requested = *AGENT_PANEL_OPEN.read();
    use_effect(move || {
        if agent_requested {
            *AGENT_PANEL_OPEN.write() = false;
            navigator.push(NextRoute::Investigate {});
        }
    });

    rsx! {
        div { class: "space-y-4",
            InlineNotice {
                title: "Progressive migration".to_string(),
                detail: "The proven step matrix, module hotspot, collective, and inspector components are reused inside the new diagnostics shell.".to_string(),
            }
            crate::pages::training::Training { show_controls: false }
        }
    }
}
