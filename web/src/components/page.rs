use dioxus::prelude::*;
// Tailwind classes inlined.

/// 页面头部组件
#[component]
pub fn PageHeader(title: String, subtitle: Option<String>) -> Element {
    rsx! {
        div {
            class: "mb-8",
            h1 {
                class: "text-3xl font-bold text-gray-900",
                "{title}"
            }
            if let Some(subtitle) = subtitle {
                p {
                    class: "mt-2 text-gray-600",
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
            class: "space-y-6",
            {children}
        }
    }
}