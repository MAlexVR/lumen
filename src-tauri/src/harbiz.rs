//! Harbiz session and download service. This module has no Tauri types.
//!
//! Login is an email + password over Meteor DDP. The password only lives for the
//! login call; what is stored is the resume token the server answered with.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use lumen_core::PlatformError;
use lumen_download::{VideoRunner, YtDlp};
use lumen_harbiz::{
    login, HarbizClient, HarbizCourseInfo, HarbizError, HarbizPlatform, HarbizPlatformApi, Login,
    Session, WsConnector,
};
use serde::Serialize;

use crate::downloads::{
    CommandError, DownloadEvents, DownloadManager, ErrorCode, RunningDownload, SessionStatus,
};
use crate::session::SessionStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarbizLoginOutcome {
    pub logged_in: bool,
    pub courses: Vec<HarbizCourseInfo>,
}

/// Builds the login and the authenticated platform adapter; tests supply fakes.
#[async_trait]
pub trait HarbizFactory: Send + Sync {
    /// Signs in and returns the session to persist.
    async fn login(&self, login: &Login) -> Result<Session, HarbizError>;
    fn build_platform(&self, session: &Session) -> Arc<dyn HarbizPlatformApi>;
}

pub struct DefaultHarbizFactory {
    videos: Arc<dyn VideoRunner>,
    connector: Arc<WsConnector>,
}

impl DefaultHarbizFactory {
    pub fn new(ytdlp: YtDlp) -> Self {
        Self {
            videos: Arc::new(ytdlp),
            connector: Arc::new(WsConnector::default()),
        }
    }
}

#[async_trait]
impl HarbizFactory for DefaultHarbizFactory {
    async fn login(&self, input: &Login) -> Result<Session, HarbizError> {
        login(self.connector.as_ref(), input).await
    }

    fn build_platform(&self, session: &Session) -> Arc<dyn HarbizPlatformApi> {
        let client = HarbizClient::new(self.connector.clone(), session.clone());
        Arc::new(HarbizPlatform::new(
            Arc::new(client),
            Arc::clone(&self.videos),
        ))
    }
}

pub struct HarbizService {
    session: SessionStore,
    factory: Arc<dyn HarbizFactory>,
    downloads: DownloadManager,
}

fn map(error: HarbizError) -> CommandError {
    match error {
        HarbizError::InvalidCredentials => CommandError::new(
            ErrorCode::InvalidCredentials,
            "the email or password is not right",
        ),
        other => CommandError::from_platform(PlatformError::from(other), ErrorCode::InvalidCookies),
    }
}

impl HarbizService {
    pub fn new(session: SessionStore, factory: Arc<dyn HarbizFactory>) -> Self {
        Self {
            session,
            factory,
            downloads: DownloadManager::new("harbiz"),
        }
    }

    fn session(&self) -> Result<Session, CommandError> {
        let stored = self
            .session
            .load()
            .map_err(|error| CommandError::internal("cannot read the stored session", error))?
            .ok_or_else(|| CommandError::new(ErrorCode::NotLoggedIn, "not logged in to Harbiz"))?;
        Session::new(&stored).map_err(|_| {
            CommandError::new(
                ErrorCode::NotLoggedIn,
                "the stored Harbiz session is unreadable: log in again",
            )
        })
    }

    /// Signs in, proves the session can read the agenda and only then persists
    /// the resume token. The password is never stored.
    pub async fn login_with_credentials(
        &self,
        email: &str,
        password: &str,
        library_root: &Path,
    ) -> Result<HarbizLoginOutcome, CommandError> {
        let input = Login::new(email, password).map_err(map)?;
        let session = self.factory.login(&input).await.map_err(map)?;
        drop(input);
        let courses = self
            .factory
            .build_platform(&session)
            .courses(library_root)
            .await
            .map_err(|error| CommandError::from_platform(error, ErrorCode::InvalidCookies))?;
        self.session
            .save(session.token())
            .map_err(|error| CommandError::internal("cannot store the session", error))?;
        Ok(HarbizLoginOutcome {
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
    ) -> Result<Vec<HarbizCourseInfo>, CommandError> {
        let session = self.session()?;
        self.factory
            .build_platform(&session)
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
        self.downloads.start(
            course_id,
            self.factory.build_platform(&session),
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
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    const PASSWORD: &str = "hunter2-secret";

    struct FakePlatform {
        expired: Mutex<bool>,
        logins: Mutex<usize>,
    }

    fn info(root: &Path) -> HarbizCourseInfo {
        HarbizCourseInfo {
            id: "harbiz".into(),
            title: "Harbiz Training".into(),
            folder: root.join("x").to_string_lossy().into_owned(),
            workouts: 3,
            downloaded: 1,
            pending: 2,
            first_date: Some("2026-06-02".into()),
            last_date: Some("2026-10-31".into()),
            photo_checkins: 5,
        }
    }

    #[async_trait]
    impl CoursePlatform for FakePlatform {
        fn id(&self) -> &'static str {
            "harbiz"
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
                course: "Harbiz Training".into(),
                lesson: "2026-06-02 - Legs".into(),
                percent: 20.0,
                completed: 0,
                total: 1,
            });
            while !cancel.is_cancelled() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let mut report = DownloadReport::new("harbiz", group);
            report.cancelled = true;
            report.finish();
            sink.on_report(&report);
            Ok(report)
        }
    }

    #[async_trait]
    impl HarbizPlatformApi for FakePlatform {
        async fn courses(&self, root: &Path) -> Result<Vec<HarbizCourseInfo>, PlatformError> {
            if *self.expired.lock().unwrap() {
                return Err(PlatformError::SessionExpired);
            }
            Ok(vec![info(root)])
        }
    }

    struct FakeFactory {
        platform: Arc<FakePlatform>,
    }

    #[async_trait]
    impl HarbizFactory for FakeFactory {
        async fn login(&self, login: &Login) -> Result<Session, HarbizError> {
            *self.platform.logins.lock().unwrap() += 1;
            if login.password_digest() != Login::new("a@b.co", PASSWORD).unwrap().password_digest()
            {
                return Err(HarbizError::InvalidCredentials);
            }
            Session::new("resume-token-123")
        }
        fn build_platform(&self, _: &Session) -> Arc<dyn HarbizPlatformApi> {
            self.platform.clone()
        }
    }

    fn fixture() -> (HarbizService, Arc<FakeFactory>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let factory = Arc::new(FakeFactory {
            platform: Arc::new(FakePlatform {
                expired: Mutex::new(false),
                logins: Mutex::new(0),
            }),
        });
        let service = HarbizService::new(
            SessionStore::for_platform(dir.path(), "harbiz"),
            factory.clone(),
        );
        (service, factory, dir)
    }

    #[tokio::test]
    async fn login_stores_the_resume_token_and_never_the_password() {
        let (service, _, dir) = fixture();
        let outcome = service
            .login_with_credentials("ana@example.com", PASSWORD, Path::new(""))
            .await
            .unwrap();
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["loggedIn"], true);
        assert_eq!(json["courses"][0]["id"], "harbiz");
        assert_eq!(json["courses"][0]["pending"], 2);
        assert_eq!(json["courses"][0]["lastDate"], "2026-10-31");
        let stored = std::fs::read_to_string(dir.path().join("harbiz-session")).unwrap();
        assert_eq!(stored, "resume-token-123");
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
        assert!(!dir.path().join("harbiz-session").exists());
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
        assert_eq!(*factory.platform.logins.lock().unwrap(), 0);
        assert!(!dir.path().join("harbiz-session").exists());
    }

    #[tokio::test]
    async fn a_session_that_cannot_read_the_agenda_is_not_stored() {
        let (service, factory, dir) = fixture();
        *factory.platform.expired.lock().unwrap() = true;
        let error = service
            .login_with_credentials("ana@example.com", PASSWORD, Path::new(""))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::SessionExpired);
        assert!(!dir.path().join("harbiz-session").exists());
    }

    #[tokio::test]
    async fn courses_need_a_session() {
        let (service, _, dir) = fixture();
        assert_eq!(
            service.courses(Path::new("")).await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
        SessionStore::for_platform(dir.path(), "harbiz")
            .save("resume-token-123")
            .unwrap();
        let courses = service.courses(Path::new("")).await.unwrap();
        assert_eq!(courses[0].title, "Harbiz Training");
    }

    #[test]
    fn status_and_logout_use_the_platform_session() {
        let (service, _, dir) = fixture();
        assert!(!service.status().unwrap().logged_in);
        let store = SessionStore::for_platform(dir.path(), "harbiz");
        store.save("resume-token-123").unwrap();
        assert!(service.status().unwrap().logged_in);
        store.save("garbage with spaces").unwrap();
        assert!(
            !service.status().unwrap().logged_in,
            "an unreadable session is logged out"
        );
        service.logout().unwrap();
        assert!(!dir.path().join("harbiz-session").exists());
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
        let idle = || Arc::new(Channel(mpsc::unbounded_channel().0));
        assert_eq!(
            service
                .start_download("harbiz", library.path().to_path_buf(), idle())
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
                .start_download("harbiz", PathBuf::new(), idle())
                .unwrap_err()
                .code,
            ErrorCode::NoLibrary
        );

        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = service
            .start_download(
                "harbiz",
                library.path().to_path_buf(),
                Arc::new(Channel(tx)),
            )
            .unwrap();
        assert_eq!(service.running()[0].platform, "harbiz");
        match rx.recv().await.unwrap() {
            Event::Progress(p) => assert_eq!(
                (p.id.as_str(), p.platform.as_str()),
                (id.as_str(), "harbiz")
            ),
            Event::Report(_) => panic!("progress comes first"),
        }
        assert!(!service.running()[0].paused);
        assert!(service.set_paused(&id, true));
        assert!(service.running()[0].paused);
        assert!(!service.set_paused("dl-missing", true));
        assert!(service.cancel(&id));
        match rx.recv().await.unwrap() {
            Event::Report(r) => {
                assert_eq!(r.report.platform, "harbiz");
                assert!(r.report.cancelled);
            }
            Event::Progress(_) => panic!("the report ends the run"),
        }
    }
}
