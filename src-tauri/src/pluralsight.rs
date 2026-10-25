//! Pluralsight session and download service. This module has no Tauri types.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lumen_core::PlatformError;
use lumen_download::{VideoRunner, YtDlp};
use lumen_pluralsight::{
    ClientConfig, Credentials, PluralsightApi, PluralsightClient, PluralsightError,
    PluralsightPlatform, PluralsightPlatformApi,
};
use serde::Serialize;

use crate::downloads::{
    CommandError, DownloadEvents, DownloadManager, ErrorCode, RunningDownload, SessionStatus,
};
use crate::session::SessionStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluralsightLoginOutcome {
    pub logged_in: bool,
    pub courses: Vec<lumen_pluralsight::platform::PluralsightCourseInfo>,
}

/// Builds authenticated API and CoursePlatform adapters; tests supply fakes.
pub trait PluralsightFactory: Send + Sync {
    fn build_api(
        &self,
        credentials: &Credentials,
    ) -> Result<Arc<dyn PluralsightApi>, PluralsightError>;
    fn build_platform(&self, api: Arc<dyn PluralsightApi>) -> Arc<dyn PluralsightPlatformApi>;
}

pub struct DefaultPluralsightFactory {
    videos: Arc<dyn VideoRunner>,
}

impl DefaultPluralsightFactory {
    pub fn new(ytdlp: YtDlp) -> Self {
        Self {
            videos: Arc::new(ytdlp),
        }
    }
}

impl PluralsightFactory for DefaultPluralsightFactory {
    fn build_api(
        &self,
        credentials: &Credentials,
    ) -> Result<Arc<dyn PluralsightApi>, PluralsightError> {
        Ok(Arc::new(PluralsightClient::new(
            credentials.clone(),
            ClientConfig::default(),
        )?))
    }

    fn build_platform(&self, api: Arc<dyn PluralsightApi>) -> Arc<dyn PluralsightPlatformApi> {
        Arc::new(PluralsightPlatform::new(api, Arc::clone(&self.videos)))
    }
}

pub struct PluralsightService {
    session: SessionStore,
    factory: Arc<dyn PluralsightFactory>,
    downloads: DownloadManager,
}

fn map(error: PluralsightError) -> CommandError {
    CommandError::from_platform(PlatformError::from(error), ErrorCode::InvalidCookies)
}

impl PluralsightService {
    pub fn new(session: SessionStore, factory: Arc<dyn PluralsightFactory>) -> Self {
        Self {
            session,
            factory,
            downloads: DownloadManager::new("pluralsight"),
        }
    }

    fn credentials(&self) -> Result<Credentials, CommandError> {
        let stored = self
            .session
            .load()
            .map_err(|error| CommandError::internal("cannot read the stored session", error))?
            .ok_or_else(|| {
                CommandError::new(ErrorCode::NotLoggedIn, "not logged in to Pluralsight")
            })?;
        Credentials::new(&stored).map_err(|_| {
            CommandError::new(
                ErrorCode::NotLoggedIn,
                "the stored Pluralsight session is unreadable: log in again",
            )
        })
    }

    /// Checks a webview cookie candidate without storing it. A stale cookie
    /// is an expected polling result; transport failures are returned to the UI.
    pub async fn validate_webview_candidate(&self, text: &str) -> Result<bool, CommandError> {
        let credentials = match Credentials::new(text) {
            Ok(credentials) => credentials,
            Err(_) => return Ok(false),
        };
        let api = self.factory.build_api(&credentials).map_err(map)?;
        match api.validate().await {
            Ok(()) => Ok(true),
            Err(PluralsightError::Unauthorized(_)) => Ok(false),
            Err(error) => Err(map(error)),
        }
    }

    /// Validates a raw or multiline Cookie header before persisting it.
    pub async fn login_with_cookies(
        &self,
        text: &str,
        library_root: &Path,
    ) -> Result<PluralsightLoginOutcome, CommandError> {
        let credentials = Credentials::new(text).map_err(map)?;
        let api = self.factory.build_api(&credentials).map_err(map)?;
        api.validate().await.map_err(map)?;
        let courses = self
            .factory
            .build_platform(api)
            .courses(library_root)
            .await
            .map_err(|error| CommandError::from_platform(error, ErrorCode::InvalidCookies))?;
        self.session
            .save(credentials.cookie_header())
            .map_err(|error| CommandError::internal("cannot store the session", error))?;
        Ok(PluralsightLoginOutcome {
            logged_in: true,
            courses,
        })
    }

    pub fn status(&self) -> Result<SessionStatus, CommandError> {
        match self.credentials() {
            Ok(_) => Ok(SessionStatus { logged_in: true }),
            Err(error) if error.code == ErrorCode::NotLoggedIn => {
                Ok(SessionStatus { logged_in: false })
            }
            Err(error) => Err(error),
        }
    }

    pub fn logout(&self) -> Result<(), CommandError> {
        self.downloads.cancel_all();
        self.session
            .clear()
            .map_err(|error| CommandError::internal("cannot remove the stored session", error))
    }

    pub async fn courses(
        &self,
        library_root: &Path,
    ) -> Result<Vec<lumen_pluralsight::platform::PluralsightCourseInfo>, CommandError> {
        let credentials = self.credentials()?;
        let api = self.factory.build_api(&credentials).map_err(map)?;
        self.factory
            .build_platform(api)
            .courses(library_root)
            .await
            .map_err(|error| CommandError::from_platform(error, ErrorCode::InvalidCookies))
    }

    pub fn start_download(
        &self,
        course_id: &str,
        library_root: PathBuf,
        events: Arc<dyn DownloadEvents>,
    ) -> Result<String, CommandError> {
        let credentials = self.credentials()?;
        if library_root.as_os_str().is_empty() {
            return Err(CommandError::new(
                ErrorCode::NoLibrary,
                "choose a library folder first",
            ));
        }
        crate::downloads::ensure_library_ready(&library_root)?;
        let api = self.factory.build_api(&credentials).map_err(map)?;
        self.downloads.start(
            course_id,
            self.factory.build_platform(api),
            library_root,
            events,
        )
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.downloads.cancel(id)
    }

    pub fn running(&self) -> Vec<RunningDownload> {
        self.downloads.running()
    }
}

/// Collects every cookie returned for the app-domain URL, including HttpOnly
/// cookies; the credential parser validates names and values before use.
pub fn cookie_header_from_jar<'a>(cookies: Vec<(&'a str, &'a str)>) -> Option<String> {
    let header = cookies
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    (!header.is_empty()).then_some(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::{ProgressEvent, ReportEvent};
    use async_trait::async_trait;
    use lumen_core::{
        CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadProgress,
        DownloadReport, PlatformGroup,
    };
    use lumen_pluralsight::{
        parse::{ClipDelivery, CourseInfo, ModuleInfo},
        platform::PluralsightCourseInfo,
    };
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::mpsc;

    const COOKIE: &str = "session=secret-cookie; XSRF-TOKEN=secret-xsrf";

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Quick,
        UntilCancelled,
    }

    struct FakeApi {
        validate_error: Mutex<Option<PluralsightError>>,
        mode: Mutex<Mode>,
        validations: Mutex<usize>,
        course_title: Mutex<String>,
    }

    #[async_trait]
    impl PluralsightApi for FakeApi {
        async fn validate(&self) -> Result<(), PluralsightError> {
            *self.validations.lock().unwrap() += 1;
            match &*self.validate_error.lock().unwrap() {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
        async fn courses(&self) -> Result<Vec<CourseInfo>, PluralsightError> {
            match &*self.validate_error.lock().unwrap() {
                Some(error) => Err(error.clone()),
                None => Ok(vec![CourseInfo {
                    id: Some("course-1".into()),
                    title: self.course_title.lock().unwrap().clone(),
                    level: Some("Intermediate".into()),
                    image_url: None,
                    issue: None,
                }]),
            }
        }
        async fn toc(&self, _course_id: &str) -> Result<Vec<ModuleInfo>, PluralsightError> {
            Ok(vec![])
        }
        async fn clip_delivery(
            &self,
            _clip_id: &str,
            _version_id: &str,
        ) -> Result<ClipDelivery, PluralsightError> {
            Err(PluralsightError::NoStream)
        }
    }

    #[async_trait]
    impl CoursePlatform for FakeApi {
        fn id(&self) -> &'static str {
            "pluralsight"
        }
        async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
            Ok(vec![PlatformGroup {
                slug: "course-1".into(),
                name: "Synthetic Course".into(),
                cover_url: None,
                num_courses: 1,
            }])
        }
        async fn download_group(
            &self,
            group: &str,
            _root: &Path,
            _options: &DownloadOptions,
            sink: &dyn CourseProgressSink,
            cancel: &CancelFlag,
        ) -> Result<DownloadReport, PlatformError> {
            let progress = DownloadProgress {
                group: group.into(),
                course: "Synthetic Course".into(),
                lesson: "Clip".into(),
                percent: 20.0,
                completed: 0,
                total: 1,
            };
            sink.on_progress(&progress);
            if *self.mode.lock().unwrap() == Mode::UntilCancelled {
                while !cancel.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            let mut report = DownloadReport::new("pluralsight", group);
            report.cancelled = cancel.is_cancelled();
            report.finish();
            sink.on_report(&report);
            Ok(report)
        }
    }

    #[async_trait]
    impl PluralsightPlatformApi for FakeApi {
        async fn courses(&self, root: &Path) -> Result<Vec<PluralsightCourseInfo>, PlatformError> {
            let course = CourseInfo {
                id: Some("course-1".into()),
                title: self.course_title.lock().unwrap().clone(),
                level: Some("Intermediate".into()),
                image_url: None,
                issue: None,
            };
            Ok(vec![PluralsightCourseInfo::from_course_at(&course, root)?])
        }
    }

    struct FakeFactory {
        api: Arc<FakeApi>,
        builds: Mutex<usize>,
    }

    impl PluralsightFactory for FakeFactory {
        fn build_api(
            &self,
            _credentials: &Credentials,
        ) -> Result<Arc<dyn PluralsightApi>, PluralsightError> {
            *self.builds.lock().unwrap() += 1;
            Ok(self.api.clone())
        }

        fn build_platform(&self, _api: Arc<dyn PluralsightApi>) -> Arc<dyn PluralsightPlatformApi> {
            self.api.clone()
        }
    }

    fn fixture() -> (PluralsightService, Arc<FakeFactory>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let factory = Arc::new(FakeFactory {
            api: Arc::new(FakeApi {
                validate_error: Mutex::new(None),
                mode: Mutex::new(Mode::Quick),
                validations: Mutex::default(),
                course_title: Mutex::new("Synthetic Course".into()),
            }),
            builds: Mutex::new(0),
        });
        let service = PluralsightService::new(
            SessionStore::for_platform(dir.path(), "pluralsight"),
            factory.clone(),
        );
        (service, factory, dir)
    }

    #[tokio::test]
    async fn webview_candidate_validation_does_not_persist_before_final_login() {
        let (service, factory, dir) = fixture();
        service.validate_webview_candidate(COOKIE).await.unwrap();
        assert_eq!(*factory.api.validations.lock().unwrap(), 1);
        assert!(!dir.path().join("pluralsight-session").exists());
    }

    #[tokio::test]
    async fn unauthorized_webview_candidate_is_rejected_without_persistence() {
        let (service, factory, dir) = fixture();
        *factory.api.validate_error.lock().unwrap() = Some(PluralsightError::Unauthorized(401));
        assert!(!service.validate_webview_candidate(COOKIE).await.unwrap());
        assert!(!dir.path().join("pluralsight-session").exists());
    }

    #[tokio::test]
    async fn valid_login_validates_before_persisting_normalized_cookies() {
        let (service, factory, dir) = fixture();
        let result = service
            .login_with_cookies(
                "Cookie: session=secret-cookie\nCookie: XSRF-TOKEN=secret-xsrf",
                Path::new(""),
            )
            .await;
        assert!(result.is_ok(), "valid credentials should authenticate");
        let json = serde_json::to_value(result.unwrap()).unwrap();
        assert_eq!(json["loggedIn"], true);
        assert!(json["courses"][0]["folder"]
            .as_str()
            .unwrap()
            .starts_with("Synthetic Course-"));
        assert_eq!(*factory.api.validations.lock().unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("pluralsight-session")).unwrap(),
            COOKIE
        );
    }

    #[tokio::test]
    async fn invalid_cookie_text_is_redacted_and_never_reaches_factory_or_disk() {
        let (service, factory, dir) = fixture();
        let error = service
            .login_with_cookies("session=TOPSECRET\nX-Evil: injected", Path::new(""))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidCookies);
        assert!(!error.message.contains("TOPSECRET"));
        assert_eq!(*factory.builds.lock().unwrap(), 0);
        assert!(!dir.path().join("pluralsight-session").exists());
    }

    #[tokio::test]
    async fn unauthorized_credentials_are_never_persisted_and_map_to_session_expired() {
        let (service, factory, dir) = fixture();
        *factory.api.validate_error.lock().unwrap() = Some(PluralsightError::Unauthorized(403));
        let error = service
            .login_with_cookies(COOKIE, Path::new(""))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::SessionExpired);
        assert!(!error.message.contains("secret-cookie"));
        assert!(!dir.path().join("pluralsight-session").exists());
    }

    #[tokio::test]
    async fn courses_require_a_session_and_return_typed_folder_metadata() {
        let (service, _, dir) = fixture();
        assert_eq!(
            service.courses(Path::new("")).await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
        SessionStore::for_platform(dir.path(), "pluralsight")
            .save(COOKIE)
            .unwrap();
        let courses = service.courses(Path::new("")).await.unwrap();
        assert_eq!(courses.len(), 1);
        assert_eq!(courses[0].id.as_deref(), Some("course-1"));
        assert!(courses[0].folder.starts_with("Synthetic Course-"));
        let json = serde_json::to_value(&courses[0]).unwrap();
        assert_eq!(json["imageUrl"], serde_json::Value::Null);
        assert!(!json.to_string().contains("secret-cookie"));
    }

    #[tokio::test]
    async fn login_and_refresh_return_folder_allocated_for_the_actual_unicode_library_root() {
        let (service, factory, session_dir) = fixture();
        let title = "A Pluralsight course title long enough to allocate a root-sensitive folder";
        *factory.api.course_title.lock().unwrap() = title.into();
        let library_root = session_dir
            .path()
            .join("界".repeat(28))
            .join("長い-library-root".repeat(7));
        let course = CourseInfo {
            id: Some("course-1".into()),
            title: title.into(),
            level: Some("Intermediate".into()),
            image_url: None,
            issue: None,
        };
        let expected_folder = lumen_pluralsight::planner::course_dir(&library_root, &course)
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let rootless_folder = lumen_pluralsight::planner::course_dir(Path::new(""), &course)
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_ne!(expected_folder, rootless_folder);

        let login = service
            .login_with_cookies(COOKIE, &library_root)
            .await
            .unwrap();
        let refreshed = service.courses(&library_root).await.unwrap();
        let local_course_folder = library_root
            .join("Pluralsight")
            .join(&login.courses[0].folder);
        assert_eq!(login.courses[0].folder, expected_folder);
        assert_eq!(refreshed[0].folder, expected_folder);
        assert_eq!(
            local_course_folder,
            lumen_pluralsight::planner::course_dir(&library_root, &course).unwrap()
        );
    }

    #[test]
    fn status_and_logout_use_platform_session_and_cancel_registry() {
        let (service, _, dir) = fixture();
        assert!(!service.status().unwrap().logged_in);
        SessionStore::for_platform(dir.path(), "pluralsight")
            .save(COOKIE)
            .unwrap();
        assert!(service.status().unwrap().logged_in);
        service.logout().unwrap();
        assert!(!service.status().unwrap().logged_in);
    }

    enum Event {
        Progress(ProgressEvent),
        Report(ReportEvent),
    }

    struct Channel(mpsc::UnboundedSender<Event>);
    impl DownloadEvents for Channel {
        fn progress(&self, event: ProgressEvent) {
            let _ = self.0.send(Event::Progress(event));
        }
        fn report(&self, event: ReportEvent) {
            let _ = self.0.send(Event::Report(event));
        }
    }

    #[tokio::test]
    async fn download_uses_shared_manager_emits_platform_id_and_cancels() {
        let (service, factory, session_dir) = fixture();
        *factory.api.mode.lock().unwrap() = Mode::UntilCancelled;
        service
            .login_with_cookies(COOKIE, Path::new(""))
            .await
            .unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let library = tempfile::tempdir().unwrap();
        let id = service
            .start_download(
                "course-1",
                library.path().to_path_buf(),
                Arc::new(Channel(tx)),
            )
            .unwrap();
        assert!(!id.is_empty());
        assert_eq!(service.running()[0].platform, "pluralsight");
        let dependency_dir = tempfile::tempdir().unwrap();
        let ytdlp = YtDlp::locate(None);
        let skool = crate::skool::SkoolService::new(
            SessionStore::for_platform(dependency_dir.path(), "skool"),
            Arc::new(crate::skool::DefaultFactory::new(ytdlp.clone())),
        );
        let hotmart = crate::hotmart::HotmartService::new(
            SessionStore::for_platform(dependency_dir.path(), "hotmart"),
            Arc::new(crate::hotmart::DefaultHotmartFactory::new(ytdlp)),
        );
        let youtube = crate::youtube::YoutubeService::new(Arc::new(lumen_youtube::Youtube::new(
            Arc::new(YtDlp::locate(None)),
            None,
        )));
        let goldenlife = crate::goldenlife::GoldenlifeService::new(
            SessionStore::for_platform(dependency_dir.path(), "goldenlife"),
            Arc::new(crate::goldenlife::DefaultGoldenlifeFactory::new(
                YtDlp::locate(None),
            )),
        );
        assert!(super::super::platform_busy(
            "pluralsight",
            &skool,
            &hotmart,
            &service,
            &youtube,
            &goldenlife
        )
        .unwrap());
        assert!(service.cancel(&id));
        let mut saw_progress = false;
        let report = loop {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                Event::Progress(event) => {
                    saw_progress = true;
                    assert_eq!(event.id, id);
                    assert_eq!(event.platform, "pluralsight");
                }
                Event::Report(event) => break event,
            }
        };
        assert!(saw_progress);
        assert_eq!(report.id, id);
        assert_eq!(report.report.platform, "pluralsight");
        assert!(report.report.cancelled);
        assert!(service.running().is_empty());
        assert!(!super::super::platform_busy(
            "pluralsight",
            &skool,
            &hotmart,
            &service,
            &youtube,
            &goldenlife
        )
        .unwrap());
        assert!(session_dir.path().join("pluralsight-session").exists());

        let (tx, mut rx) = mpsc::unbounded_channel();
        let next_id = service
            .start_download(
                "course-1",
                library.path().to_path_buf(),
                Arc::new(Channel(tx)),
            )
            .unwrap();
        assert!(!next_id.is_empty());
        service.logout().unwrap();
        let report = loop {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                Event::Progress(_) => {}
                Event::Report(event) => break event,
            }
        };
        assert!(report.report.cancelled);
        assert!(!session_dir.path().join("pluralsight-session").exists());
        assert!(!service.status().unwrap().logged_in);
    }

    #[test]
    fn cookie_jar_helper_keeps_all_pairs_without_debug_or_logging() {
        let header = cookie_header_from_jar(vec![
            ("session", "synthetic-session"),
            ("XSRF-TOKEN", "synthetic-xsrf"),
        ])
        .unwrap();
        let credentials = Credentials::new(&header).unwrap();
        assert_eq!(credentials.xsrf_token(), Some("synthetic-xsrf"));
        assert!(!format!("{credentials:?}").contains("synthetic-session"));
        assert_eq!(cookie_header_from_jar(vec![]), None);
    }
}
