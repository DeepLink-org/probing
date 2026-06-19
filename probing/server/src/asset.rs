use std::env;

use axum::body::Body;
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
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

/// Normalize request paths such as `/./assets/foo.js` → `assets/foo.js`.
fn normalize_asset_path(path: &str) -> String {
    let mut p = path.trim_start_matches('/').to_string();
    while p.starts_with("./") {
        p = p[2..].to_string();
    }
    p
}

fn read_embedded(key: &str) -> Option<Bytes> {
    ASSET
        .get_file(key)
        .map(|f| Bytes::copy_from_slice(f.contents()))
}

fn read_from_disk(key: &str) -> Option<Bytes> {
    let assets_root = env::var("PROBING_ASSETS_ROOT").ok()?;
    let path = format!("{}/{}", assets_root, key);
    let content = std::fs::read(path).ok()?;
    Some(Bytes::from(content))
}

fn read_asset(key: &str) -> Option<Bytes> {
    read_from_disk(key).or_else(|| read_embedded(key))
}

pub fn contains(path: &str) -> bool {
    let key = normalize_asset_path(path);
    if env::var("PROBING_ASSETS_ROOT").is_ok() {
        let path = format!("{}/{}", env::var("PROBING_ASSETS_ROOT").unwrap(), key);
        std::path::Path::new(path.as_str()).exists()
    } else {
        ASSET.contains(&key)
    }
}

pub fn get(path: &str) -> Bytes {
    let key = normalize_asset_path(path);
    read_asset(&key).unwrap_or_default()
}

fn accepts_brotli(accept_encoding: &str) -> bool {
    accept_encoding.split(',').any(|part| {
        part.split(';')
            .next()
            .unwrap_or(part)
            .trim()
            .eq_ignore_ascii_case("br")
    })
}

/// Dioxus content-hashed bundles embed `-dxh` in the filename.
fn is_content_hashed(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.contains("-dxh"))
}

fn cache_control(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.ends_with(".html") {
        "no-cache"
    } else if is_content_hashed(path) {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    }
}

/// Resolve asset bytes, preferring pre-compressed Brotli companions when available.
fn resolve_asset(path: &str, accept_encoding: &str) -> (Bytes, Option<&'static str>) {
    let key = normalize_asset_path(path);

    if accepts_brotli(accept_encoding) {
        let br_key = format!("{key}.br");
        if let Some(data) = read_asset(&br_key) {
            if !data.is_empty() {
                return (data, Some("br"));
            }
        }
    }

    (get(path), None)
}

/// Strip a trailing `.br` before inferring MIME type.
fn logical_path(path: &str) -> &str {
    path.strip_suffix(".br").unwrap_or(path)
}

/// Get the content type of a file based on its extension
fn get_content_type(path: &str) -> &'static str {
    match logical_path(path) {
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
    (
        [
            (header::CONTENT_TYPE, "text/html"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        html,
    )
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
pub async fn static_files(uri: Uri, headers: HeaderMap) -> Result<Response, StatusCode> {
    let path = uri.path();
    if !contains(path) {
        return Err(StatusCode::NOT_FOUND);
    }

    let accept_encoding = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let key = normalize_asset_path(path);
    let (data, encoding) = resolve_asset(path, accept_encoding);
    let content_type = get_content_type(&key);
    let cache = cache_control(&key);

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache);
    if let Some(enc) = encoding {
        builder = builder.header(header::CONTENT_ENCODING, enc);
    }

    Ok(builder
        .body(Body::from(data))
        .unwrap_or_else(|_| Response::new(Body::empty())))
}

#[cfg(test)]
mod tests {
    use super::{
        accepts_brotli, cache_control, get_content_type, is_content_hashed, normalize_asset_path,
    };

    #[test]
    fn normalize_dioxus_asset_paths() {
        assert_eq!(normalize_asset_path("/./assets/web.js"), "assets/web.js");
        assert_eq!(
            normalize_asset_path("/assets/web_bg.wasm"),
            "assets/web_bg.wasm"
        );
    }

    #[test]
    fn accepts_brotli_encoding() {
        assert!(accepts_brotli("br"));
        assert!(accepts_brotli("gzip, br"));
        assert!(accepts_brotli("gzip, br;q=0.9"));
        assert!(!accepts_brotli("gzip"));
        assert!(!accepts_brotli(""));
    }

    #[test]
    fn content_type_ignores_brotli_suffix() {
        assert_eq!(
            get_content_type("assets/web_bg-dxhabc.wasm.br"),
            "application/wasm"
        );
        assert_eq!(
            get_content_type("assets/web-dxhabc.js.br"),
            "application/javascript"
        );
    }

    #[test]
    fn cache_control_for_hashed_assets() {
        assert_eq!(
            cache_control("assets/web_bg-dxhabc123.wasm"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control("index.html"), "no-cache");
        assert_eq!(cache_control("assets/tailwind.css"), "public, max-age=3600");
    }

    #[test]
    fn detects_content_hashed_names() {
        assert!(is_content_hashed("assets/web-dxh9874fc485ebe9e2.js"));
        assert!(!is_content_hashed("assets/tailwind.css"));
    }
}
