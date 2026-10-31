//! CoursePlatform adapter for Harbiz: the whole agenda is one group, "Harbiz
//! Training", whose lessons are the workouts.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use lumen_core::{
    CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadReport, PlatformError,
    PlatformGroup,
};
use lumen_download::VideoRunner;
use serde::Serialize;

use crate::client::HarbizApi;
use crate::downloader::{WorkoutDownloader, PLATFORM_ID};
use crate::error::HarbizError;
use crate::parse::{Parsed, Workout};
use crate::planner::{course_dir, is_downloaded, plan_workouts, COURSE_TITLE};

/// Slug of the only group.
pub const GROUP_SLUG: &str = "harbiz";

/// Listing row for the UI: what is on the agenda and what is already local.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarbizCourseInfo {
    pub id: String,
    pub title: String,
    pub folder: String,
    pub workouts: u32,
    pub downloaded: u32,
    /// Workouts on the agenda that are not (fully) in the library yet.
    pub pending: u32,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub photo_checkins: u32,
}

#[async_trait]
pub trait HarbizPlatformApi: CoursePlatform {
    async fn courses(&self, library_root: &Path) -> Result<Vec<HarbizCourseInfo>, PlatformError>;
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

pub struct HarbizPlatform {
    api: Arc<dyn HarbizApi>,
    videos: Arc<dyn VideoRunner>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl HarbizPlatform {
    pub fn new(api: Arc<dyn HarbizApi>, videos: Arc<dyn VideoRunner>) -> Self {
        Self {
            api,
            videos,
            clock: Arc::new(system_now_ms),
        }
    }

    /// Replaces the clock (milliseconds since the epoch) the agenda is read with.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }
}

impl HarbizPlatform {
    async fn agenda(&self) -> Result<Parsed, PlatformError> {
        Ok(self.api.workouts((self.clock)()).await?)
    }
}

#[async_trait]
impl CoursePlatform for HarbizPlatform {
    fn id(&self) -> &'static str {
        PLATFORM_ID
    }

    async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
        // Reading the agenda proves the session works before offering the group.
        self.agenda().await?;
        Ok(vec![PlatformGroup {
            slug: GROUP_SLUG.to_string(),
            name: COURSE_TITLE.to_string(),
            cover_url: None,
            num_courses: 1,
        }])
    }

    async fn download_group(
        &self,
        group_slug: &str,
        library_root: &Path,
        options: &DownloadOptions,
        sink: &dyn CourseProgressSink,
        cancel: &CancelFlag,
    ) -> Result<DownloadReport, PlatformError> {
        if group_slug != GROUP_SLUG {
            return Err(PlatformError::InvalidInput("unknown Harbiz group".into()));
        }
        let parsed = self.agenda().await?;
        Ok(WorkoutDownloader {
            videos: self.videos.as_ref(),
            sink,
            cancel,
            options: options.clone(),
        }
        .run(&parsed, library_root)
        .await?)
    }
}

#[async_trait]
impl HarbizPlatformApi for HarbizPlatform {
    async fn courses(&self, library_root: &Path) -> Result<Vec<HarbizCourseInfo>, PlatformError> {
        let parsed = self.agenda().await?;
        let dir = course_dir(library_root);
        let plan = plan_workouts(&dir, &parsed.workouts);
        let downloaded = plan.iter().filter(|p| is_downloaded(p)).count();
        let date = |w: Option<&Workout>| w.map(|w| w.date.clone());
        Ok(vec![HarbizCourseInfo {
            id: GROUP_SLUG.to_string(),
            title: COURSE_TITLE.to_string(),
            folder: COURSE_TITLE.to_string(),
            workouts: count(plan.len()),
            downloaded: count(downloaded),
            pending: count(plan.len() - downloaded),
            first_date: date(parsed.workouts.first()),
            last_date: date(parsed.workouts.last()),
            photo_checkins: count(parsed.photo_checkins),
        }])
    }
}

impl From<HarbizError> for PlatformError {
    fn from(error: HarbizError) -> Self {
        match error {
            HarbizError::Unauthorized => Self::SessionExpired,
            HarbizError::InvalidCredentials => Self::InvalidInput(error.to_string()),
            HarbizError::Network(_) => Self::Network(error.to_string()),
            HarbizError::Io(message) => Self::Io(message),
            HarbizError::InvalidResponse(_) => Self::Parse(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Exercise;
    use lumen_core::NullSink;
    use lumen_download::{VideoError, VideoJob};
    use serde_json::Value;
    use std::sync::Mutex;

    struct FakeApi {
        reply: Mutex<Result<Parsed, HarbizError>>,
        asked_at: Mutex<Vec<i64>>,
    }

    impl FakeApi {
        fn new(reply: Result<Parsed, HarbizError>) -> Arc<Self> {
            Arc::new(Self {
                reply: Mutex::new(reply),
                asked_at: Mutex::default(),
            })
        }
    }

    #[async_trait]
    impl HarbizApi for FakeApi {
        async fn workouts(&self, now_ms: i64) -> Result<Parsed, HarbizError> {
            self.asked_at.lock().unwrap().push(now_ms);
            self.reply.lock().unwrap().clone()
        }
    }

    #[derive(Default)]
    struct FakeVideos {
        urls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl VideoRunner for FakeVideos {
        async fn download(
            &self,
            job: &VideoJob,
            _cancel: &CancelFlag,
            _on_percent: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            self.urls.lock().unwrap().push(job.url.clone());
            let path = job
                .output_template
                .to_string_lossy()
                .replace("%(ext)s", "mp4");
            std::fs::write(path, "video").unwrap();
            Ok(())
        }
    }

    fn workout(id: &str, date: &str) -> Workout {
        Workout {
            id: id.to_string(),
            title: id.to_string(),
            date: date.to_string(),
            completed: true,
            description: String::new(),
            exercises: vec![Exercise {
                name: "Move".to_string(),
                url: Some(format!("https://youtu.be/{id}")),
                section: None,
                sets: String::new(),
                target: String::new(),
                rest: String::new(),
                notes: String::new(),
                result: Value::Null,
            }],
            result_general: Value::Null,
        }
    }

    fn agenda(workouts: Vec<Workout>) -> Parsed {
        Parsed {
            workouts,
            photo_checkins: 5,
            unsupported_events: 0,
        }
    }

    fn platform(api: Arc<FakeApi>, videos: Arc<FakeVideos>) -> HarbizPlatform {
        HarbizPlatform::new(api, videos).with_clock(Arc::new(|| 1_234))
    }

    async fn download(
        platform: &HarbizPlatform,
        root: &Path,
        slug: &str,
    ) -> Result<DownloadReport, PlatformError> {
        platform
            .download_group(
                slug,
                root,
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
    }

    #[tokio::test]
    async fn the_agenda_is_a_single_group() {
        let api = FakeApi::new(Ok(agenda(vec![workout("a", "2026-06-02")])));
        let platform = platform(api.clone(), Arc::default());
        assert_eq!(platform.id(), "harbiz");
        let groups = platform.groups().await.unwrap();
        assert_eq!(
            groups,
            [PlatformGroup {
                slug: "harbiz".to_string(),
                name: "Harbiz Training".to_string(),
                cover_url: None,
                num_courses: 1
            }]
        );
        assert_eq!(*api.asked_at.lock().unwrap(), [1_234]);
    }

    #[tokio::test]
    async fn an_expired_session_surfaces_when_listing() {
        let api = FakeApi::new(Err(HarbizError::Unauthorized));
        let platform = platform(api, Arc::default());
        assert!(matches!(
            platform.groups().await,
            Err(PlatformError::SessionExpired)
        ));
    }

    #[tokio::test]
    async fn an_unknown_group_downloads_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = Arc::new(FakeVideos::default());
        let api = FakeApi::new(Ok(agenda(vec![workout("a", "2026-06-02")])));
        let platform = platform(api, videos.clone());
        let err = download(&platform, tmp.path(), "nope").await.unwrap_err();
        assert!(matches!(err, PlatformError::InvalidInput(_)));
        assert!(videos.urls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn downloading_the_group_fetches_the_agenda_and_downloads_it() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = Arc::new(FakeVideos::default());
        let api = FakeApi::new(Ok(agenda(vec![workout("a", "2026-06-02")])));
        let platform = platform(api, videos.clone());
        let report = download(&platform, tmp.path(), GROUP_SLUG).await.unwrap();
        assert_eq!(report.platform, "harbiz");
        assert_eq!(report.completed_lessons, 1);
        assert_eq!(*videos.urls.lock().unwrap(), ["https://youtu.be/a"]);
    }

    #[tokio::test]
    async fn courses_count_what_is_local_and_what_is_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = Arc::new(FakeVideos::default());
        let api = FakeApi::new(Ok(agenda(vec![
            workout("a", "2026-06-02"),
            workout("b", "2026-06-04"),
        ])));
        let platform = platform(api.clone(), videos);

        let before = platform.courses(tmp.path()).await.unwrap();
        assert_eq!(before.len(), 1);
        let info = &before[0];
        assert_eq!(
            (info.id.as_str(), info.title.as_str(), info.folder.as_str()),
            ("harbiz", "Harbiz Training", "Harbiz Training")
        );
        assert_eq!((info.workouts, info.downloaded, info.pending), (2, 0, 2));
        assert_eq!(info.first_date.as_deref(), Some("2026-06-02"));
        assert_eq!(info.last_date.as_deref(), Some("2026-06-04"));
        assert_eq!(info.photo_checkins, 5);

        download(&platform, tmp.path(), GROUP_SLUG).await.unwrap();
        assert!(course_dir(tmp.path()).is_dir());
        let after = platform.courses(tmp.path()).await.unwrap();
        assert_eq!(
            (after[0].workouts, after[0].downloaded, after[0].pending),
            (2, 2, 0)
        );

        *api.reply.lock().unwrap() = Ok(agenda(vec![
            workout("a", "2026-06-02"),
            workout("b", "2026-06-04"),
            workout("c", "2026-10-31"),
        ]));
        let later = platform.courses(tmp.path()).await.unwrap();
        assert_eq!(
            (later[0].workouts, later[0].downloaded, later[0].pending),
            (3, 2, 1)
        );
        assert_eq!(later[0].last_date.as_deref(), Some("2026-10-31"));
    }

    #[tokio::test]
    async fn an_empty_agenda_lists_zero_workouts_without_dates() {
        let tmp = tempfile::tempdir().unwrap();
        let platform = platform(FakeApi::new(Ok(Parsed::default())), Arc::default());
        let info = platform.courses(tmp.path()).await.unwrap().remove(0);
        assert_eq!((info.workouts, info.pending), (0, 0));
        assert_eq!((info.first_date, info.last_date), (None, None));
    }

    #[test]
    fn errors_map_to_stable_platform_errors() {
        assert!(matches!(
            PlatformError::from(HarbizError::Unauthorized),
            PlatformError::SessionExpired
        ));
        assert!(matches!(
            PlatformError::from(HarbizError::InvalidCredentials),
            PlatformError::InvalidInput(_)
        ));
        assert!(matches!(
            PlatformError::from(HarbizError::Network("x".into())),
            PlatformError::Network(_)
        ));
        assert!(matches!(
            PlatformError::from(HarbizError::Io("x".into())),
            PlatformError::Io(_)
        ));
        assert!(matches!(
            PlatformError::from(HarbizError::InvalidResponse("x".into())),
            PlatformError::Parse(_)
        ));
    }
}
