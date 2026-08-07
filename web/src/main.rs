#![allow(clippy::suspicious_else_formatting, clippy::useless_format)]

use dioxus::prelude::*;

mod agent;
mod api;
mod components;
mod hooks;
mod next;
mod overhead;
mod state;
mod utils;

use next::NextApp;
use utils::base_path::base_path;

fn main() {
    let base = base_path();
    if base.is_empty() {
        launch(NextApp);
    } else {
        let prefix = Some(base);
        let config = dioxus_web::Config::new()
            .history(std::rc::Rc::new(dioxus_web::WebHistory::new(prefix, true)));
        dioxus_web::launch::launch_cfg(NextApp, config);
    }
}
