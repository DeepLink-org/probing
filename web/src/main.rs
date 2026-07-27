#![allow(clippy::suspicious_else_formatting, clippy::useless_format)]

use dioxus::prelude::*;

mod agent;
mod api;
mod app;
mod components;
mod hooks;
mod next;
mod overhead;
mod pages;
mod rl_contract;
mod state;
mod ui_version;
mod utils;

use ui_version::RootApp;
use utils::base_path::base_path;

fn main() {
    let base = base_path();
    if base.is_empty() {
        launch(RootApp);
    } else {
        let prefix = Some(base);
        let config = dioxus_web::Config::new()
            .history(std::rc::Rc::new(dioxus_web::WebHistory::new(prefix, true)));
        dioxus_web::launch::launch_cfg(RootApp, config);
    }
}
