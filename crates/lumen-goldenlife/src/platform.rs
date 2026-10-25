//! CoursePlatform adapter for GoldenLife.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use lumen_core::{
    CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadReport, PlatformError,
    PlatformGroup,
};
use lumen_download::VideoRunner;
use serde::Serialize;

use crate::client::GoldenlifeApi;
use crate::downloader::{CourseDownloader, Retry, PLATFORM_ID};
use crate::error::GoldenlifeError;
use crate::parse::CourseInfo;
use crate::planner::course_dir;

/// Listing row for the UI; `folder` is where the course lands in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenlifeCourseInfo {
    pub id: Option<String>,
    pub title: String,
    pub shelf: Option<String>,
    pub image_url: Option<String>,
    pub folder: String,
    pub issue: Option<String>,
}

impl GoldenlifeCourseInfo {
    pub fn from_course_at(course: &CourseInfo, library_root: &Path) -> Result<Self, PlatformError> {
        let folder = course_dir(library_root, &course.title)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| {
                PlatformError::InvalidInput("invalid GoldenLife course folder".into())
            })?;
        Ok(Self {
            id: Some(course.id.clone()),
            title: course.title.clone(),
            shelf: course.shelf.clone(),
            image_url: course.image_url.clone(),
            folder,
            issue: None,
        })
    }
}

/// Course listing plus the shared course-platform implementation.
#[async_trait]
pub trait GoldenlifePlatformApi: CoursePlatform {
    async fn courses(
        &self,
        library_root: &Path,
    ) -> Result<Vec<GoldenlifeCourseInfo>, PlatformError>;
}

pub struct GoldenlifePlatform {
    api: Arc<dyn GoldenlifeApi>,
    videos: Arc<dyn VideoRunner>,
    retry: Retry,
}

impl GoldenlifePlatform {
    pub fn new(api: Arc<dyn GoldenlifeApi>, videos: Arc<dyn VideoRunner>) -> Self {
        Self {
            api,
            videos,
            retry: Retry::default(),
        }
    }

    #[must_use]
    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    async fn listed_courses(&self) -> Result<Vec<CourseInfo>, PlatformError> {
        self.api.courses().await.map_err(PlatformError::from)
    }
}

#[async_trait]
impl CoursePlatform for GoldenlifePlatform {
    fn id(&self) -> &'static str {
        PLATFORM_ID
    }

    async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
        Ok(self
            .listed_courses()
            .await?
            .into_iter()
            .map(|course| PlatformGroup {
                slug: course.id,
                name: course.title,
                cover_url: course.image_url,
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
            .listed_courses()
            .await?
            .into_iter()
            .find(|course| course.id == group_slug)
            .ok_or_else(|| PlatformError::InvalidInput("unknown GoldenLife course".into()))?;
        Ok(CourseDownloader {
            api: self.api.as_ref(),
            videos: self.videos.as_ref(),
            sink,
            cancel,
            options: options.clone(),
            retry: self.retry.clone(),
        }
        .run(&course, library_root)
        .await?)
    }
}

#[async_trait]
impl GoldenlifePlatformApi for GoldenlifePlatform {
    async fn courses(
        &self,
        library_root: &Path,
    ) -> Result<Vec<GoldenlifeCourseInfo>, PlatformError> {
        self.listed_courses()
            .await?
            .iter()
            .map(|course| GoldenlifeCourseInfo::from_course_at(course, library_root))
            .collect()
    }
}

impl From<GoldenlifeError> for PlatformError {
    fn from(error: GoldenlifeError) -> Self {
        match error {
            GoldenlifeError::Unauthorized => Self::SessionExpired,
            GoldenlifeError::InvalidCredentials => Self::InvalidInput(error.to_string()),
            GoldenlifeError::Network(_) | GoldenlifeError::HttpStatus(_) => {
                Self::Network(error.to_string())
            }
            GoldenlifeError::Io(message) => Self::Io(message),
            other => Self::Parse(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{LessonPage, LessonRef, Outline, Section};
    use lumen_core::NullSink;
    use lumen_download::{VideoError, VideoJob};

    struct FakeApi(Vec<CourseInfo>);

    #[async_trait]
    impl GoldenlifeApi for FakeApi {
        async fn validate(&self) -> Result<(), GoldenlifeError> {
            Ok(())
        }
        async fn courses(&self) -> Result<Vec<CourseInfo>, GoldenlifeError> {
            Ok(self.0.clone())
        }
        async fn outline(&self, _: &str) -> Result<Outline, GoldenlifeError> {
            Ok(Outline {
                product_id: None,
                sections: vec![Section {
                    id: "s".into(),
                    title: "Modulo".into(),
                    lessons: vec![LessonRef {
                        id: "1".into(),
                        title: "Clase".into(),
                    }],
                }],
            })
        }
        async fn lesson(&self, _: &str) -> Result<LessonPage, GoldenlifeError> {
            Ok(LessonPage {
                content_html: "<p>x</p>".into(),
                ..LessonPage::default()
            })
        }
        async fn download_attachment(&self, _: &str, _: &Path) -> Result<(), GoldenlifeError> {
            Ok(())
        }
        fn referer(&self) -> String {
            "https://academy.example.test/".into()
        }
    }

    struct NoVideos;

    #[async_trait]
    impl VideoRunner for NoVideos {
        async fn download(
            &self,
            _: &VideoJob,
            _: &CancelFlag,
            _: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            Err(VideoError::Failed("unused".into()))
        }
    }

    fn course(id: &str, title: &str) -> CourseInfo {
        CourseInfo {
            id: id.into(),
            title: title.into(),
            shelf: Some("UGC".into()),
            image_url: Some("https://cdn.example.test/c.webp".into()),
        }
    }

    fn platform(courses: Vec<CourseInfo>) -> GoldenlifePlatform {
        GoldenlifePlatform::new(Arc::new(FakeApi(courses)), Arc::new(NoVideos))
    }

    #[tokio::test]
    async fn courses_are_listed_with_their_library_folder() {
        let p = platform(vec![course("1", "Meta Ads: Basico?")]);
        let listed = p.courses(Path::new("/lib")).await.unwrap();
        assert_eq!(
            listed,
            vec![GoldenlifeCourseInfo {
                id: Some("1".into()),
                title: "Meta Ads: Basico?".into(),
                shelf: Some("UGC".into()),
                image_url: Some("https://cdn.example.test/c.webp".into()),
                folder: "Meta Ads_ Basico_".into(),
                issue: None,
            }]
        );
        let json = serde_json::to_value(&listed[0]).unwrap();
        assert_eq!(json["imageUrl"], "https://cdn.example.test/c.webp");
        let groups = p.groups().await.unwrap();
        assert_eq!((groups[0].slug.as_str(), groups[0].num_courses), ("1", 1));
        assert_eq!(p.id(), "goldenlife");
    }

    #[tokio::test]
    async fn an_unknown_course_is_rejected_and_a_known_one_downloads() {
        let p = platform(vec![course("1", "Curso")]);
        let root = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        let err = p
            .download_group(
                "2",
                root.path(),
                &DownloadOptions::default(),
                &NullSink,
                &cancel,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PlatformError::InvalidInput(_)), "{err:?}");
        assert!(
            !root.path().join("GoldenLife").exists(),
            "nothing is created for it"
        );

        let report = p
            .download_group(
                "1",
                root.path(),
                &DownloadOptions::default(),
                &NullSink,
                &cancel,
            )
            .await
            .unwrap();
        assert_eq!(report.platform, "goldenlife");
        assert!(report.complete, "{report:?}");
        assert!(root
            .path()
            .join("GoldenLife/Curso/1. Modulo/1. Clase/Description.html")
            .is_file());
    }

    #[test]
    fn errors_map_to_platform_errors() {
        assert_eq!(
            PlatformError::from(GoldenlifeError::Unauthorized),
            PlatformError::SessionExpired
        );
        assert!(matches!(
            PlatformError::from(GoldenlifeError::HttpStatus(500)),
            PlatformError::Network(_)
        ));
        assert!(matches!(
            PlatformError::from(GoldenlifeError::InvalidResponse("x")),
            PlatformError::Parse(_)
        ));
        assert!(matches!(
            PlatformError::from(GoldenlifeError::InvalidCredentials),
            PlatformError::InvalidInput(_)
        ));
    }
}
