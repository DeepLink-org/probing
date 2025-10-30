use dioxus::prelude::*;
use crate::styles::combinations::*;

/// 页面头部组件
#[component]
pub fn PageHeader(title: String, subtitle: Option<String>) -> Element {
    rsx! {
        div {
            class: PAGE_HEADER,
            h1 {
                class: PAGE_TITLE,
                "{title}"
            }
            if let Some(subtitle) = subtitle {
                p {
                    class: PAGE_SUBTITLE,
                    "{subtitle}"
                }
            }
        }
    }
}

/// 简化的页面容器
#[component]
pub fn PageContainer(children: Element) -> Element {
    rsx! {
        div {
            class: PAGE_CONTAINER,
            {children}
        }
    }
}