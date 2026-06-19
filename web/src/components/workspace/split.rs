//! Horizontal split: main workspace + optional side panel with flex ratios.

use dioxus::prelude::*;

#[component]
pub fn WorkspaceSplit(
    main: Element,
    #[props(optional)] side: Option<Element>,
    main_flex: u8,
    side_flex: u8,
    #[props(default = true)] main_scroll: bool,
    #[props(default = false)] main_fullscreen: bool,
) -> Element {
    let main_overflow = if main_scroll {
        "overflow-y-auto"
    } else {
        "overflow-hidden"
    };
    let main_height = if main_fullscreen { "h-full" } else { "" };
    rsx! {
        div { class: "flex flex-1 min-h-0 overflow-hidden w-full",
            main {
                class: "min-w-0 {main_overflow} {main_height} p-4 sm:p-6 bg-gray-50",
                style: "flex: {main_flex} 1 0%; min-width: min(100%, 280px);",
                {main}
            }
            if let Some(side_panel) = side {
                div {
                    class: "min-w-0 flex flex-col",
                    style: "flex: {side_flex} 1 0%; min-width: min(100%, 320px);",
                    {side_panel}
                }
            }
        }
    }
}
