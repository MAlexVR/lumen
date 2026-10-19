//! Loopback HTTP server that streams files from the local library to the
//! webview. Other local processes and web pages can reach loopback, so every
//! request needs a per-run token in the path and an exact `Host` match.

mod confine;
mod error;
mod server;
mod thumbs;

pub use confine::resolve_confined;
pub use error::MediaError;
pub use server::MediaServer;
pub use thumbs::{
    ThumbConfig, ThumbDone, ThumbItem, ThumbReply, ThumbService, ThumbState, ThumbStatus,
};
