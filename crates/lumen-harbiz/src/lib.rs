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

pub use client::{login, Connector, DdpConnection, DdpError, HarbizApi, HarbizClient};
pub use credentials::{Login, Session};
pub use ddp::WsConnector;
pub use error::HarbizError;
pub use parse::{Exercise, Parsed, Workout};
