//! Markdown → HTML for agent chat and skill summaries.

use pulldown_cmark::{html, Options, Parser};

/// Render a Markdown string to sanitized HTML (no raw HTML passthrough).
pub fn markdown_to_html(input: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let parser = Parser::new_ext(input, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}
