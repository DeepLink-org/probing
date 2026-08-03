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
            } => format!("{message} · {action}"),
            Self::Http { message, .. } => message.clone(),
            _ => self.to_string(),
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
