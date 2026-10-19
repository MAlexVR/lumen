use lumen_core::PlatformError;

/// Every failure the Udemy adapter can report. Messages never carry cookie or
/// token values and URLs are always redacted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UdemyError {
    /// The pasted cookie text is unusable (never includes its value).
    #[error("invalid cookies: {0}")]
    InvalidCookies(String),
    /// 401: the session is not valid (expired, logged out).
    #[error("session expired or invalid (HTTP 401)")]
    Unauthorized,
    /// 403 with a bot/captcha challenge page instead of JSON.
    #[error("Udemy asked for a bot or captcha check (HTTP 403)")]
    BotChallenge,
    /// 403 without a recognisable challenge: blocked or no permission.
    #[error("access denied (HTTP 403)")]
    Forbidden,
    #[error("not found (HTTP 404) for {url}")]
    NotFound { url: String },
    #[error("HTTP {status} for {url}")]
    Http { status: u16, url: String },
    #[error("network error: {0}")]
    Network(String),
    /// The body is not JSON.
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("unexpected response shape: {0}")]
    Shape(String),
    #[error("cancelled")]
    Cancelled,
}

impl UdemyError {
    /// HTTP status seen, when the failure came from a response.
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Unauthorized => Some(401),
            Self::BotChallenge | Self::Forbidden => Some(403),
            Self::NotFound { .. } => Some(404),
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Stable label used by the diagnostic: `http_401`, `parse_error`, `network`...
    pub fn status_label(&self) -> String {
        match self {
            Self::Network(_) => "network".to_string(),
            Self::Json(_) | Self::Shape(_) => "parse_error".to_string(),
            Self::InvalidCookies(_) => "invalid_cookies".to_string(),
            Self::Cancelled => "cancelled".to_string(),
            other => other
                .http_status()
                .map_or_else(|| "error".to_string(), |s| format!("http_{s}")),
        }
    }
}

impl From<UdemyError> for PlatformError {
    fn from(e: UdemyError) -> Self {
        match e {
            UdemyError::Unauthorized | UdemyError::Forbidden | UdemyError::BotChallenge => {
                Self::SessionExpired
            }
            UdemyError::InvalidCookies(m) => Self::InvalidInput(m),
            UdemyError::Json(m) | UdemyError::Shape(m) => Self::Parse(m),
            UdemyError::Network(_) | UdemyError::Http { .. } | UdemyError::NotFound { .. } => {
                Self::Network(e.to_string())
            }
            UdemyError::Cancelled => Self::Io("cancelled".into()),
        }
    }
}
