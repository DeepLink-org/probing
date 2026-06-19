use super::config::{allowed_file_base_dirs, get_max_file_size};
use crate::server::error::{ApiError, ApiResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Validate that the requested path is safe and within allowed directories
/// Made public for integration tests
pub fn validate_path(path: &str) -> Result<PathBuf, String> {
    // Reject empty paths
    if path.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    // Reject paths with null bytes (security risk)
    if path.contains('\0') {
        return Err("Path contains invalid characters".to_string());
    }

    // Convert to canonical path to resolve any .. or . components
    let requested_path = Path::new(path);
    let canonical_path = match requested_path.canonicalize() {
        Ok(path) => path,
        Err(_) => return Err("Invalid or non-existent path".to_string()),
    };

    // Check if the canonical path is within any allowed base directory
    let mut is_allowed = false;
    for base_dir in allowed_file_base_dirs() {
        let base_path = match base_dir.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };

        if canonical_path.starts_with(&base_path) {
            is_allowed = true;
            break;
        }
    }

    if !is_allowed {
        return Err("Access denied: path is outside allowed directories".to_string());
    }

    Ok(canonical_path)
}

/// Read a file from the filesystem with security checks
pub async fn read_file(
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> ApiResult<String> {
    let path = params
        .get("path")
        .ok_or_else(|| ApiError::bad_request("Missing 'path' parameter"))?;

    // Validate the path
    let safe_path = validate_path(path).map_err(|e| {
        log::warn!("Path validation failed for '{path}': {e}");
        ApiError::bad_request(format!("Invalid path: {e}"))
    })?;

    // Check file size before reading
    let metadata = tokio::fs::metadata(&safe_path).await.map_err(|e| {
        log::warn!("Failed to get metadata for {safe_path:?}: {e}");
        anyhow::anyhow!("Cannot access file")
    })?;

    let max_file_size = get_max_file_size();
    if metadata.len() > max_file_size {
        return Err(ApiError::payload_too_large(format!(
            "File too large (max {max_file_size} bytes allowed)"
        )));
    }

    // Read file content asynchronously
    let content = tokio::fs::read_to_string(&safe_path).await.map_err(|e| {
        log::warn!("Failed to read file {safe_path:?}: {e}");
        anyhow::anyhow!("Cannot read file")
    })?;

    log::info!("Successfully read file: {safe_path:?}");
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_validate_path_empty() {
        let result = validate_path("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[tokio::test]
    async fn test_validate_path_null_byte() {
        let result = validate_path("test\0file.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid characters"));
    }

    #[tokio::test]
    async fn test_validate_path_nonexistent() {
        let result = validate_path("/nonexistent/path/file.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid or non-existent"));
    }

    // Note: Lengthy tests (requiring temporary directories, files, etc.) have been moved to tests/file_api_complex_tests.rs

    #[tokio::test]
    async fn test_read_file_missing_path_param() {
        let params = HashMap::new();
        let result = read_file(axum::extract::Query(params)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_file_nonexistent() {
        let mut params = HashMap::new();
        params.insert("path".to_string(), "/nonexistent/file.txt".to_string());

        let result = read_file(axum::extract::Query(params)).await;
        assert!(result.is_err());
    }
}
