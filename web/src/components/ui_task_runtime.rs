//! Global UI task runtime — keeps elapsed timers ticking app-wide.

use dioxus::prelude::*;

use crate::state::ui_tasks::{any_ui_task_running, UI_TASK_TICK};

#[component]
pub fn UiTaskRuntime() -> Element {
    let _tick = UI_TASK_TICK.read();
    let running = any_ui_task_running();

    use_effect(move || {
        if !running {
            return;
        }
        spawn(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(500).await;
                if !any_ui_task_running() {
                    break;
                }
                *UI_TASK_TICK.write() += 1;
            }
        });
    });

    rsx! {}
}
