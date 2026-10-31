//! GoldenLife session and download service. This module has no Tauri types.
//!
//! Login is an email + password form. The password only lives for the login
//! call; what is stored is the cookie header the academy answered with (it
//! includes a long-lived "remember me" cookie).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use lumen_core::PlatformError;
use lumen_download::{VideoRunner, YtDlp};
use lumen_goldenlife::{
    ClientConfig, GoldenlifeApi, GoldenlifeClient, GoldenlifeCourseInfo, GoldenlifeError,
    GoldenlifePlatform, GoldenlifePlatformApi, Login, Session,
};
use serde::Serialize;

use crate::downloads::{
    CommandError, DownloadEvents, DownloadManager, ErrorCode, RunningDownload, SessionStatus,
};
use crate::session::SessionStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenlifeLoginOutcome {
    pub logged_in: bool,
    pub courses: Vec<GoldenlifeCourseInfo>,
}

/// Builds authenticated API and CoursePlatform adapters; tests supply fakes.
#[async_trait]
pub trait GoldenlifeFactory: Send + Sync {
    /// Signs in and returns the API plus the session to persist.
    async fn login(
        &self,
        login: &Login,
    ) -> Result<(Arc<dyn GoldenlifeApi>, Session), GoldenlifeError>;
    fn build_api(&self, session: &Session) -> Result<Arc<dyn GoldenlifeApi>, GoldenlifeError>;
    fn build_platform(&self, api: Arc<dyn GoldenlifeApi>) -> Arc<dyn GoldenlifePlatformApi>;
}

pub struct DefaultGoldenlifeFactory {
    videos: Arc<dyn VideoRunner>,
}

impl DefaultGoldenlifeFactory {
    pub fn new(ytdlp: YtDlp) -> Self {
        Self {
            videos: Arc::new(ytdlp),
        }
    }
}

#[async_trait]
impl GoldenlifeFactory for DefaultGoldenlifeFactory {
    async fn login(
        &self,
        login: &Login,
    ) -> Result<(Arc<dyn GoldenlifeApi>, Session), GoldenlifeError> {
        let client = GoldenlifeClient::login(&ClientConfig::default(), login).await?;
        let session = client.session()?;
        Ok((Arc::new(client), session))
    }

    fn build_api(&self, session: &Session) -> Result<Arc<dyn GoldenlifeApi>, GoldenlifeError> {
        Ok(Arc::new(GoldenlifeClient::new(
            session,
            &ClientConfig::default(),
        )?))
    }

    fn build_platform(&self, api: Arc<dyn GoldenlifeApi>) -> Arc<dyn GoldenlifePlatformApi> {
        Arc::new(GoldenlifePlatform::new(api, Arc::clone(&self.videos)))
    }
}

pub struct GoldenlifeService {
    session: SessionStore,
    factory: Arc<dyn GoldenlifeFactory>,
    downloads: DownloadManager,
}

fn map(error: GoldenlifeError) -> CommandError {
    match error {
        GoldenlifeError::InvalidCredentials => CommandError::new(
            ErrorCode::InvalidCredentials,
            "the email or password is not right",
        ),
        other => CommandError::from_platform(PlatformError::from(other), ErrorCode::InvalidCookies),
    }
}

impl GoldenlifeService {
    pub fn new(session: SessionStore, factory: Arc<dyn GoldenlifeFactory>) -> Self {
        Self {
            session,
            factory,
            downloads: DownloadManager::new("goldenlife"),
        }
    }

    fn session(&self) -> Result<Session, CommandError> {
        let stored = self
            .session
            .load()
            .map_err(|error| CommandError::internal("cannot read the stored session", error))?
            .ok_or_else(|| {
                CommandError::new(ErrorCode::NotLoggedIn, "not logged in to Golden Life")
            })?;
        Session::new(&stored).map_err(|_| {
            CommandError::new(
                ErrorCode::NotLoggedIn,
                "the stored Golden Life session is unreadable: log in again",
            )
        })
    }

    /// Signs in, checks the session works, lists the courses and only then
    /// persists the cookies. The password is never stored.
    pub async fn login_with_credentials(
        &self,
        email: &str,
        password: &str,
        library_root: &Path,
    ) -> Result<GoldenlifeLoginOutcome, CommandError> {
        let login = Login::new(email, password).map_err(map)?;
        let (api, session) = self.factory.login(&login).await.map_err(map)?;
        drop(login);
        api.validate().await.map_err(map)?;
        let courses = self
            .factory
            .build_platform(api)
            .courses(library_root)
            .await
            .map_err(|error| CommandError::from_platform(error, ErrorCode::InvalidCookies))?;
        self.session
            .save(session.cookie_header())
            .map_err(|error| CommandError::internal("cannot store the session", error))?;
        Ok(GoldenlifeLoginOutcome {
            logged_in: true,
            courses,
        })
    }

    pub fn status(&self) -> Result<SessionStatus, CommandError> {
        match self.session() {
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
    ) -> Result<Vec<GoldenlifeCourseInfo>, CommandError> {
        let session = self.session()?;
        let api = self.factory.build_api(&session).map_err(map)?;
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
        let session = self.session()?;
        if library_root.as_os_str().is_empty() {
            return Err(CommandError::new(
                ErrorCode::NoLibrary,
                "choose a library folder first",
            ));
        }
        crate::downloads::ensure_library_ready(&library_root)?;
        let api = self.factory.build_api(&session).map_err(map)?;
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

    pub fn set_paused(&self, id: &str, paused: bool) -> bool {
        self.downloads.set_paused(id, paused)
    }

    pub fn running(&self) -> Vec<RunningDownload> {
        self.downloads.running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::{ProgressEvent, ReportEvent};
    use lumen_core::{
        CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadProgress,
        DownloadReport, PlatformGroup,
    };
    use lumen_goldenlife::{CourseInfo, LessonPage, Outline};
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    const PASSWORD: &str = "hunter2-secret";

    struct FakeApi {
        wrong_password: bool,
        expired: Mutex<bool>,
        logins: Mutex<usize>,
    }

    fn course() -> CourseInfo {
        CourseInfo {
            id: "574182".into(),
            title: "Meta Ads Basico".into(),
            shelf: Some("ADS".into()),
            image_url: None,
        }
    }

    #[async_trait]
    impl GoldenlifeApi for FakeApi {
        async fn validate(&self) -> Result<(), GoldenlifeError> {
            if *self.expired.lock().unwrap() {
                Err(GoldenlifeError::Unauthorized)
            } else {
                Ok(())
            }
        }
        async fn courses(&self) -> Result<Vec<CourseInfo>, GoldenlifeError> {
            Ok(vec![course()])
        }
        async fn outline(&self, _: &str) -> Result<Outline, GoldenlifeError> {
            Ok(Outline::default())
        }
        async fn lesson(&self, _: &str) -> Result<LessonPage, GoldenlifeError> {
            Ok(LessonPage::default())
        }
        async fn download_attachment(&self, _: &str, _: &Path) -> Result<(), GoldenlifeError> {
            Ok(())
        }
        fn referer(&self) -> String {
            "https://academy.example.test/".into()
        }
    }

    #[async_trait]
    impl CoursePlatform for FakeApi {
        fn id(&self) -> &'static str {
            "goldenlife"
        }
        async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
            Ok(Vec::new())
        }
        async fn download_group(
            &self,
            group: &str,
            _: &Path,
            _: &DownloadOptions,
            sink: &dyn CourseProgressSink,
            cancel: &CancelFlag,
        ) -> Result<DownloadReport, PlatformError> {
            sink.on_progress(&DownloadProgress {
                group: group.into(),
                course: "Modulo".into(),
                lesson: "Clase".into(),
                percent: 20.0,
                completed: 0,
                total: 1,
            });
            while !cancel.is_cancelled() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let mut report = DownloadReport::new("goldenlife", group);
            report.cancelled = true;
            report.finish();
            sink.on_report(&report);
            Ok(report)
        }
    }

    #[async_trait]
    impl GoldenlifePlatformApi for FakeApi {
        async fn courses(&self, root: &Path) -> Result<Vec<GoldenlifeCourseInfo>, PlatformError> {
            Ok(vec![GoldenlifeCourseInfo::from_course_at(&course(), root)?])
        }
    }

    struct FakeFactory {
        api: Arc<FakeApi>,
    }

    #[async_trait]
    impl GoldenlifeFactory for FakeFactory {
        async fn login(
            &self,
            login: &Login,
        ) -> Result<(Arc<dyn GoldenlifeApi>, Session), GoldenlifeError> {
            *self.api.logins.lock().unwrap() += 1;
            if self.api.wrong_password || login.password() != PASSWORD {
                return Err(GoldenlifeError::InvalidCredentials);
            }
            Ok((
                self.api.clone(),
                Session::new("app_session=s2; remember_cademi_x=long").unwrap(),
            ))
        }
        fn build_api(&self, _: &Session) -> Result<Arc<dyn GoldenlifeApi>, GoldenlifeError> {
            Ok(self.api.clone())
        }
        fn build_platform(&self, _: Arc<dyn GoldenlifeApi>) -> Arc<dyn GoldenlifePlatformApi> {
            self.api.clone()
        }
    }

    fn fixture() -> (GoldenlifeService, Arc<FakeFactory>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let factory = Arc::new(FakeFactory {
            api: Arc::new(FakeApi {
                wrong_password: false,
                expired: Mutex::new(false),
                logins: Mutex::new(0),
            }),
        });
        let service = GoldenlifeService::new(
            SessionStore::for_platform(dir.path(), "goldenlife"),
            factory.clone(),
        );
        (service, factory, dir)
    }

    #[tokio::test]
    async fn login_stores_the_cookies_and_never_the_password() {
        let (service, _, dir) = fixture();
        let outcome = service
            .login_with_credentials("ana@example.com", PASSWORD, Path::new(""))
            .await
            .unwrap();
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["loggedIn"], true);
        assert_eq!(json["courses"][0]["id"], "574182");
        assert_eq!(json["courses"][0]["shelf"], "ADS");
        assert_eq!(json["courses"][0]["folder"], "Meta Ads Basico");
        let stored = std::fs::read_to_string(dir.path().join("goldenlife-session")).unwrap();
        assert_eq!(stored, "app_session=s2; remember_cademi_x=long");
        assert!(!stored.contains(PASSWORD) && !stored.contains("ana@"));
        assert!(service.status().unwrap().logged_in);
    }

    #[tokio::test]
    async fn a_wrong_password_is_an_invalid_credentials_error_and_nothing_is_stored() {
        let (service, _, dir) = fixture();
        let error = service
            .login_with_credentials("ana@example.com", "not-the-password", Path::new(""))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidCredentials);
        assert!(!error.message.contains("not-the-password"));
        assert!(!dir.path().join("goldenlife-session").exists());
        assert!(!service.status().unwrap().logged_in);
    }

    #[tokio::test]
    async fn unusable_input_never_reaches_the_network() {
        let (service, factory, dir) = fixture();
        for (email, password) in [
            ("", PASSWORD),
            ("not-an-email", PASSWORD),
            ("ana@example.com", ""),
        ] {
            let error = service
                .login_with_credentials(email, password, Path::new(""))
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidCredentials);
            assert!(!error.message.contains(PASSWORD));
        }
        assert_eq!(*factory.api.logins.lock().unwrap(), 0);
        assert!(!dir.path().join("goldenlife-session").exists());
    }

    #[tokio::test]
    async fn a_session_that_does_not_validate_is_not_stored() {
        let (service, factory, dir) = fixture();
        *factory.api.expired.lock().unwrap() = true;
        let error = service
            .login_with_credentials("ana@example.com", PASSWORD, Path::new(""))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::SessionExpired);
        assert!(!dir.path().join("goldenlife-session").exists());
    }

    #[tokio::test]
    async fn courses_need_a_session() {
        let (service, _, dir) = fixture();
        assert_eq!(
            service.courses(Path::new("")).await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
        SessionStore::for_platform(dir.path(), "goldenlife")
            .save("app_session=abc")
            .unwrap();
        let courses = service.courses(Path::new("")).await.unwrap();
        assert_eq!(courses[0].title, "Meta Ads Basico");
    }

    #[test]
    fn status_and_logout_use_the_platform_session() {
        let (service, _, dir) = fixture();
        assert!(!service.status().unwrap().logged_in);
        let store = SessionStore::for_platform(dir.path(), "goldenlife");
        store.save("app_session=abc").unwrap();
        assert!(service.status().unwrap().logged_in);
        store.save("garbage without equals").unwrap();
        assert!(
            !service.status().unwrap().logged_in,
            "an unreadable session is logged out"
        );
        service.logout().unwrap();
        assert!(!dir.path().join("goldenlife-session").exists());
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
    async fn downloads_carry_the_platform_id_and_can_be_cancelled() {
        let (service, _, _dir) = fixture();
        let library = tempfile::tempdir().unwrap();
        assert_eq!(
            service
                .start_download(
                    "574182",
                    library.path().to_path_buf(),
                    Arc::new(Channel(mpsc::unbounded_channel().0))
                )
                .unwrap_err()
                .code,
            ErrorCode::NotLoggedIn
        );
        service
            .login_with_credentials("ana@example.com", PASSWORD, Path::new(""))
            .await
            .unwrap();
        assert_eq!(
            service
                .start_download(
                    "574182",
                    PathBuf::new(),
                    Arc::new(Channel(mpsc::unbounded_channel().0))
                )
                .unwrap_err()
                .code,
            ErrorCode::NoLibrary
        );

        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = service
            .start_download(
                "574182",
                library.path().to_path_buf(),
                Arc::new(Channel(tx)),
            )
            .unwrap();
        assert_eq!(service.running()[0].platform, "goldenlife");
        match rx.recv().await.unwrap() {
            Event::Progress(p) => assert_eq!(
                (p.id.as_str(), p.platform.as_str()),
                (id.as_str(), "goldenlife")
            ),
            Event::Report(_) => panic!("progress comes first"),
        }
        assert!(service.cancel(&id));
        match rx.recv().await.unwrap() {
            Event::Report(r) => {
                assert_eq!(r.report.platform, "goldenlife");
                assert!(r.report.cancelled);
            }
            Event::Progress(_) => panic!("the report ends the run"),
        }
    }
}
