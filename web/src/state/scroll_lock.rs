//! Shared document scroll lock for viewport overlays.

pub fn lock_body_scroll() {
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
