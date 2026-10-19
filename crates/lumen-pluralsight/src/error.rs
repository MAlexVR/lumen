#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluralsightError {
    #[error("invalid Pluralsight cookies: {0}")]
    InvalidCredentials(String),
    #[error("invalid Pluralsight response: {0}")]
    InvalidResponse(&'static str),
    #[error("Pluralsight clip is explicitly marked as protected or encrypted")]
    ProtectedContent,
    #[error("Pluralsight response has no valid HLS stream")]
    NoStream,
    #[error("invalid Pluralsight client configuration: {0}")]
    InvalidClientConfig(&'static str),
    #[error("session expired or invalid (HTTP {0})")]
    Unauthorized(u16),
    #[error("HTTP error (status {0})")]
    HttpStatus(u16),
    #[error("network error: {0}")]
    Network(String),
    #[error("invalid Pluralsight course identifier")]
    InvalidCourseId,
    #[error("Pluralsight filesystem operation failed")]
    Io,
    #[error("Pluralsight download report could not be written")]
    ReportWrite,
}
