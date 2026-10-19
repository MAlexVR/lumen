//! Pluralsight adapter credentials and platform integrations.

pub mod client;
pub mod credentials;
pub mod downloader;
pub mod error;
pub mod parse;
pub mod planner;
pub mod platform;

pub use client::{ClientConfig, PluralsightApi, PluralsightClient};
pub use credentials::Credentials;
pub use error::PluralsightError;
pub use parse::{Clip, ClipDelivery, CourseInfo, ModuleInfo};
pub use platform::{PluralsightPlatform, PluralsightPlatformApi};
