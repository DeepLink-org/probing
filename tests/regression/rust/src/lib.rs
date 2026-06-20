//! Shared helpers for Rust regression tests under `tests/regression/rust/probing/`.

pub mod test_helpers;

/// Path to `tests/regression/spec/api_spec.json`.
pub fn api_spec_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/api_spec.json")
}
