use dioxus::prelude::*;
use dioxus_router::Link;
use crate::app::Route;

#[component]
pub fn ThreadsCard(threads: Vec<u64>) -> Element {
    rsx! {
        div {
            class: "flex flex-wrap gap-2",
            div {
                class: "text-xs text-gray-500 mb-2",
                "Debug: ThreadsCard received {threads.len()} threads"
            }
            if threads.is_empty() {
                span {
                    class: "text-gray-500 italic",
                    "No threads found"
                }
            } else {
                for tid in threads {
                    Link {
                        to: Route::StackPage {},
                        button {
                            class: "px-3 py-1 text-sm bg-blue-100 text-blue-800 hover:bg-blue-200 rounded-md transition-colors",
                            "{tid}"
                        }
                    }
                }
            }
        }
    }
}
