use lumen_download::VideoError;

use crate::parse::ParseError;
use crate::url::UrlError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum YoutubeError {
    #[error("that is not a valid link")]
    InvalidUrl,
    #[error("only YouTube video and playlist links are supported")]
    UnsupportedUrl,
    /// The video exists but cannot be fetched (private, removed, region...).
    #[error("{0}")]
    Unavailable(String),
    #[error("the playlist has no videos")]
    EmptyPlaylist,
    #[error("{0}")]
    Tool(VideoError),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error("file error: {0}")]
    Io(String),
}

impl From<UrlError> for YoutubeError {
    fn from(e: UrlError) -> Self {
        match e {
            UrlError::Invalid => Self::InvalidUrl,
            UrlError::Unsupported => Self::UnsupportedUrl,
        }
    }
}

impl From<VideoError> for YoutubeError {
    fn from(e: VideoError) -> Self {
        Self::Tool(e)
    }
}
