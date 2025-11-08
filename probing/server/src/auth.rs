use axum::{
    extract::Request,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use once_cell::sync::Lazy;
use probing_core::config;
use std::env;

// Auth token environment variable name
pub const AUTH_USERNAME_ENV: &str = "PROBING_AUTH_USERNAME"; // Optional, default is "admin"
pub const AUTH_REALM_ENV: &str = "PROBING_AUTH_REALM"; // Optional, default is "Probe Server"

// Static variable to hold the configured token
pub static AUTH_USERNAME: Lazy<String> =
    Lazy::new(|| env::var(AUTH_USERNAME_ENV).unwrap_or_else(|_| "admin".to_string()));

pub static AUTH_REALM: Lazy<String> =
    Lazy::new(|| env::var(AUTH_REALM_ENV).unwrap_or_else(|_| "Probe Server".to_string()));

/// Get the auth token from the request
pub(crate) fn get_token_from_request(headers: &HeaderMap) -> Option<String> {
    // Try Bearer token first
    let bearer_token = headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer ").map(|s| s.to_string()));

    if bearer_token.is_some() {
        return bearer_token;
    }

    // Try Basic Auth
    let basic_auth = headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Basic ").map(|s| s.to_string()))
        .and_then(|base64_value| BASE64.decode(base64_value).ok())
        .and_then(|decoded| String::from_utf8(decoded).ok())
        .and_then(|credentials| {
            // Basic auth format is "username:password"
            let parts: Vec<&str> = credentials.splitn(2, ':').collect();
            if parts.len() == 2 && parts[0] == AUTH_USERNAME.as_str() {
                Some(parts[1].to_string())
            } else {
                None
            }
        });

    if basic_auth.is_some() {
        return basic_auth;
    }

    // Finally try custom header
    headers
        .get("X-Probing-Token")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string())
}

/// Create a response that prompts the browser to show a login dialog
fn unauthorized_response() -> Response {
    let realm = format!("Basic realm=\"{}\"", AUTH_REALM.as_str());

    // Create WWW-Authenticate header value, fallback to default if invalid
    let www_auth = HeaderValue::from_str(&realm)
        .unwrap_or_else(|e| {
            log::error!("Failed to create WWW-Authenticate header value: {e}, using default");
            HeaderValue::from_static("Basic realm=\"Probe Server\"")
        });

    (
        StatusCode::UNAUTHORIZED,
        [
            (header::WWW_AUTHENTICATE, www_auth),
            (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
        ],
        "Unauthorized: Please login to access this resource",
    )
        .into_response()
}

/// Authentication middleware
pub async fn auth_middleware(request: Request, next: Next) -> Result<Response, impl IntoResponse> {
    // Get the configured token
    let configured_token = config::get_str("server.auth_token")
        .await
        .unwrap_or_default();
    log::debug!("Configured auth token: {configured_token:?}");

    if !configured_token.is_empty() {
        // Extract token from the request
        let provided_token = get_token_from_request(request.headers());

        // Check if token matches
        return match provided_token {
            Some(token) if token == configured_token => Ok(next.run(request).await),
            _ => Err(unauthorized_response()),
        };
    }

    Ok(next.run(request).await)
}

// Path prefixes that should bypass authentication
pub(crate) fn is_public_path(path: &str) -> bool {
    // Allow static assets without authentication
    path.starts_with("/static/")
        || path == "/"
        || path == "/index.html"
        || path.starts_with("/favicon")
}

/// Selective auth middleware that skips authentication for specific paths
pub async fn selective_auth_middleware(
    request: Request,
    next: Next,
) -> Result<Response, impl IntoResponse> {
    let path = request.uri().path();

    // Skip authentication for public paths
    if is_public_path(path) {
        return Ok(next.run(request).await);
    }

    // Apply authentication for all other paths
    auth_middleware(request, next).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use base64::engine::general_purpose::STANDARD as BASE64;


    #[test]
    fn test_get_token_from_request_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("Bearer test_token_123"),
        );

        let token = get_token_from_request(&headers);
        assert_eq!(token, Some("test_token_123".to_string()));
    }

    #[test]
    fn test_get_token_from_request_bearer_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("bearer test_token_123"),
        );

        let token = get_token_from_request(&headers);
        // Note: strip_prefix is case-sensitive, so this should not match
        assert_eq!(token, None);
    }

    #[test]
    fn test_get_token_from_request_bearer_empty() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Bearer "));

        let token = get_token_from_request(&headers);
        assert_eq!(token, Some("".to_string()));
    }

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
    fn test_get_token_from_request_custom_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Probing-Token",
            HeaderValue::from_static("custom_token_456"),
        );

        let token = get_token_from_request(&headers);
        assert_eq!(token, Some("custom_token_456".to_string()));
    }

    #[test]
    fn test_get_token_from_request_priority_bearer_first() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("Bearer bearer_token"),
        );
        headers.insert(
            "X-Probing-Token",
            HeaderValue::from_static("custom_token"),
        );

        // Bearer token should take priority
        let token = get_token_from_request(&headers);
        assert_eq!(token, Some("bearer_token".to_string()));
    }

    #[test]
    fn test_get_token_from_request_no_token() {
        let headers = HeaderMap::new();
        let token = get_token_from_request(&headers);
        assert_eq!(token, None);
    }

    #[test]
    fn test_is_public_path_static() {
        assert!(is_public_path("/static/style.css"));
        assert!(is_public_path("/static/js/app.js"));
        assert!(is_public_path("/static/"));
    }

    #[test]
    fn test_is_public_path_root() {
        assert!(is_public_path("/"));
    }

    #[test]
    fn test_is_public_path_index() {
        assert!(is_public_path("/index.html"));
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

    // Note: Testing middleware functions (auth_middleware, selective_auth_middleware)
    // requires creating a Next instance which is complex in axum 0.8.
    // These functions are tested through integration tests.
    // Here we test the core functions that can be tested directly:
    // - get_token_from_request (tested above)
    // - is_public_path (tested above)
}
