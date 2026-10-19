//! Lumen domain core: pure types and ports, no UI or network framework dependencies.

pub mod error;
pub mod hotmart_scan;
pub mod model;
pub mod platform;
pub mod pluralsight_scan;
pub mod progress;
pub mod report;
pub mod sanitize;
pub mod scanner;
mod youtube_scan;

pub use error::ScanError;
pub use model::{
    Course, Group, GroupStatus, Lesson, Library, LibraryPlatform, PlatformLibrary, Resource,
    ResourceKind, Section,
};
pub use platform::{
    CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, NullSink, PlatformError,
    PlatformGroup,
};
pub use report::{redact_url, DownloadProgress, DownloadReport, Issue, IssueKind, Severity};
pub use scanner::LibraryScanner;
