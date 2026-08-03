use super::config::get_max_request_body_size;
use axum::{
    body::Body,
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::error::ApiError;

static IN_FLIGHT_REQUESTS: AtomicUsize = AtomicUsize::new(0);

struct RequestPermit;

impl RequestPermit {
    fn try_acquire() -> Option<Self> {
        loop {
            let current = IN_FLIGHT_REQUESTS.load(Ordering::Acquire);
            let limit = crate::runtime_state::config().max_connections();
            if current >= limit {
                return None;
            }
            if IN_FLIGHT_REQUESTS
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(Self);
            }
        }
    }
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        IN_FLIGHT_REQUESTS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Reject new HTTP requests when the in-flight request limit is reached.
pub async fn connection_limit_middleware(request: Request, next: Next) -> Response {
    let _permit = match RequestPermit::try_acquire() {
        Some(permit) => permit,
        None => {
            log::warn!(
                "in-flight request limit reached (max {})",
                crate::runtime_state::config().max_connections()
            );
            return ApiError::service_unavailable("server request concurrency limit reached")
                .with_action("retry after an in-flight request completes")
                .into_response();
        }
    };
    next.run(request).await
}

pub async fn request_timeout_middleware(request: Request, next: Next) -> Response {
    let timeout_secs = crate::runtime_state::config().request_timeout_secs();
    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        next.run(request),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => ApiError::gateway_timeout(format!("request exceeded {timeout_secs}s deadline"))
            .with_action("retry with a smaller query or narrower time window")
            .into_response(),
    }
}

/// Middleware to limit request body size
pub async fn request_size_limit_middleware(request: Request, next: Next) -> Response {
    let max_size = get_max_request_body_size();

    // Get the content-length header if present
    let content_length = request
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok());

    // If content-length is present and exceeds limit, reject immediately
    if let Some(length) = content_length {
        if length > max_size {
            log::warn!("Request rejected: Content-Length {length} exceeds limit {max_size}");
            return ApiError::payload_too_large(format!(
                "Request body too large (max {max_size} bytes allowed)"
            ))
            .into_response();
        }
    }

    // For requests without content-length or with acceptable content-length,
    // we need to check the actual body size
    let (parts, body) = request.into_parts();

    // Collect body with size limit
    let body_bytes = match collect_body_with_limit(body, max_size).await {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("Request body collection failed: {e}");
            return ApiError::payload_too_large(e).into_response();
        }
    };

    // Reconstruct the request with the limited body
    let new_body = Body::from(body_bytes);
    let new_request = Request::from_parts(parts, new_body);

    // Continue to the next middleware/handler
    next.run(new_request).await
}

/// Collect body bytes while enforcing the limit during streaming.
async fn collect_body_with_limit(body: Body, limit: usize) -> Result<Bytes, String> {
    let collected = Limited::new(body, limit)
        .collect()
        .await
        .map_err(|e| format!("failed to collect request body: {e}"))?;

    let bytes = collected.to_bytes();
    Ok(bytes)
}

/// Middleware for logging requests (optional - for debugging)
pub async fn request_logging_middleware(request: Request, next: Next) -> Response {
    let method = request.method().to_string();
    let uri = request.uri().to_string();
    let start = std::time::Instant::now();

    log::debug!("Incoming request: {method} {uri}");

    let response = next.run(request).await;
    let duration = start.elapsed();

    log::debug!(
        "Request completed: {} {} - {} in {:?}",
        method,
        uri,
        response.status(),
        duration
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use bytes::Bytes;

    #[tokio::test]
    async fn test_collect_body_with_limit_success() {
        let body = Body::from("Hello, World!");
        let result = collect_body_with_limit(body, 100).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Bytes::from("Hello, World!"));
    }

    #[tokio::test]
    async fn test_collect_body_with_limit_exceeded() {
        let large_data = "x".repeat(1000);
        let body = Body::from(large_data);
        let result = collect_body_with_limit(body, 100).await;
        assert!(result.is_err());
    }
}
