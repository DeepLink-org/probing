//! Classic/next UI selection.
//!
//! Only one application root is mounted at a time. Switching versions persists
//! the preference and reloads the page so hooks, global signals, and listeners
//! from the previous UI cannot leak into the next one.

use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
const UI_VERSION_KEY: &str = "probing.ui.version";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiVersion {
    Classic,
    #[default]
    Next,
}

impl UiVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Next => "next",
        }
    }

    const fn other(self) -> Self {
        match self {
            Self::Classic => Self::Next,
            Self::Next => Self::Classic,
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "classic" | "legacy" | "v1" => Some(Self::Classic),
            "next" | "v2" => Some(Self::Next),
            _ => None,
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn version_from_search(search: &str) -> Option<UiVersion> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| (key == "ui").then(|| UiVersion::parse(value)).flatten())
}

#[cfg(any(target_arch = "wasm32", test))]
fn search_with_version(search: &str, version: UiVersion) -> String {
    let mut pairs = search
        .trim_start_matches('?')
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| pair.split_once('=').is_none_or(|(key, _)| key != "ui"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    pairs.push(format!("ui={}", version.as_str()));
    format!("?{}", pairs.join("&"))
}

#[cfg(target_arch = "wasm32")]
fn persist(version: UiVersion) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item(UI_VERSION_KEY, version.as_str());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn persist(_version: UiVersion) {}

fn initial_version() -> UiVersion {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return UiVersion::default();
        };
        if let Ok(search) = window.location().search() {
            if let Some(version) = version_from_search(&search) {
                persist(version);
                return version;
            }
        }
        if let Ok(Some(storage)) = window.local_storage() {
            if let Ok(Some(value)) = storage.get_item(UI_VERSION_KEY) {
                if let Some(version) = UiVersion::parse(&value) {
                    return version;
                }
            }
        }
    }
    UiVersion::default()
}

pub fn activate(version: UiVersion) {
    persist(version);
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return;
        };
        let location = window.location();
        let search = location.search().unwrap_or_default();
        if location
            .set_search(&search_with_version(&search, version))
            .is_err()
        {
            let _ = location.reload();
        }
    }
}

pub fn href_for(path: &str, version: UiVersion) -> String {
    let path = crate::utils::base_path::with_base(path);
    let separator = if path.contains('?') { '&' } else { '?' };
    format!("{path}{separator}ui={}", version.as_str())
}

#[component]
pub fn RootApp() -> Element {
    let version = use_hook(initial_version);
    rsx! {
        match version {
            UiVersion::Classic => rsx! { crate::app::App {} },
            UiVersion::Next => rsx! { crate::next::NextApp {} },
        }
        UiVersionSwitch { current: version }
    }
}

#[component]
fn UiVersionSwitch(current: UiVersion) -> Element {
    if current == UiVersion::Next {
        return rsx! {};
    }
    let target = current.other();
    rsx! {
        button {
            r#type: "button",
            class: "fixed bottom-4 right-4 z-[10050] rounded-full border border-blue-300 \
                    bg-white/95 px-3 py-1.5 text-xs font-medium text-blue-700 shadow-lg \
                    backdrop-blur hover:bg-blue-50 focus:outline-none focus:ring-2 \
                    focus:ring-blue-500 focus:ring-offset-2",
            title: "Switch Probing web interface",
            onclick: move |_| activate(target),
            "Try next UI"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_is_the_default_interface() {
        assert_eq!(UiVersion::default(), UiVersion::Next);
        assert_eq!(initial_version(), UiVersion::Next);
    }

    #[test]
    fn query_version_has_aliases() {
        assert_eq!(version_from_search("?ui=next"), Some(UiVersion::Next));
        assert_eq!(
            version_from_search("?trace=abc&ui=v1"),
            Some(UiVersion::Classic)
        );
        assert_eq!(version_from_search("?ui=unknown"), None);
    }

    #[test]
    fn replacing_version_preserves_other_query_parameters() {
        assert_eq!(
            search_with_version("?trace_id=abc&ui=classic&rank=7", UiVersion::Next),
            "?trace_id=abc&rank=7&ui=next"
        );
        assert_eq!(search_with_version("", UiVersion::Classic), "?ui=classic");
    }
}
