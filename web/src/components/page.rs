//! Page-level structure: title block and content container.
//! Every content page should use PageContainer and usually PageTitle.

use dioxus::prelude::*;

use crate::components::icon::Icon;
use icondata::Icon as IconData;

/// Page heading with optional icon and subtitle. Place at top of page content.
#[component]
pub fn PageTitle(
    title: String,
    subtitle: Option<String>,
    #[props(optional)] icon: Option<&'static IconData>,
) -> Element {
    rsx! {
        div {
            class: "mb-4",
            div {
                class: "flex items-center gap-2",
                if let Some(icon_data) = icon {
                    Icon { icon: icon_data, class: "w-5 h-5 text-blue-600" }
                }
                h1 {
                    class: "text-xl font-semibold text-gray-900",
                    "{title}"
                }
            }
            if let Some(subtitle) = subtitle {
                p {
                    class: "text-sm text-gray-500 mt-0.5",
                    "{subtitle}"
                }
            }
        }
    }
}

/// Wrapper for page content with consistent vertical spacing. Use as the root of each page’s rsx.
#[component]
pub fn PageContainer(children: Element) -> Element {
    rsx! {
        div {
            class: "space-y-4",
            {children}
        }
    }
}
