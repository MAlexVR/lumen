#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HarbizError {
    #[error("invalid email or password")]
    InvalidCredentials,
    #[error("session expired or not logged in")]
    Unauthorized,
    #[error("invalid Harbiz response: {0}")]
    InvalidResponse(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("file error: {0}")]
    Io(String),
}
