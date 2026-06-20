// 认证授权复杂场景测试
// 这些测试需要base64编码、多个header设置等，因此放在独立的测试文件中

use axum::http::{HeaderMap, HeaderValue};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use probing_server::auth::{get_token_from_request, is_public_path};

// ========== Token提取复杂测试 ==========

#[test]
fn test_get_token_from_request_basic_auth() {
    let mut headers = HeaderMap::new();
    // Basic auth: "admin:password123" -> base64("admin:password123")
    let credentials = "admin:password123";
    let encoded = BASE64.encode(credentials);
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Basic {}", encoded)).unwrap(),
    );

    let token = get_token_from_request(&headers);
    assert_eq!(token, Some("password123".to_string()));
}

#[test]
fn test_get_token_from_request_basic_auth_wrong_username() {
    let mut headers = HeaderMap::new();
    // Basic auth with wrong username
    let credentials = "wronguser:password123";
    let encoded = BASE64.encode(credentials);
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Basic {}", encoded)).unwrap(),
    );

    let token = get_token_from_request(&headers);
    assert_eq!(token, None);
}

#[test]
fn test_get_token_from_request_priority_bearer_first() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Authorization",
        HeaderValue::from_static("Bearer bearer_token"),
    );
    headers.insert("X-Probing-Token", HeaderValue::from_static("custom_token"));

    // Bearer token should take priority
    let token = get_token_from_request(&headers);
    assert_eq!(token, Some("bearer_token".to_string()));
}

// ========== 公开路径判断复杂测试 ==========

#[test]
fn test_is_public_path_static() {
    assert!(is_public_path("/static/style.css"));
    assert!(is_public_path("/static/js/app.js"));
    assert!(is_public_path("/static/"));
}

#[test]
fn test_is_public_path_favicon() {
    assert!(is_public_path("/favicon.ico"));
    assert!(is_public_path("/favicon.png"));
    assert!(is_public_path("/favicon"));
}

#[test]
fn test_is_public_path_protected() {
    assert!(!is_public_path("/query"));
    assert!(!is_public_path("/apis/nodes"));
    assert!(!is_public_path("/config"));
    assert!(!is_public_path("/static"));
    assert!(!is_public_path("/staticfile"));
}
