use dioxus::prelude::*;
use icondata::Icon as IconData;

#[component]
pub fn Icon(icon: &'static IconData, #[props(default = "w-5 h-5")] class: &'static str) -> Element {
    let view_box = icon.view_box.unwrap_or("0 0 24 24");

    rsx! {
        svg {
            class: "{class}",
            view_box: "{view_box}",
            fill: "currentColor",
            dangerous_inner_html: "{icon.data}"
        }
    }
}
