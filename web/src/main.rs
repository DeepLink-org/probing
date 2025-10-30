use dioxus::prelude::*;

mod api;
mod app;
mod components;
mod hooks;
mod pages;
mod styles;
mod utils;

use app::App;

fn main() {
    launch(App);
}