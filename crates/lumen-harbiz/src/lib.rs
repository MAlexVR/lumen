//! Harbiz adapter: the coaching app (https://app.harbiz.io) is a Meteor app, so
//! data travels over DDP (websocket). A client's agenda is a list of calendar
//! events; workouts carry exercises whose demo videos are YouTube links,
//! downloaded through the shared yt-dlp runner.

pub mod error;
pub mod parse;

pub use error::HarbizError;
pub use parse::{Exercise, Parsed, Workout};
