//! Centralized error types for the mc-launcher.

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("XML parse error: {0}")]
    Xml(String),
    #[error("URL parse error: {0}")]
    Url(#[from] url::ParseError),
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Java not found: {0}")]
    JavaNotFound(String),
    #[error("Version not found: {0}")]
    VersionNotFound(String),
    #[error("Loader error: {0}")]
    Loader(String),
    #[error("Mod error: {0}")]
    Mod(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("{0}")]
    Generic(String),
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Generic(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Generic(s.to_string())
    }
}

impl From<std::time::SystemTimeError> for AppError {
    fn from(e: std::time::SystemTimeError) -> Self {
        AppError::Generic(format!("System time error: {}", e))
    }
}

/// Convenience type alias for results using our error type.
pub type AppResult<T> = Result<T, AppError>;