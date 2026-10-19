use lumen_core::PlatformError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkoolError {
    /// The HTML page carried no `__NEXT_DATA__`: expired session or WAF block.
    #[error("page has no __NEXT_DATA__ (session expired or blocked)")]
    SessionBlocked,
    #[error("session expired or invalid (HTTP {0})")]
    Unauthorized(u16),
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("unexpected page shape: {0}")]
    Shape(String),
    #[error("HTTP {status} for {url}")]
    Http { status: u16, url: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("invalid cookie: {0}")]
    Cookie(String),
    #[error("file error: {0}")]
    Io(String),
}

impl From<lumen_download::FetchError> for SkoolError {
    fn from(e: lumen_download::FetchError) -> Self {
        use lumen_download::FetchError as F;
        match e {
            F::Unauthorized(s) => Self::Unauthorized(s),
            F::Http { status, url } => Self::Http { status, url },
            F::Network(m) => Self::Network(m),
            F::Io(m) => Self::Io(m),
        }
    }
}

impl From<SkoolError> for PlatformError {
    fn from(e: SkoolError) -> Self {
        match e {
            SkoolError::SessionBlocked | SkoolError::Unauthorized(_) => {
                PlatformError::SessionExpired
            }
            SkoolError::Json(m) | SkoolError::Shape(m) => PlatformError::Parse(m),
            SkoolError::Network(_) | SkoolError::Http { .. } => {
                PlatformError::Network(e.to_string())
            }
            SkoolError::Cookie(m) => PlatformError::InvalidInput(m),
            SkoolError::Io(m) => PlatformError::Io(m),
        }
    }
}
