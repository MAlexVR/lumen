//! Platform-agnostic download building blocks shared by every course platform
//! adapter (Skool, Hotmart, ...): the yt-dlp runner, a retrying HTTP helper
//! with URL redaction, `.part` streaming, the report writer with the
//! `.complete` rule, and the Windows-safe folder layout helpers.

pub mod files;
pub mod http;
pub mod layout;
pub mod panda;
pub mod report;
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::unwrap_used)] // test double: panicking on a broken fixture is the point
pub mod testsrv;
pub mod ytdlp;

pub use http::{FetchError, RetryPolicy};
pub use ytdlp::{locate_ffmpeg, MediaJob, MediaTool, VideoError, VideoJob, VideoRunner, YtDlp};
