//! YouTube adapter for single videos and playlists (not courses): URL
//! normalisation, yt-dlp metadata parsing, folder planning and the
//! downloader. Everything YouTube-specific lives here; `lumen-core` only sees
//! the scanner layout and the report types.

pub mod client;
pub mod downloader;
pub mod error;
pub mod parse;
pub mod planner;
pub mod reason;
pub mod url;

pub use client::{Resolved, ResolvedKind, Youtube};
pub use downloader::Request;
pub use error::YoutubeError;
pub use parse::ItemInfo;
pub use planner::Quality;
