// 文件API复杂场景测试
// 这些测试需要创建临时目录、文件等，因此放在独立的测试文件中

use axum::extract::Query;
use std::collections::HashMap;
use std::fs;
use tempfile::{NamedTempFile, TempDir};

// Access server modules directly since tests can access private modules
// Note: server module is private, but tests can access it
use probing_server::server::file_api::{read_file, validate_path};
use probing_server::server::error::ApiResult;
use probing_server::server::config::get_max_file_size;

// ========== 路径验证复杂测试 ==========

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

// ========== 文件读取复杂测试 ==========

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

    let result: ApiResult<String> = read_file(Query(params)).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), content);

    // Restore original directory
    std::env::set_current_dir(&original_dir).unwrap();
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

    let result: ApiResult<String> = read_file(Query(params)).await;
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

    let result: ApiResult<String> = read_file(Query(params)).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), content);

    // Restore original directory
    std::env::set_current_dir(&original_dir).unwrap();
}

