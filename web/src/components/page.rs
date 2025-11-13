use dioxus::prelude::*;
use crate::components::icon::Icon;
use icondata::Icon as IconData;

/// 页面标题组件 - 统一设计
#[component]
pub fn PageTitle(
    title: String, 
    subtitle: Option<String>,
    #[props(optional)] icon: Option<&'static IconData>,
) -> Element {
    rsx! {
        div {
            class: "mb-6",
            div {
                class: "flex items-center gap-3 mb-2",
                if let Some(icon_data) = icon {
                    Icon { icon: icon_data, class: "w-6 h-6 text-indigo-600" }
                }
                h1 {
                    class: "text-2xl font-bold text-gray-900",
                    "{title}"
                }
            }
            if let Some(subtitle) = subtitle {
                p {
                    class: "text-sm text-gray-600 ml-9",
                    "{subtitle}"
                }
            }
        }
    }
}

/// 页面头部组件（保留向后兼容）
#[component]
pub fn PageHeader(title: String, subtitle: Option<String>) -> Element {
    rsx! {
        PageTitle { title, subtitle, icon: None }
    }
}

/// 页面容器 - 统一间距系统
/// 使用 space-y-6 (24px) 作为 Card 之间的标准间距
#[component]
pub fn PageContainer(children: Element) -> Element {
    rsx! {
        div {
            class: "space-y-6",
            {children}
        }
    }
}