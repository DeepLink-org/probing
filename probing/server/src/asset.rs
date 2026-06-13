use std::env;

use axum::http::{header, StatusCode, Uri};
use axum::response::IntoResponse;
use bytes::Bytes;
use include_dir::include_dir;
use include_dir::Dir;
use once_cell::sync::Lazy;

static BASE_PATH: Lazy<String> = Lazy::new(|| {
    env::var("PROBING_BASE_PATH")
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string()
});

static ASSET: Dir = include_dir!("web/dist");

pub fn contains(path: &str) -> bool {
    if let Ok(assets_root) = env::var("PROBING_ASSETS_ROOT") {
        let path = format!("{}/{}", assets_root, path.trim_start_matches('/'));
        std::path::Path::new(path.as_str()).exists()
    } else {
        ASSET.contains(path.trim_start_matches('/'))
    }
}

pub fn get(path: &str) -> Bytes {
    if let Ok(assets_root) = env::var("PROBING_ASSETS_ROOT") {
        let path = format!("{}/{}", assets_root, path.trim_start_matches('/'));
        let content = std::fs::read(path).unwrap_or_default();
        Bytes::from(content)
    } else {
        ASSET
            .get_file(path.trim_start_matches('/'))
            .map(|f| Bytes::copy_from_slice(f.contents()))
            .unwrap_or_default()
    }
}

/// Get the content type of a file based on its extension
fn get_content_type(path: &str) -> &'static str {
    match path {
        p if p.ends_with(".html") => "text/html",
        p if p.ends_with(".js") => "application/javascript",
        p if p.ends_with(".css") => "text/css",
        p if p.ends_with(".svg") => "image/svg+xml",
        p if p.ends_with(".wasm") => "application/wasm",
        p if p.ends_with(".json") => "application/json",
        p if p.ends_with(".png") => "image/png",
        p if p.ends_with(".jpg") || p.ends_with(".jpeg") => "image/jpeg",
        p if p.ends_with(".gif") => "image/gif",
        p if p.ends_with(".ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Handler for index page
pub async fn index() -> impl IntoResponse {
    let mut html = String::from_utf8_lossy(&get("/index.html")).to_string();
    let base_path = BASE_PATH.clone();
    if !base_path.is_empty() {
        // Inject JS global for the frontend runtime
        let inject = format!(
            r#"<script>window.__PROBING_BASE_PATH__ = "{}";</script>"#,
            base_path
        );
        // Intercept fetch to rewrite WASM URLs with base path prefix
        let fetch_intercept = [
            "<script>(function(){var bp=\"",
            &base_path,
            "\";var o=window.fetch;window.fetch=function(i,n){if(typeof i==='string'&&i.startsWith('/'))i=bp+i;return o.call(this,i,n)}})();</script>",
        ].concat();
        let replacement = format!("<head>{}{}", inject, fetch_intercept);
        html = html.replacen("<head>", &replacement, 1);

        // Rewrite absolute paths in HTML to include base path prefix
        html = rewrite_html_paths(&html, &base_path);
    }
    ([(header::CONTENT_TYPE, "text/html")], html)
}

/// Rewrite absolute paths (src="/...", href="/...") in HTML to include base_path prefix.
/// Skips external URLs (//, http://, https://).
fn rewrite_html_paths(html: &str, base_path: &str) -> String {
    let mut result = html.to_string();
    for attr in &["src", "href"] {
        let mut offset = 0;
        let pattern = format!("{}=\"/", attr);
        while let Some(pos) = result[offset..].find(&pattern) {
            let global_pos = offset + pos;
            let value_start = global_pos + pattern.len(); // position right after the leading /
                                                          // Find the closing quote to get the full attribute value
            let value_end = result[value_start..]
                .find('"')
                .map(|i| value_start + i)
                .unwrap_or(result.len());
            let value = &result[value_start..value_end];
            // Skip protocol-relative URLs: //cdn.example.com/...
            if value.starts_with('/') {
                offset = global_pos + 1;
                continue;
            }
            // Skip external URLs: http:// or https://
            if value.starts_with("http://") || value.starts_with("https://") {
                offset = global_pos + 1;
                continue;
            }
            // Insert base_path right after the leading /
            // e.g. href="/./assets/foo.js" -> href="/proxy/task-123/./assets/foo.js"
            result.insert_str(
                value_start,
                &format!("{}/", base_path.trim_start_matches('/')),
            );
            offset = value_start + base_path.len() + 1;
        }
    }
    result
}

/// Handler for serving static files
pub async fn static_files(uri: Uri) -> Result<impl IntoResponse, StatusCode> {
    let path = uri.path();
    if !contains(path) {
        return Err(StatusCode::NOT_FOUND);
    }

    let content_type = get_content_type(path);
    let data = get(path);

    Ok(([(header::CONTENT_TYPE, content_type)], data))
}
