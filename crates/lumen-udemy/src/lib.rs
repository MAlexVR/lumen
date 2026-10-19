//! Udemy adapter, phase 1: a tolerant client, parsers, a DRM-aware
//! classifier, a sanitizer and a diagnostic that measures what Udemy really
//! returns for the owner's session. Every endpoint, header and field here is a
//! HYPOTHESIS until a real diagnostic run; there is no downloader yet.

pub mod classify;
pub mod client;
pub mod credentials;
pub mod diagnose;
pub mod error;
pub mod parse;
pub mod sanitize;

pub use client::{ClientConfig, Endpoints, Pages, UdemyClient};
pub use credentials::Credentials;
pub use error::UdemyError;
