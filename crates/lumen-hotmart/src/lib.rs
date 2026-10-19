//! Hotmart adapter: pure JSON/HTML parsers, credentials, folder naming, HTTP
//! client, planner and downloader. Everything Hotmart-specific lives here;
//! `lumen-core` only sees the `CoursePlatform` port.

pub mod client;
pub mod credentials;
pub mod downloader;
pub mod error;
pub mod naming;
pub mod parse;
pub mod planner;
pub mod platform;

pub use client::{ClientConfig, Endpoints, HotmartClient};
pub use credentials::Credentials;
pub use downloader::Retry;
pub use error::HotmartError;
pub use platform::{CourseInfo, HotmartApi, HotmartPlatform};
