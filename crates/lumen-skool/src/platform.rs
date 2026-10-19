//! `CoursePlatform` implementation for Skool.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use lumen_core::{
    CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadReport, PlatformError,
    PlatformGroup,
};

use crate::client::SkoolClient;
use crate::downloader::{GroupDownloader, PLATFORM_ID};
use lumen_download::VideoRunner;

pub struct SkoolPlatform {
    client: Arc<SkoolClient>,
    videos: Arc<dyn VideoRunner>,
}

impl SkoolPlatform {
    pub fn new(client: Arc<SkoolClient>, videos: Arc<dyn VideoRunner>) -> Self {
        Self { client, videos }
    }
}

#[async_trait]
impl CoursePlatform for SkoolPlatform {
    fn id(&self) -> &'static str {
        PLATFORM_ID
    }

    async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
        Ok(self.client.groups().await?)
    }

    async fn download_group(
        &self,
        group_slug: &str,
        library_root: &Path,
        options: &DownloadOptions,
        sink: &dyn CourseProgressSink,
        cancel: &CancelFlag,
    ) -> Result<DownloadReport, PlatformError> {
        let group = self
            .client
            .groups()
            .await?
            .into_iter()
            .find(|g| g.slug == group_slug)
            .ok_or_else(|| {
                PlatformError::InvalidInput(format!("unknown group \"{group_slug}\""))
            })?;
        let downloader = GroupDownloader {
            source: self.client.as_ref(),
            videos: self.videos.as_ref(),
            sink,
            cancel,
            options: options.clone(),
        };
        Ok(downloader.run(&group, library_root).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ClientConfig, RetryPolicy};
    use lumen_core::{IssueKind, NullSink, Severity};
    use lumen_download::testsrv::{Recorded, Reply, TestServer};
    use lumen_download::{VideoError, VideoJob};
    use std::sync::Mutex;
    use std::time::Duration;

    const CLASSROOM: &str = include_str!("../tests/fixtures/classroom.html");
    const COURSE: &str = include_str!("../tests/fixtures/course.html");
    const MODULE: &str = include_str!("../tests/fixtures/module.html");
    const GROUPS: &str = include_str!("../tests/fixtures/groups.json");

    #[derive(Default)]
    struct FakeVideo {
        jobs: Mutex<Vec<VideoJob>>,
    }

    #[async_trait]
    impl VideoRunner for FakeVideo {
        async fn download(
            &self,
            job: &VideoJob,
            _: &CancelFlag,
            _: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            self.jobs.lock().unwrap().push(job.clone());
            let file = job
                .output_template
                .to_string_lossy()
                .replace("%(ext)s", "mp4");
            std::fs::write(file, "v").map_err(|e| VideoError::Failed(e.to_string()))
        }
    }

    fn site(req: &Recorded, _: usize) -> Reply {
        let host = req.header("host").unwrap_or_default();
        let port = host.rsplit(':').next().unwrap_or_default();
        let p = req.path_and_query.as_str();
        match p {
            _ if p.starts_with("/self/groups") => Reply::json(GROUPS),
            "/growth-lab/classroom" => Reply::html(CLASSROOM),
            "/growth-lab/classroom/intro-course" => Reply::html(COURSE),
            "/growth-lab/classroom/intro-course?md=m1" => Reply::html(&MODULE.replace(
                "https://assets.skool.com/f/guide",
                &format!("http://127.0.0.1:{port}/files/guide.pdf"),
            )),
            _ if p.starts_with("/growth-lab/classroom/intro-course?md=") => Reply::html(COURSE),
            "/files/guide.pdf" => Reply::bytes(200, "application/pdf", b"%PDF".to_vec()),
            _ => Reply::status(404),
        }
    }

    fn platform(srv: &TestServer, videos: Arc<FakeVideo>) -> SkoolPlatform {
        let cfg = ClientConfig {
            web_base: srv.base(),
            api_base: srv.base(),
            retry: RetryPolicy {
                max_attempts: 2,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(2),
            },
            module_interval: Duration::ZERO,
        };
        let client = SkoolClient::with_config("auth_token=t", cfg).unwrap();
        SkoolPlatform::new(Arc::new(client), videos)
    }

    #[tokio::test]
    async fn lists_groups_through_the_port() {
        let srv = TestServer::start(site).await;
        let p = platform(&srv, Arc::default());
        assert_eq!(p.id(), "skool");
        let groups = p.groups().await.unwrap();
        assert_eq!(groups[0].name, "Growth Lab");
    }

    #[tokio::test]
    async fn downloads_a_group_end_to_end_against_a_mock_site() {
        let srv = TestServer::start(site).await;
        let videos = Arc::new(FakeVideo::default());
        let p = platform(&srv, Arc::clone(&videos));
        let tmp = tempfile::tempdir().unwrap();
        let report = p
            .download_group(
                "growth-lab",
                tmp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();

        let course = tmp.path().join("Skool/Growth Lab/01 - Intro Course");
        assert!(course
            .join("01 - Getting Started/01 - Welcome/Welcome.mp4")
            .is_file());
        assert!(course
            .join("01 - Getting Started/01 - Welcome/Guide.pdf")
            .is_file());
        let desc = std::fs::read_to_string(
            course.join("01 - Getting Started/01 - Welcome/description.html"),
        )
        .unwrap();
        assert!(desc.contains("Hi &lt;b&gt;"));
        assert!(course
            .join("01 - Getting Started/02 - External/External.mp4")
            .is_file());
        assert!(course.join("02 - Intro Course/01 - Standalone").is_dir());
        assert!(course
            .join("03 - Section/01 - Untitled set lesson")
            .is_dir());

        assert_eq!(report.total_lessons, 4);
        let urls: Vec<String> = videos
            .jobs
            .lock()
            .unwrap()
            .iter()
            .map(|j| j.url.clone())
            .collect();
        assert!(urls.contains(&"https://stream.mux.com/PB123.m3u8?token=TOK456".to_string()));
        assert!(urls.contains(&"https://www.youtube.com/watch?v=abc".to_string()));

        // "no-title" course page is a 404 on the mock site: one course error.
        let course_errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Course && i.severity == Severity::Error)
            .collect();
        assert_eq!(course_errors.len(), 1);
        assert_eq!(
            report
                .issues
                .iter()
                .filter(|i| i.kind == IssueKind::EmptyLesson)
                .count(),
            2
        );
        assert!(!report.complete);
        assert!(tmp
            .path()
            .join("Skool/Growth Lab/download-report.json")
            .is_file());
        assert!(!tmp.path().join("Skool/Growth Lab/.complete").exists());
        for r in srv.requests() {
            assert!(!r.path_and_query.contains("_next"));
        }
    }

    #[tokio::test]
    async fn unknown_group_is_an_input_error() {
        let srv = TestServer::start(site).await;
        let p = platform(&srv, Arc::default());
        let tmp = tempfile::tempdir().unwrap();
        let err = p
            .download_group(
                "nope",
                tmp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PlatformError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn expired_session_maps_to_the_core_error() {
        let srv = TestServer::start(|_, _| Reply::status(401)).await;
        let p = platform(&srv, Arc::default());
        assert_eq!(p.groups().await.unwrap_err(), PlatformError::SessionExpired);
    }
}
