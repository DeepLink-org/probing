use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum AppError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("JSON parsing error: {0}")]
    Json(String),

    #[error("API error: {0}")]
    Api(String),
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
