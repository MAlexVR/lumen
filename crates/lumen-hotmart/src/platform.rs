//! `CoursePlatform` implementation for Hotmart. Every purchased course is one
//! "group" of the port, identified by its product id.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use lumen_core::{
    CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadReport, PlatformError,
    PlatformGroup,
};
use lumen_download::VideoRunner;
use serde::Serialize;

use crate::client::HotmartClient;
use crate::downloader::{CourseDownloader, Retry, PLATFORM_ID};
use crate::parse::HotmartCourse;
use crate::planner::course_dir;

/// A purchased course as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseInfo {
    /// Product id: what `download_group` takes.
    pub id: String,
    pub name: String,
    pub seller: Option<String>,
    pub cover_url: Option<String>,
    /// `YYYY-MM-DD` when Hotmart gave a recognisable purchase date.
    pub purchase_date: Option<String>,
    /// Folder name under `Hotmart/` where the course is (or will be) stored.
    pub folder: String,
}

impl CourseInfo {
    pub fn from_course(course: &HotmartCourse) -> Self {
        let folder = course_dir(Path::new(""), course)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            id: course.id.clone(),
            name: course.name.clone(),
            seller: course.seller.clone(),
            cover_url: course.cover_url.clone(),
            purchase_date: course.purchase_date.clone(),
            folder,
        }
    }
}

/// The port plus the Hotmart-specific course listing.
#[async_trait]
pub trait HotmartApi: CoursePlatform {
    async fn courses(&self) -> Result<Vec<CourseInfo>, PlatformError>;
}

pub struct HotmartPlatform {
    client: Arc<HotmartClient>,
    videos: Arc<dyn VideoRunner>,
    retry: Retry,
}

impl HotmartPlatform {
    pub fn new(client: Arc<HotmartClient>, videos: Arc<dyn VideoRunner>) -> Self {
        Self {
            client,
            videos,
            retry: Retry::default(),
        }
    }

    #[must_use]
    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }
}

#[async_trait]
impl CoursePlatform for HotmartPlatform {
    fn id(&self) -> &'static str {
        PLATFORM_ID
    }

    async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
        Ok(self
            .client
            .courses()
            .await?
            .courses
            .into_iter()
            .map(|c| PlatformGroup {
                slug: c.id,
                name: c.name,
                cover_url: c.cover_url,
                num_courses: 1,
            })
            .collect())
    }

    async fn download_group(
        &self,
        group_slug: &str,
        library_root: &Path,
        options: &DownloadOptions,
        sink: &dyn CourseProgressSink,
        cancel: &CancelFlag,
    ) -> Result<DownloadReport, PlatformError> {
        let course = self
            .client
            .courses()
            .await?
            .courses
            .into_iter()
            .find(|c| c.id == group_slug)
            .ok_or_else(|| {
                PlatformError::InvalidInput(format!("unknown course \"{group_slug}\""))
            })?;
        let downloader = CourseDownloader {
            source: self.client.as_ref(),
            videos: self.videos.as_ref(),
            sink,
            cancel,
            options: options.clone(),
            retry: self.retry.clone(),
        };
        Ok(downloader.run(&course, library_root).await?)
    }
}

#[async_trait]
impl HotmartApi for HotmartPlatform {
    async fn courses(&self) -> Result<Vec<CourseInfo>, PlatformError> {
        Ok(self
            .client
            .courses()
            .await?
            .courses
            .iter()
            .map(CourseInfo::from_course)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ClientConfig, Endpoints};
    use crate::credentials::Credentials;
    use lumen_core::NullSink;
    use lumen_download::testsrv::{Recorded, Reply, TestServer};
    use lumen_download::{RetryPolicy, VideoError, VideoJob};
    use std::sync::Mutex;
    use std::time::Duration;

    const PURCHASES: &str = include_str!("../tests/fixtures/purchases.json");

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
        if p.starts_with("/check_token") {
            Reply::json(include_str!("../tests/fixtures/check_token.json"))
        } else if p.starts_with("/purchase/products/") {
            Reply::json(
                r#"{"product":{"membership":{"registerAddress":"https://area.example.test/"}}}"#,
            )
        } else if p.starts_with("/purchase/") {
            Reply::json(PURCHASES)
        } else if p == "/navigation" {
            Reply::json(
                r#"{"modules":[{"name":"Start","pages":[{"hash":"h1","name":"Welcome"}]}]}"#,
            )
        } else if p == "/lesson/h1" {
            Reply::json(
                r#"{"attachments":[{"fileMembershipId":"a1","fileName":"Plan.pdf"}],
                    "content":"<p>Hi</p><iframe src=\"https://player.vimeo.com/video/9\"></iframe>"}"#,
            )
        } else if p == "/attachment/a1/download" {
            Reply::json(&format!(
                r#"{{"directDownloadUrl":"http://127.0.0.1:{port}/files/plan.pdf"}}"#
            ))
        } else if p == "/files/plan.pdf" {
            Reply::bytes(200, "application/pdf", b"%PDF".to_vec())
        } else {
            Reply::status(404)
        }
    }

    fn platform(srv: &TestServer, videos: Arc<FakeVideo>) -> HotmartPlatform {
        let cfg = ClientConfig {
            endpoints: Endpoints::under(&srv.base()),
            retry: RetryPolicy {
                max_attempts: 2,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(2),
            },
        };
        let client =
            HotmartClient::with_config(Credentials::new("tok", None).unwrap(), cfg).unwrap();
        HotmartPlatform::new(Arc::new(client), videos).with_retry(Retry {
            attempts: 2,
            unit: Duration::ZERO,
        })
    }

    #[tokio::test]
    async fn groups_are_the_purchased_courses_keyed_by_product_id() {
        let srv = TestServer::start(site).await;
        let p = platform(&srv, Arc::default());
        assert_eq!(p.id(), "hotmart");
        let groups = p.groups().await.unwrap();
        let slugs: Vec<_> = groups.iter().map(|g| g.slug.as_str()).collect();
        assert_eq!(slugs, ["1001", "1002", "1003", "1005"]);
        assert_eq!(groups[0].name, "METODO HORIZONTE");
        assert_eq!(
            groups[0].cover_url.as_deref(),
            Some("https://cdn.example.test/horizonte.jpg")
        );
        assert!(groups.iter().all(|g| g.num_courses == 1));
    }

    #[tokio::test]
    async fn courses_expose_seller_date_and_the_library_folder() {
        let srv = TestServer::start(site).await;
        let courses = platform(&srv, Arc::default()).courses().await.unwrap();
        assert_eq!(courses[0].folder, "METODO HORIZONTE - Marta Ledesma");
        assert_eq!(courses[0].purchase_date.as_deref(), Some("2024-03-09"));
        assert_eq!(courses[1].folder, "Yoga y Flexibilidad - Bruno Carrasco");
        let json = serde_json::to_value(&courses[0]).unwrap();
        assert_eq!(json["coverUrl"], "https://cdn.example.test/horizonte.jpg");
        assert_eq!(json["purchaseDate"], "2024-03-09");
        assert_eq!(json["id"], "1001");
    }

    #[tokio::test]
    async fn download_group_runs_the_whole_pipeline_over_http() {
        let srv = TestServer::start(site).await;
        let videos = Arc::new(FakeVideo::default());
        let p = platform(&srv, videos.clone());
        let tmp = tempfile::tempdir().unwrap();
        let report = p
            .download_group(
                "1001",
                tmp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(report.platform, "hotmart");
        let course = tmp.path().join("Hotmart/METODO HORIZONTE - Marta Ledesma");
        assert!(course.join(".complete").is_file());
        let lesson = course.join("1. Start/1. Welcome");
        assert_eq!(
            std::fs::read(lesson.join("Materials/Plan.pdf")).unwrap(),
            b"%PDF"
        );
        assert!(lesson.join("Description.html").is_file());
        assert!(lesson.join("1. Lesson.mp4").is_file());
        let jobs = videos.jobs.lock().unwrap();
        assert_eq!(
            jobs[0].referer.as_deref(),
            Some("https://area.example.test/")
        );
    }

    #[tokio::test]
    async fn errors_map_to_the_core_error_kinds() {
        let srv = TestServer::start(site).await;
        let p = platform(&srv, Arc::default());
        let tmp = tempfile::tempdir().unwrap();
        let unknown = p
            .download_group(
                "999",
                tmp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(unknown, PlatformError::InvalidInput(m) if m.contains("999")));
        let expired = TestServer::start(|_, _| Reply::status(401)).await;
        assert_eq!(
            platform(&expired, Arc::default())
                .groups()
                .await
                .unwrap_err(),
            PlatformError::SessionExpired
        );
        let down = TestServer::start(|_, _| Reply::status(500)).await;
        assert!(matches!(
            platform(&down, Arc::default()).courses().await.unwrap_err(),
            PlatformError::Network(_)
        ));
    }
}
