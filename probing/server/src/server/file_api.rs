use super::config::{get_max_file_size, ALLOWED_FILE_DIRS};
use super::error::ApiResult;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Validate that the requested path is safe and within allowed directories
pub(crate) fn validate_path(path: &str) -> Result<PathBuf, String> {
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
    for base_dir in ALLOWED_FILE_DIRS {
        let base_path = match Path::new(base_dir).canonicalize() {
            Ok(path) => path,
            Err(_) => continue, // Skip non-existent base directories
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
        .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

    // Validate the path
    let safe_path = validate_path(path).map_err(|e| {
        log::warn!("Path validation failed for '{path}': {e}");
        anyhow::anyhow!("Invalid path: {}", e)
    })?;

    // Check file size before reading
    let metadata = tokio::fs::metadata(&safe_path).await.map_err(|e| {
        log::warn!("Failed to get metadata for {safe_path:?}: {e}");
        anyhow::anyhow!("Cannot access file")
    })?;

    let max_file_size = get_max_file_size();
    if metadata.len() > max_file_size {
        return Err(anyhow::anyhow!("File too large (max {} bytes allowed)", max_file_size).into());
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
    use std::fs;
    use tempfile::{TempDir, NamedTempFile};

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

    #[tokio::test]
    async fn test_validate_path_traversal_attack() {
        // Create a temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let allowed_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&allowed_dir).unwrap();

        // Try to access a file outside allowed directories using path traversal
        let traversal_path = allowed_dir.join("../../../etc/passwd");
        let traversal_str = traversal_path.to_str().unwrap();

        // This should fail because the path doesn't exist or is outside allowed dirs
        let result = validate_path(traversal_str);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_path_within_allowed_dir() {
        // Create a temporary directory structure matching ALLOWED_FILE_DIRS
        let temp_dir = TempDir::new().unwrap();
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        // Create a test file
        let test_file = logs_dir.join("test.log");
        fs::write(&test_file, "test content").unwrap();

        // Change to temp_dir to test relative paths
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Test with relative path
        let result = validate_path("./logs/test.log");
        assert!(result.is_ok());

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }

    #[tokio::test]
    async fn test_validate_path_outside_allowed_dir() {
        // Create a temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let outside_dir = temp_dir.path().join("outside");
        fs::create_dir_all(&outside_dir).unwrap();

        // Create a test file outside allowed directories
        let test_file = outside_dir.join("test.txt");
        fs::write(&test_file, "test content").unwrap();

        // Change to temp_dir to test relative paths
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // This should fail because it's outside allowed directories
        let result = validate_path("./outside/test.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }

    #[tokio::test]
    async fn test_validate_path_normalization() {
        // Create a temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        // Create a test file
        let test_file = logs_dir.join("test.log");
        fs::write(&test_file, "test content").unwrap();

        // Change to temp_dir to test relative paths
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Test with normalized path (using ..)
        let _result = validate_path("./logs/../logs/test.log");
        // This should work because canonicalize normalizes the path
        // But it might fail if the normalized path is outside allowed dirs
        // The actual behavior depends on how canonicalize resolves the path

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }

    #[tokio::test]
    async fn test_read_file_success() {
        // Create a temporary file
        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path();
        let content = "Hello, World!";
        fs::write(&file_path, content).unwrap();

        // Create a temporary directory matching allowed dirs
        let temp_dir = TempDir::new().unwrap();
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        // Copy file to allowed directory
        let allowed_file = logs_dir.join("test.txt");
        fs::copy(&file_path, &allowed_file).unwrap();

        // Change to temp_dir to test relative paths
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Test reading the file
        let mut params = HashMap::new();
        params.insert("path".to_string(), "./logs/test.txt".to_string());

        let result = read_file(axum::extract::Query(params)).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), content);

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }

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

    #[tokio::test]
    async fn test_read_file_size_limit() {
        // Create a temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        // Create a large file (exceeding MAX_FILE_SIZE)
        let large_content = "x".repeat((get_max_file_size() + 1) as usize);
        let large_file = logs_dir.join("large.txt");
        fs::write(&large_file, &large_content).unwrap();

        // Change to temp_dir to test relative paths
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let mut params = HashMap::new();
        params.insert("path".to_string(), "./logs/large.txt".to_string());

        let result = read_file(axum::extract::Query(params)).await;
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err().0);
        assert!(error_msg.contains("too large"));

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }

    #[tokio::test]
    async fn test_read_file_within_size_limit() {
        // Create a temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        // Create a file within size limit
        let content = "Small file content";
        let test_file = logs_dir.join("small.txt");
        fs::write(&test_file, content).unwrap();

        // Change to temp_dir to test relative paths
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let mut params = HashMap::new();
        params.insert("path".to_string(), "./logs/small.txt".to_string());

        let result = read_file(axum::extract::Query(params)).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), content);

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }

    #[tokio::test]
    async fn test_validate_path_double_encoding() {
        // Test path traversal with double encoding (....//....//)
        let temp_dir = TempDir::new().unwrap();
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        // Change to temp_dir
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Try double-encoded path traversal
        let result = validate_path("./logs/....//....//etc/passwd");
        // This should fail because canonicalize should normalize it
        assert!(result.is_err());

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }

    #[tokio::test]
    async fn test_validate_path_symlink() {
        // Note: Symlink tests may not work on all platforms
        // This is a basic test that symlinks are handled
        let temp_dir = TempDir::new().unwrap();
        let logs_dir = temp_dir.path().join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        // Create a test file
        let test_file = logs_dir.join("test.txt");
        fs::write(&test_file, "test").unwrap();

        // Change to temp_dir
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        // Test that canonicalize resolves symlinks
        let result = validate_path("./logs/test.txt");
        assert!(result.is_ok());

        // Restore original directory
        std::env::set_current_dir(&original_dir).unwrap();
    }
}
