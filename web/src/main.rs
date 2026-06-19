use dioxus::prelude::*;

mod api;
mod app;
mod agent;
mod components;
mod hooks;
mod pages;
mod state;
mod utils;

use app::App;
use utils::base_path::base_path;

fn main() {
    let base = base_path();
    if base.is_empty() {
        launch(App);
    } else {
        let prefix = Some(base);
        let config = dioxus_web::Config::new()
            .history(std::rc::Rc::new(dioxus_web::WebHistory::new(prefix, true)));
        dioxus_web::launch::launch_cfg(App, config);
    }
}
