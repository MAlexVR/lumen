//! Port implemented by every course platform (Skool first). The core knows
//! nothing about HTTP, HTML or yt-dlp; adapters live in their own crates.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::report::{DownloadProgress, DownloadReport};

/// A community/producer the user can download (a Skool group).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformGroup {
    pub slug: String,
    pub name: String,
    pub cover_url: Option<String>,
    pub num_courses: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOptions {
    /// Lessons processed at once. 1 keeps the sequential behaviour.
    pub max_parallel_lessons: usize,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            max_parallel_lessons: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    #[error("session expired or blocked")]
    SessionExpired,
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("file error: {0}")]
    Io(String),
    #[error("{0}")]
    InvalidInput(String),
}

/// Cooperative cancellation and pause shared between the UI commands and the
/// download. A paused download finishes the item in flight and then holds at
/// its next checkpoint until it is resumed or cancelled.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag {
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

/// Receives progress and the final report; implemented by the app shell.
pub trait CourseProgressSink: Send + Sync {
    fn on_progress(&self, progress: &DownloadProgress);
    fn on_report(&self, report: &DownloadReport);
}

/// Sink that discards everything.
pub struct NullSink;

impl CourseProgressSink for NullSink {
    fn on_progress(&self, _: &DownloadProgress) {}
    fn on_report(&self, _: &DownloadReport) {}
}

#[async_trait]
pub trait CoursePlatform: Send + Sync {
    /// Stable lowercase platform id used in reports (the library folder is named separately).
    fn id(&self) -> &'static str;

    async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError>;

    /// Downloads every accessible course of `group_slug` under `library_root`.
    /// Item-level failures go in the report; only fatal ones return `Err`.
    async fn download_group(
        &self,
        group_slug: &str,
        library_root: &Path,
        options: &DownloadOptions,
        sink: &dyn CourseProgressSink,
        cancel: &CancelFlag,
    ) -> Result<DownloadReport, PlatformError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn cancel_flag_is_shared_between_clones() {
        let a = CancelFlag::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }

    #[test]
    fn pause_is_shared_between_clones_and_independent_of_cancel() {
        let a = CancelFlag::new();
        let b = a.clone();
        assert!(!b.is_paused());
        a.pause();
        assert!(b.is_paused());
        assert!(!b.is_cancelled());
        b.resume();
        assert!(!a.is_paused());
    }

    #[test]
    fn cancelling_a_paused_flag_keeps_it_cancelled_after_resume() {
        let flag = CancelFlag::new();
        flag.pause();
        flag.cancel();
        assert!(flag.is_cancelled());
        flag.resume();
        assert!(flag.is_cancelled());
    }

    #[test]
    fn default_options_are_sequential() {
        assert_eq!(DownloadOptions::default().max_parallel_lessons, 1);
    }

    struct Fake {
        seen: Mutex<Vec<String>>,
    }

    impl CourseProgressSink for Fake {
        fn on_progress(&self, p: &DownloadProgress) {
            self.seen.lock().unwrap().push(p.lesson.clone());
        }
        fn on_report(&self, r: &DownloadReport) {
            self.seen
                .lock()
                .unwrap()
                .push(format!("report:{}", r.group));
        }
    }

    #[async_trait]
    impl CoursePlatform for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
            Err(PlatformError::SessionExpired)
        }
        async fn download_group(
            &self,
            group_slug: &str,
            _root: &Path,
            _o: &DownloadOptions,
            sink: &dyn CourseProgressSink,
            _c: &CancelFlag,
        ) -> Result<DownloadReport, PlatformError> {
            let mut r = DownloadReport::new("fake", group_slug);
            r.finish();
            sink.on_report(&r);
            Ok(r)
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_and_drives_the_sink() {
        let fake = Fake {
            seen: Mutex::new(vec![]),
        };
        let platform: &dyn CoursePlatform = &fake;
        assert_eq!(platform.groups().await, Err(PlatformError::SessionExpired));
        let r = platform
            .download_group(
                "g",
                Path::new("/tmp"),
                &DownloadOptions::default(),
                &fake,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(r.complete);
        assert_eq!(fake.seen.lock().unwrap().as_slice(), ["report:g"]);
    }
}
