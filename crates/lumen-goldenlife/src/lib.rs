//! GoldenLife adapter: the Golden Life academy (https://my.goldenlife-academy.com)
//! runs on the Cademí LMS. Login is a CSRF form post; the catalogue, course
//! outline and lessons are server-rendered HTML; videos are Panda Video (plain
//! HLS) or YouTube embeds, downloaded through the shared yt-dlp runner.

pub mod client;
pub mod credentials;
pub mod downloader;
pub mod error;
pub mod parse;
pub mod planner;
pub mod platform;

pub use client::{ClientConfig, GoldenlifeApi, GoldenlifeClient};
pub use credentials::{Login, Session};
pub use downloader::{CourseDownloader, Retry, PLATFORM_ID};
pub use error::GoldenlifeError;
pub use parse::{CourseInfo, LessonPage, Outline};
pub use platform::{GoldenlifeCourseInfo, GoldenlifePlatform, GoldenlifePlatformApi};
