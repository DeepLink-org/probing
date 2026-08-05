use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum AppError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("JSON parsing error: {0}")]
    Json(String),

    #[error("API error: {0}")]
    Api(String),

    #[error("{message}")]
    Http {
        status: u16,
        code: String,
        message: String,
        retryable: bool,
        action: Option<String>,
    },
    #[error("Cancelled")]
    Cancelled,
}

impl AppError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, AppError::Cancelled)
    }

    /// User-facing message for display in the UI (enables future i18n).
    pub fn display_message(&self) -> String {
        match self {
            Self::Http {
                message,
                action: Some(action),
                ..
            } => format!("{} · {action}", safe_error_message(message)),
            Self::Http { message, .. } => safe_error_message(message),
            Self::Network(message) if message.to_ascii_lowercase().contains("timed out") => {
                "The Probing server did not respond in time.".to_string()
            }
            Self::Network(_) => "The Probing server could not be reached.".to_string(),
            Self::Json(_) => "The server returned an unreadable response.".to_string(),
            Self::Api(message) => safe_error_message(message),
            Self::Cancelled => "The request was cancelled.".to_string(),
        }
    }

    pub fn http(
        status: u16,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        action: Option<String>,
    ) -> Self {
        Self::Http {
            status,
            code: code.into(),
            message: message.into(),
            retryable,
            action,
        }
    }
}

fn safe_error_message(message: &str) -> String {
    let trimmed = message.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("schema error") || lower.contains("no field named") {
        return "This evidence is not available in the current runtime.".to_string();
    }
    if lower.contains("no handler found") {
        return "This capability is not available in the current runtime.".to_string();
    }
    if lower.contains("json parse error") || lower.contains("failed to decode") {
        return "The server returned an unreadable response.".to_string();
    }
    if trimmed.is_empty() {
        return "The request failed before usable evidence was returned.".to_string();
    }
    if trimmed.chars().count() > 240 {
        return "The request failed before usable evidence was returned.".to_string();
    }
    trimmed.to_string()
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Json(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_details_are_not_exposed_to_product_pages() {
        let error = AppError::Api(
            "Schema error: No field named metric_name. Valid fields are internal._error"
                .to_string(),
        );
        assert_eq!(
            error.display_message(),
            "This evidence is not available in the current runtime."
        );
    }

    #[test]
    fn concise_api_errors_remain_actionable() {
        let error = AppError::Api("No completed step samples were returned".to_string());
        assert_eq!(
            error.display_message(),
            "No completed step samples were returned"
        );
    }
}
