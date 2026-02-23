use dioxus::prelude::*;

use crate::components::colors::colors;
use crate::state::sidebar::{save_sidebar_state, SIDEBAR_WIDTH};

#[component]
pub fn ResizeHandle() -> Element {
    let mut is_resizing = use_signal(|| false);
    let mut drag_start_x = use_signal(|| 0.0);
    let mut drag_start_width = use_signal(|| 256.0);

    let hover_class = format!("hover:bg-{}/50", colors::PRIMARY);
    let active_class = if *is_resizing.read() {
        format!("bg-{}", colors::PRIMARY)
    } else {
        "bg-transparent".to_string()
    };
    let drag_handle_class = format!(
        "absolute top-0 right-0 w-1 h-full cursor-col-resize {} transition-colors group z-20 {}",
        hover_class, active_class
    );

    rsx! {
        div {
            class: "{drag_handle_class}",
            onmousedown: move |ev| {
                *is_resizing.write() = true;
                *drag_start_x.write() = ev.element_coordinates().x as f64;
                *drag_start_width.write() = *SIDEBAR_WIDTH.read();
                ev.prevent_default();
            },
            onmousemove: move |ev| {
                if *is_resizing.read() {
                    let current_x = ev.element_coordinates().x as f64;
                    let delta_x = current_x - *drag_start_x.read();
                    let new_width =
                        (*drag_start_width.read() + delta_x).max(200.0).min(600.0);
                    *SIDEBAR_WIDTH.write() = new_width;
                }
            },
            onmouseup: move |_| {
                if *is_resizing.read() {
                    *is_resizing.write() = false;
                    save_sidebar_state();
                }
            },
            onmouseleave: move |_| {
                if *is_resizing.read() {
                    *is_resizing.write() = false;
                }
            },
            div {
                class: "absolute top-1/2 right-0 transform translate-x-1/2 -translate-y-1/2 w-1 h-8 bg-gray-300 rounded-full opacity-0 group-hover:opacity-100 transition-opacity",
            }
        }
    }
}
