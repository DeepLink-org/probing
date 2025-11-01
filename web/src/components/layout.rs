use dioxus::prelude::*;

use crate::components::header::Header;

#[component]
pub fn AppLayout(children: Element) -> Element {
    rsx! {
        div {
            class: "min-h-screen bg-gray-50",
            Header {}
            main {
                class: "p-6",
                {children}
            }
        }
    }
}