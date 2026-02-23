use dioxus::prelude::*;

pub static SIDEBAR_WIDTH: GlobalSignal<f64> = Signal::global(|| 256.0);
pub static SIDEBAR_HIDDEN: GlobalSignal<bool> = Signal::global(|| false);

/// Load sidebar width and hidden state from localStorage into global signals.
pub fn load_sidebar_state() {
    if let Some(w) = web_sys::window() {
        if let Some(storage) = w.local_storage().ok().flatten() {
            if let Ok(Some(width_str)) = storage.get_item("sidebar_width") {
                if let Ok(width) = width_str.parse::<f64>() {
                    if (200.0..=600.0).contains(&width) {
                        *SIDEBAR_WIDTH.write() = width;
                    }
                }
            }
            if let Ok(Some(hidden_str)) = storage.get_item("sidebar_hidden") {
                if hidden_str == "true" {
                    *SIDEBAR_HIDDEN.write() = true;
                }
            }
        }
    }
}

/// Persist current sidebar width and hidden state to localStorage.
pub fn save_sidebar_state() {
    if let Some(w) = web_sys::window() {
        if let Some(storage) = w.local_storage().ok().flatten() {
            let _ = storage.set_item("sidebar_width", &SIDEBAR_WIDTH.read().to_string());
            let _ = storage.set_item("sidebar_hidden", &SIDEBAR_HIDDEN.read().to_string());
        }
    }
}
