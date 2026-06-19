//! Renders Markdown content with scoped `.markdown-body` styles.

use dioxus::prelude::*;

use crate::utils::markdown::markdown_to_html;

#[component]
pub fn MarkdownView(
    content: String,
    #[props(default = String::new())] class: String,
) -> Element {
    let html = markdown_to_html(&content);
    if html.is_empty() {
        return rsx! {};
    }

    rsx! {
        div {
            class: if class.is_empty() {
                "markdown-body".to_string()
            } else {
                format!("markdown-body {class}")
            },
            dangerous_inner_html: "{html}",
        }
    }
}
