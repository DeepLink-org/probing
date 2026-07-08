//! HTTP client and typed endpoints.
//!
//! Naming: `trace` = Python live variable tracing (`/python` page);
//! `traces` = distributed span trees and Chrome/Ray timelines.

use crate::utils::error::{AppError, Result};

/// Base API client
pub struct ApiClient;

impl ApiClient {
    pub fn new() -> Self {
        Self
    }

    /// Get current page origin
    fn get_origin() -> Result<String> {
        web_sys::window()
            .ok_or_else(|| AppError::Api("No window object".to_string()))?
            .location()
            .origin()
            .map_err(|_| AppError::Api("Failed to get origin".to_string()))
    }

    /// Build API URL
    fn build_url(path: &str) -> Result<String> {
        Ok(format!(
            "{}{}",
            Self::get_origin()?,
            crate::utils::base_path::with_base(path)
        ))
    }

    /// Send GET request
    async fn get_request(&self, path: &str) -> Result<String> {
        let url = Self::build_url(path)?;
        let response = reqwest::get(&url).await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
                    return Err(AppError::Api(err.to_string()));
                }
            }
            return Err(AppError::Api(format!("HTTP error: {status}")));
        }

        Ok(body)
    }

    /// Send POST request (custom Content-Type)
    async fn post_request_with_body(&self, path: &str, body: String) -> Result<String> {
        let url = Self::build_url(path)?;
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .body(body)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(AppError::Api(format!("HTTP error: {}", response.status())));
        }

        Ok(response.text().await?)
    }

    /// Send GET request (public wrapper for agent / extensions).
    pub async fn get_raw(&self, path: &str) -> Result<String> {
        self.get_request(path).await
    }

    /// Parse JSON response
    pub fn parse_json<T: serde::de::DeserializeOwned>(response: &str) -> Result<T> {
        serde_json::from_str(response)
            .map_err(|e| AppError::Api(format!("JSON parse error: {}", e)))
    }
}

// Export all API modules
mod analytics;
mod cluster;
mod cpu;
mod dashboard;
mod files;
mod gpu;
mod profiling;
mod pulsing;
mod pytorch;
mod repl;
mod skills;
mod stack;
mod trace;
mod traces;
mod training;

#[allow(unused_imports)]
pub use analytics::*;
#[allow(unused_imports)]
pub use cluster::*;
#[allow(unused_imports)]
pub use cpu::*;
#[allow(unused_imports)]
pub use dashboard::*;
#[allow(unused_imports)]
pub use gpu::*;
#[allow(unused_imports)]
pub use profiling::*;
#[allow(unused_imports)]
pub use pulsing::*;
#[allow(unused_imports)]
pub use pytorch::*;
#[allow(unused_imports)]
pub use repl::*;
#[allow(unused_imports)]
pub use skills::*;
#[allow(unused_imports)]
pub use stack::*;
#[allow(unused_imports)]
pub use trace::*;
#[allow(unused_imports)]
pub use traces::*;
#[allow(unused_imports)]
pub use training::*;
