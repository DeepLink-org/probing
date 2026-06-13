/// Read the base path injected by the server into index.html.
///
/// When probing is deployed behind a reverse proxy with a sub-path prefix
/// (e.g. `/proxy/task-123`), the server sets `window.__PROBING_BASE_PATH__`
/// so the frontend can build correct URLs.
pub fn base_path() -> String {
    #[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
        export function get_base_path() {
            return window.__PROBING_BASE_PATH__ || "";
        }
    "#)]
    extern "C" {
        fn get_base_path() -> String;
    }
    get_base_path()
}

/// Prepend the base path to an absolute path (e.g. `/apis/nodes` → `/proxy/xxx/apis/nodes`).
pub fn with_base(path: &str) -> String {
    let base = base_path();
    format!("{}{}", base.trim_end_matches('/'), path)
}
