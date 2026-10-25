#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GoldenlifeError {
    #[error("invalid email or password")]
    InvalidCredentials,
    #[error("session expired or not logged in")]
    Unauthorized,
    #[error("invalid GoldenLife response: {0}")]
    InvalidResponse(&'static str),
    #[error("invalid GoldenLife client configuration: {0}")]
    InvalidClientConfig(&'static str),
    #[error("invalid GoldenLife session: {0}")]
    InvalidSession(&'static str),
    #[error("HTTP error (status {0})")]
    HttpStatus(u16),
    #[error("network error: {0}")]
    Network(String),
    #[error("file error: {0}")]
    Io(String),
}
