//! Global floating source code preview.

use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceViewerTarget {
    pub path: String,
    pub line: Option<i64>,
}

pub static SOURCE_VIEWER_OPEN: GlobalSignal<bool> = Signal::global(|| false);
pub static SOURCE_VIEWER_TARGET: GlobalSignal<Option<SourceViewerTarget>> = Signal::global(|| None);

pub fn open_source_viewer(path: String, line: Option<i64>) {
    *SOURCE_VIEWER_TARGET.write() = Some(SourceViewerTarget { path, line });
    *SOURCE_VIEWER_OPEN.write() = true;
    lock_body_scroll();
}

pub fn close_source_viewer() {
    *SOURCE_VIEWER_OPEN.write() = false;
    *SOURCE_VIEWER_TARGET.write() = None;
    unlock_body_scroll();
}

fn lock_body_scroll() {
    let Some(body) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.body())
    else {
        return;
    };
    let _ = body.set_attribute("data-probing-scroll-lock", "1");
    let _ = body.style().set_property("overflow", "hidden");
}

pub fn unlock_body_scroll() {
    let Some(body) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.body())
    else {
        return;
    };
    if body.get_attribute("data-probing-scroll-lock").is_some() {
        let _ = body.remove_attribute("data-probing-scroll-lock");
        let _ = body.style().set_property("overflow", "");
    }
}
