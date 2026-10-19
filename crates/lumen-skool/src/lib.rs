//! Skool adapter: pure page parsers, HTTP client, planner and downloader.
//! Everything Skool-specific lives here; `lumen-core` only sees the port.

pub mod client;
pub mod downloader;
pub mod error;
pub mod links;
pub mod parse;
pub mod planner;
pub mod platform;
pub mod richtext;
pub mod session;

pub use client::{ClientConfig, SkoolClient};
pub use error::SkoolError;
pub use platform::SkoolPlatform;
