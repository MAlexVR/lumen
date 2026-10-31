//! Harbiz adapter: the coaching app (https://app.harbiz.io) is a Meteor app, so
//! data travels over DDP (websocket). A client's agenda is a list of calendar
//! events; workouts carry exercises whose demo videos are YouTube links,
//! downloaded through the shared yt-dlp runner.

pub mod client;
pub mod credentials;
pub mod ddp;
pub mod downloader;
pub mod error;
pub mod parse;
pub mod planner;
pub mod platform;
pub mod refresh;

pub use client::{login, Connector, DdpConnection, DdpError, HarbizApi, HarbizClient};
pub use credentials::{Login, Session};
pub use ddp::WsConnector;
pub use downloader::{WorkoutDownloader, PLATFORM_ID};
pub use error::HarbizError;
pub use parse::{Exercise, Parsed, Workout};
pub use platform::{HarbizCourseInfo, HarbizPlatform, HarbizPlatformApi, GROUP_SLUG};
pub use refresh::refresh_notes;
