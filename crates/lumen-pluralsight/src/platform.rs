//! CoursePlatform adapter for Pluralsight.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use lumen_core::{
    CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadReport, PlatformError,
    PlatformGroup,
};
use lumen_download::VideoRunner;
use serde::Serialize;

use crate::client::PluralsightApi;
use crate::downloader::{PluralsightDownloader, Retry};
use crate::parse::CourseInfo;
use crate::planner::course_dir;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluralsightCourseInfo {
    pub id: Option<String>,
    pub title: String,
    pub level: Option<String>,
    pub image_url: Option<String>,
    pub folder: String,
    pub issue: Option<String>,
}

impl PluralsightCourseInfo {
    /// Builds listing metadata with the same root-aware folder allocation as downloads.
    pub fn from_course_at(course: &CourseInfo, library_root: &Path) -> Result<Self, PlatformError> {
        let folder = course_dir(library_root, course)?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| {
                PlatformError::InvalidInput("invalid Pluralsight course folder".into())
            })?;
        Ok(Self {
            id: course.id.clone(),
            title: course.title.clone(),
            level: course.level.clone(),
            image_url: course.image_url.clone(),
            folder,
            issue: course.issue.clone(),
        })
    }
}

/// Course listing and shared course-platform implementation.
#[async_trait]
pub trait PluralsightPlatformApi: CoursePlatform {
    async fn courses(
        &self,
        library_root: &Path,
    ) -> Result<Vec<PluralsightCourseInfo>, PlatformError>;
}

pub struct PluralsightPlatform {
    client: Arc<dyn PluralsightApi>,
    videos: Arc<dyn VideoRunner>,
    retry: Retry,
}

impl PluralsightPlatform {
    pub fn new(client: Arc<dyn PluralsightApi>, videos: Arc<dyn VideoRunner>) -> Self {
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

    async fn listed_courses(&self) -> Result<Vec<CourseInfo>, PlatformError> {
        self.client.courses().await.map_err(PlatformError::from)
    }
}

#[async_trait]
impl CoursePlatform for PluralsightPlatform {
    fn id(&self) -> &'static str {
        "pluralsight"
    }

    async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
        self.listed_courses()
            .await?
            .into_iter()
            .map(|course| {
                let id = course
                    .id
                    .clone()
                    .filter(|id| !id.trim().is_empty())
                    .ok_or_else(|| {
                        PlatformError::Parse(
                            course
                                .issue
                                .clone()
                                .unwrap_or_else(|| "course record is missing an id".into()),
                        )
                    })?;
                Ok(PlatformGroup {
                    slug: id,
                    name: course.title,
                    cover_url: course.image_url,
                    num_courses: 1,
                })
            })
            .collect()
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
            .find(|course| course.id.as_deref() == Some(group_slug))
            .ok_or_else(|| PlatformError::InvalidInput("unknown Pluralsight course".into()))?;
        PluralsightDownloader {
            api: self.client.clone(),
            videos: self.videos.clone(),
            retry: self.retry.clone(),
        }
        .download(&course, library_root, options, sink, cancel)
        .await
    }
}

#[async_trait]
impl PluralsightPlatformApi for PluralsightPlatform {
    async fn courses(
        &self,
        library_root: &Path,
    ) -> Result<Vec<PluralsightCourseInfo>, PlatformError> {
        Ok(self
            .listed_courses()
            .await?
            .iter()
            .map(|course| PluralsightCourseInfo::from_course_at(course, library_root))
            .collect::<Result<Vec<_>, _>>()?)
    }
}

impl From<crate::PluralsightError> for PlatformError {
    fn from(error: crate::PluralsightError) -> Self {
        match error {
            crate::PluralsightError::Unauthorized(_) => Self::SessionExpired,
            crate::PluralsightError::Network(_) | crate::PluralsightError::HttpStatus(_) => {
                Self::Network("Pluralsight service request failed".into())
            }
            crate::PluralsightError::InvalidCredentials(message) => Self::InvalidInput(message),
            crate::PluralsightError::Io => {
                Self::Io("Pluralsight filesystem operation failed".into())
            }
            crate::PluralsightError::ReportWrite => {
                Self::Io("Pluralsight download report could not be written".into())
            }
            crate::PluralsightError::InvalidCourseId => {
                Self::InvalidInput("invalid Pluralsight course identifier".into())
            }
            other => Self::Parse(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipDelivery, ModuleInfo, PluralsightError};

    struct FakeApi(Vec<CourseInfo>);
    #[async_trait]
    impl PluralsightApi for FakeApi {
        async fn validate(&self) -> Result<(), PluralsightError> {
            Ok(())
        }
        async fn courses(&self) -> Result<Vec<CourseInfo>, PluralsightError> {
            Ok(self.0.clone())
        }
        async fn toc(&self, _: &str) -> Result<Vec<ModuleInfo>, PluralsightError> {
            Ok(vec![])
        }
        async fn clip_delivery(&self, _: &str, _: &str) -> Result<ClipDelivery, PluralsightError> {
            Err(PluralsightError::NoStream)
        }
    }

    #[tokio::test]
    async fn platform_lists_history_courses_and_maps_stable_course_ids() {
        let api = Arc::new(FakeApi(vec![CourseInfo {
            id: Some("ps-1".into()),
            title: "Course".into(),
            level: Some("Intermediate".into()),
            image_url: Some("https://images.example.test/course.jpg".into()),
            issue: None,
        }]));
        let platform =
            PluralsightPlatform::new(api, Arc::new(lumen_download::YtDlp::new("uninvoked")));
        assert_eq!(platform.id(), "pluralsight");
        assert_eq!(
            platform.groups().await.unwrap(),
            vec![PlatformGroup {
                slug: "ps-1".into(),
                name: "Course".into(),
                cover_url: Some("https://images.example.test/course.jpg".into()),
                num_courses: 1,
            }]
        );
        assert_eq!(
            PluralsightPlatformApi::courses(&platform, Path::new(""))
                .await
                .unwrap()[0]
                .folder,
            "Course-3f360a4cec83e633"
        );
    }

    #[tokio::test]
    async fn platform_rejects_history_row_without_id_instead_of_silently_dropping_it() {
        let api = Arc::new(FakeApi(vec![CourseInfo {
            id: None,
            title: "Malformed".into(),
            level: None,
            image_url: None,
            issue: Some("course record is missing an id".into()),
        }]));
        let platform =
            PluralsightPlatform::new(api, Arc::new(lumen_download::YtDlp::new("uninvoked")));
        assert!(matches!(
            platform.groups().await,
            Err(PlatformError::Parse(_))
        ));
        assert_eq!(
            PlatformError::from(PluralsightError::Unauthorized(403)),
            PlatformError::SessionExpired
        );
    }

    #[test]
    fn course_metadata_folder_uses_the_same_unicode_root_budget_as_downloads() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary
            .path()
            .join("界".repeat(28))
            .join("長い-library-root".repeat(7));
        let course = CourseInfo {
            id: Some("course-long-root".into()),
            title: "A Pluralsight course title long enough to allocate a root-sensitive folder"
                .into(),
            level: None,
            image_url: None,
            issue: None,
        };
        let expected = course_dir(&root, &course)
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let actual = PluralsightCourseInfo::from_course_at(&course, &root)
            .unwrap()
            .folder;

        assert_ne!(
            expected,
            course_dir(Path::new(""), &course)
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
        assert_eq!(actual, expected);
    }
}
