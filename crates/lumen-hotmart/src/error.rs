use lumen_core::PlatformError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotmartError {
    /// The pasted token or cookie text is malformed (never includes its value).
    #[error("invalid token: {0}")]
    InvalidToken(String),
    /// 401/403: expired token, blocked request or no permission.
    #[error("session expired or invalid (HTTP {0})")]
    Unauthorized(u16),
    /// Hotmart answered 200 with a `message` instead of data.
    #[error("Hotmart said: {0}")]
    Api(String),
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("unexpected response shape: {0}")]
    Shape(String),
    #[error("HTTP {status} for {url}")]
    Http { status: u16, url: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("file error: {0}")]
    Io(String),
}

impl From<lumen_download::FetchError> for HotmartError {
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

impl From<HotmartError> for PlatformError {
    fn from(e: HotmartError) -> Self {
        match e {
            HotmartError::Unauthorized(_) => Self::SessionExpired,
            HotmartError::InvalidToken(m) => Self::InvalidInput(m),
            HotmartError::Json(m) | HotmartError::Shape(m) | HotmartError::Api(m) => Self::Parse(m),
            HotmartError::Network(_) | HotmartError::Http { .. } => Self::Network(e.to_string()),
            HotmartError::Io(m) => Self::Io(m),
        }
    }
}
