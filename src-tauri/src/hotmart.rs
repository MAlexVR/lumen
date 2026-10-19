//! Hotmart commands' application layer. Same contract as Skool's, with the
//! `hmVlcIntegration` token (and optional cookies) as credentials. It holds no
//! Tauri types, so it is tested without a window.

use std::path::PathBuf;
use std::sync::Arc;

use lumen_core::PlatformError;
use lumen_download::{VideoRunner, YtDlp};
use lumen_hotmart::credentials::TOKEN_COOKIE;
use lumen_hotmart::{CourseInfo, Credentials, HotmartApi, HotmartClient, HotmartPlatform};
use serde::Serialize;

use crate::downloads::{
    CommandError, DownloadEvents, DownloadManager, ErrorCode, RunningDownload, SessionStatus,
};
use crate::session::SessionStore;

/// Token from the webview cookie jar, once a logged-in session exists.
pub fn token_from_jar<'a>(cookies: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<String> {
    cookies
        .into_iter()
        .find(|(name, value)| *name == TOKEN_COOKIE && !value.trim().is_empty())
        .map(|(_, value)| value.trim().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotmartLoginOutcome {
    pub logged_in: bool,
    pub courses: Vec<CourseInfo>,
}

/// Builds the Hotmart backend for some credentials (replaced by a fake in tests).
pub trait HotmartFactory: Send + Sync {
    fn build(&self, creds: &Credentials) -> Result<Arc<dyn HotmartApi>, PlatformError>;
}

/// Real backend: Hotmart over HTTP plus the yt-dlp sidecar.
pub struct DefaultHotmartFactory {
    videos: Arc<dyn VideoRunner>,
}

impl DefaultHotmartFactory {
    pub fn new(ytdlp: YtDlp) -> Self {
        Self {
            videos: Arc::new(ytdlp),
        }
    }
}

impl HotmartFactory for DefaultHotmartFactory {
    fn build(&self, creds: &Credentials) -> Result<Arc<dyn HotmartApi>, PlatformError> {
        let client = HotmartClient::new(creds.clone())?;
        Ok(Arc::new(HotmartPlatform::new(
            Arc::new(client),
            Arc::clone(&self.videos),
        )))
    }
}

pub struct HotmartService {
    session: SessionStore,
    factory: Arc<dyn HotmartFactory>,
    downloads: DownloadManager,
}

fn map(e: PlatformError) -> CommandError {
    CommandError::from_platform(e, ErrorCode::InvalidToken)
}

impl HotmartService {
    pub fn new(session: SessionStore, factory: Arc<dyn HotmartFactory>) -> Self {
        Self {
            session,
            factory,
            downloads: DownloadManager::new("hotmart"),
        }
    }

    fn credentials(&self) -> Result<Credentials, CommandError> {
        let stored = self
            .session
            .load()
            .map_err(|e| CommandError::internal("cannot read the stored session", e))?
            .ok_or_else(|| CommandError::new(ErrorCode::NotLoggedIn, "not logged in to Hotmart"))?;
        Credentials::from_stored(&stored).ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotLoggedIn,
                "the stored Hotmart session is unreadable: log in again",
            )
        })
    }

    /// Validates a pasted token (and optional cookies) against Hotmart and
    /// stores them.
    pub async fn login_with_credentials(
        &self,
        token: &str,
        cookies: Option<&str>,
    ) -> Result<HotmartLoginOutcome, CommandError> {
        let creds = Credentials::new(token, cookies)
            .map_err(|e| CommandError::new(ErrorCode::InvalidToken, e.to_string()))?;
        let courses = self
            .factory
            .build(&creds)
            .map_err(map)?
            .courses()
            .await
            .map_err(map)?;
        self.session
            .save(&creds.to_stored())
            .map_err(|e| CommandError::internal("cannot store the session", e))?;
        Ok(HotmartLoginOutcome {
            logged_in: true,
            courses,
        })
    }

    pub fn status(&self) -> Result<SessionStatus, CommandError> {
        match self.credentials() {
            Ok(_) => Ok(SessionStatus { logged_in: true }),
            Err(e) if e.code == ErrorCode::NotLoggedIn => Ok(SessionStatus { logged_in: false }),
            Err(e) => Err(e),
        }
    }

    /// Forgets the session and cancels running downloads.
    pub fn logout(&self) -> Result<(), CommandError> {
        self.downloads.cancel_all();
        self.session
            .clear()
            .map_err(|e| CommandError::internal("cannot remove the stored session", e))
    }

    pub async fn courses(&self) -> Result<Vec<CourseInfo>, CommandError> {
        let creds = self.credentials()?;
        self.factory
            .build(&creds)
            .map_err(map)?
            .courses()
            .await
            .map_err(map)
    }

    /// Starts a background download of a course (product id) and returns its
    /// id. Needs a Tokio runtime.
    pub fn start_download(
        &self,
        course_id: &str,
        library_root: PathBuf,
        events: Arc<dyn DownloadEvents>,
    ) -> Result<String, CommandError> {
        let creds = self.credentials()?;
        if library_root.as_os_str().is_empty() {
            return Err(CommandError::new(
                ErrorCode::NoLibrary,
                "choose a library folder first",
            ));
        }
        let platform = self.factory.build(&creds).map_err(map)?;
        self.downloads
            .start(course_id, platform, library_root, events)
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.downloads.cancel(id)
    }

    pub fn running(&self) -> Vec<RunningDownload> {
        self.downloads.running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::{ProgressEvent, ReportEvent};
    use async_trait::async_trait;
    use lumen_core::{
        CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadProgress,
        DownloadReport, IssueKind, PlatformGroup,
    };
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::mpsc;

    const TOKEN: &str = "SECRET.token-1";

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Quick,
        UntilCancelled,
        Fails,
    }

    struct FakeApi {
        courses: Result<Vec<CourseInfo>, PlatformError>,
        mode: Mode,
    }

    #[async_trait]
    impl CoursePlatform for FakeApi {
        fn id(&self) -> &'static str {
            "hotmart"
        }
        async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
            Ok(vec![])
        }
        async fn download_group(
            &self,
            course: &str,
            _root: &Path,
            _o: &DownloadOptions,
            sink: &dyn CourseProgressSink,
            cancel: &CancelFlag,
        ) -> Result<DownloadReport, PlatformError> {
            if self.mode == Mode::Fails {
                return Err(PlatformError::Network("boom".into()));
            }
            let progress = |percent: f32| DownloadProgress {
                group: course.into(),
                course: "Module".into(),
                lesson: "L".into(),
                percent,
                completed: 0,
                total: 1,
            };
            sink.on_progress(&progress(0.0));
            sink.on_progress(&progress(0.2));
            sink.on_progress(&progress(40.0));
            if self.mode == Mode::UntilCancelled {
                while !cancel.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            let mut report = DownloadReport::new("hotmart", course);
            report.cancelled = cancel.is_cancelled();
            report.finish();
            sink.on_report(&report);
            Ok(report)
        }
    }

    #[async_trait]
    impl HotmartApi for FakeApi {
        async fn courses(&self) -> Result<Vec<CourseInfo>, PlatformError> {
            self.courses.clone()
        }
    }

    struct FakeFactory {
        api: Arc<FakeApi>,
        seen: Mutex<Vec<(String, Option<String>)>>,
    }

    impl HotmartFactory for FakeFactory {
        fn build(&self, creds: &Credentials) -> Result<Arc<dyn HotmartApi>, PlatformError> {
            self.seen.lock().unwrap().push((
                creds.token().to_string(),
                creds.cookies().map(str::to_string),
            ));
            Ok(self.api.clone())
        }
    }

    enum Event {
        Progress(ProgressEvent),
        Report(ReportEvent),
    }

    struct Channel(mpsc::UnboundedSender<Event>);

    impl DownloadEvents for Channel {
        fn progress(&self, e: ProgressEvent) {
            let _ = self.0.send(Event::Progress(e));
        }
        fn report(&self, e: ReportEvent) {
            let _ = self.0.send(Event::Report(e));
        }
    }

    fn info(id: &str) -> CourseInfo {
        CourseInfo {
            id: id.into(),
            name: id.to_uppercase(),
            seller: Some("Seller".into()),
            cover_url: None,
            purchase_date: None,
            folder: format!("{} - Seller", id.to_uppercase()),
        }
    }

    struct Fixture {
        service: HotmartService,
        factory: Arc<FakeFactory>,
        dir: tempfile::TempDir,
    }

    fn fixture(courses: Result<Vec<CourseInfo>, PlatformError>, mode: Mode) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let factory = Arc::new(FakeFactory {
            api: Arc::new(FakeApi { courses, mode }),
            seen: Mutex::default(),
        });
        let service = HotmartService::new(
            SessionStore::for_platform(dir.path(), "hotmart"),
            factory.clone(),
        );
        Fixture {
            service,
            factory,
            dir,
        }
    }

    fn ok_fixture(mode: Mode) -> Fixture {
        fixture(Ok(vec![info("horizonte")]), mode)
    }

    async fn logged_in(mode: Mode) -> Fixture {
        let f = ok_fixture(mode);
        f.service.login_with_credentials(TOKEN, None).await.unwrap();
        f
    }

    async fn next_report(
        rx: &mut mpsc::UnboundedReceiver<Event>,
    ) -> (ReportEvent, Vec<ProgressEvent>) {
        let mut progress = Vec::new();
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("event in time")
                .expect("channel open");
            match ev {
                Event::Progress(p) => progress.push(p),
                Event::Report(r) => return (r, progress),
            }
        }
    }

    #[tokio::test]
    async fn a_valid_token_is_verified_then_persisted_privately() {
        let f = ok_fixture(Mode::Quick);
        let out = f
            .service
            .login_with_credentials("  Bearer SECRET.token-1\n", Some("Cookie: a=1"))
            .await
            .unwrap();
        assert!(out.logged_in);
        assert_eq!(out.courses[0].id, "horizonte");
        assert_eq!(
            *f.factory.seen.lock().unwrap(),
            [(TOKEN.to_string(), Some("a=1".to_string()))]
        );
        let stored = std::fs::read_to_string(f.dir.path().join("hotmart-session")).unwrap();
        assert_eq!(
            Credentials::from_stored(&stored),
            Some(Credentials::new(TOKEN, Some("a=1")).unwrap())
        );
        assert!(f.service.status().unwrap().logged_in);
    }

    #[tokio::test]
    async fn malformed_tokens_never_reach_hotmart_or_disk() {
        let f = ok_fixture(Mode::Quick);
        let err = f
            .service
            .login_with_credentials("SECRETVALUE WITH SPACES", None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidToken);
        assert!(!err.message.contains("SECRETVALUE"));
        assert!(f.factory.seen.lock().unwrap().is_empty());
        assert!(!f.dir.path().join("hotmart-session").exists());
        assert_eq!(
            f.service
                .login_with_credentials("", None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidToken
        );
    }

    #[tokio::test]
    async fn rejected_tokens_are_not_persisted() {
        let f = fixture(Err(PlatformError::SessionExpired), Mode::Quick);
        let err = f
            .service
            .login_with_credentials(TOKEN, None)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SessionExpired);
        assert!(!f.dir.path().join("hotmart-session").exists());
        assert!(!f.service.status().unwrap().logged_in);
    }

    #[tokio::test]
    async fn courses_require_a_session_and_report_expiry() {
        let f = ok_fixture(Mode::Quick);
        assert_eq!(
            f.service.courses().await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
        f.service.login_with_credentials(TOKEN, None).await.unwrap();
        assert_eq!(f.service.courses().await.unwrap()[0].id, "horizonte");

        let stale = fixture(Err(PlatformError::SessionExpired), Mode::Quick);
        let creds = Credentials::new(TOKEN, None).unwrap();
        std::fs::write(stale.dir.path().join("hotmart-session"), creds.to_stored()).unwrap();
        assert_eq!(
            stale.service.courses().await.unwrap_err().code,
            ErrorCode::SessionExpired
        );
        let down = fixture(Err(PlatformError::Network("x".into())), Mode::Quick);
        std::fs::write(down.dir.path().join("hotmart-session"), creds.to_stored()).unwrap();
        assert_eq!(
            down.service.courses().await.unwrap_err().code,
            ErrorCode::Network
        );
    }

    #[tokio::test]
    async fn an_unreadable_stored_session_means_not_logged_in() {
        let f = ok_fixture(Mode::Quick);
        std::fs::write(f.dir.path().join("hotmart-session"), "garbage").unwrap();
        assert!(!f.service.status().unwrap().logged_in);
        assert_eq!(
            f.service.courses().await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
    }

    #[tokio::test]
    async fn logout_forgets_the_session_and_is_idempotent() {
        let f = logged_in(Mode::Quick).await;
        f.service.logout().unwrap();
        f.service.logout().unwrap();
        assert!(!f.service.status().unwrap().logged_in);
        assert!(!f.dir.path().join("hotmart-session").exists());
    }

    #[tokio::test]
    async fn download_needs_a_session_and_a_library_folder() {
        let f = ok_fixture(Mode::Quick);
        let (tx, _rx) = mpsc::unbounded_channel();
        let events: Arc<dyn DownloadEvents> = Arc::new(Channel(tx));
        let e = f
            .service
            .start_download("horizonte", f.dir.path().into(), events.clone())
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NotLoggedIn);
        f.service.login_with_credentials(TOKEN, None).await.unwrap();
        let e = f
            .service
            .start_download("horizonte", PathBuf::new(), events)
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NoLibrary);
    }

    #[tokio::test]
    async fn download_emits_hotmart_tagged_throttled_progress_then_one_report() {
        let f = logged_in(Mode::Quick).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = f
            .service
            .start_download("horizonte", f.dir.path().into(), Arc::new(Channel(tx)))
            .unwrap();
        let (report, progress) = next_report(&mut rx).await;
        assert_eq!(report.id, id);
        assert_eq!(report.report.platform, "hotmart");
        assert!(report.report.complete);
        let percents: Vec<f32> = progress.iter().map(|p| p.progress.percent).collect();
        assert_eq!(percents, [0.0, 40.0], "0.2 is below the 1% throttle step");
        assert!(progress
            .iter()
            .all(|p| p.id == id && p.platform == "hotmart"));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !f.service.running().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_same_course_cannot_run_twice_and_cancel_ends_it() {
        let f = logged_in(Mode::UntilCancelled).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let events: Arc<dyn DownloadEvents> = Arc::new(Channel(tx));
        let id = f
            .service
            .start_download("horizonte", f.dir.path().into(), events.clone())
            .unwrap();
        assert_eq!(
            serde_json::to_value(f.service.running()).unwrap(),
            serde_json::json!([{"id": id, "platform": "hotmart", "group": "horizonte"}])
        );
        let dup = f
            .service
            .start_download("horizonte", f.dir.path().into(), events)
            .unwrap_err();
        assert_eq!(dup.code, ErrorCode::AlreadyRunning);
        assert!(!f.service.cancel("dl-unknown"));
        assert!(f.service.cancel(&id));
        let (report, _) = next_report(&mut rx).await;
        assert!(report.report.cancelled && !report.report.complete);
    }

    #[tokio::test]
    async fn a_fatal_platform_error_still_produces_a_failed_hotmart_report() {
        let f = logged_in(Mode::Fails).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        f.service
            .start_download("horizonte", f.dir.path().into(), Arc::new(Channel(tx)))
            .unwrap();
        let (report, _) = next_report(&mut rx).await;
        assert_eq!(report.report.platform, "hotmart");
        assert!(!report.report.complete);
        assert_eq!(report.report.issues[0].kind, IssueKind::Course);
        assert!(report.report.issues[0].message.contains("boom"));
    }

    #[tokio::test]
    async fn logout_cancels_running_downloads() {
        let f = logged_in(Mode::UntilCancelled).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        f.service
            .start_download("horizonte", f.dir.path().into(), Arc::new(Channel(tx)))
            .unwrap();
        f.service.logout().unwrap();
        let (report, _) = next_report(&mut rx).await;
        assert!(report.report.cancelled);
    }

    #[test]
    fn the_jar_yields_a_token_only_once_logged_in() {
        assert_eq!(
            token_from_jar([("aws-waf-token", "w"), ("hmSsoExp", "1")]),
            None
        );
        assert_eq!(token_from_jar([("hmVlcIntegration", "")]), None);
        assert_eq!(
            token_from_jar([("aws-waf-token", "w"), ("hmVlcIntegration", "tok.en")]).as_deref(),
            Some("tok.en")
        );
    }

    #[test]
    fn platform_errors_map_to_the_shared_codes() {
        let code = |e: PlatformError| map(e).code;
        assert_eq!(
            code(PlatformError::SessionExpired),
            ErrorCode::SessionExpired
        );
        assert_eq!(
            code(PlatformError::InvalidInput("x".into())),
            ErrorCode::InvalidToken
        );
        assert_eq!(code(PlatformError::Network("x".into())), ErrorCode::Network);
        assert_eq!(code(PlatformError::Parse("x".into())), ErrorCode::Internal);
        assert_eq!(code(PlatformError::Io("x".into())), ErrorCode::Internal);
    }

    #[test]
    fn outcome_and_error_codes_have_the_documented_json_shape() {
        let out = HotmartLoginOutcome {
            logged_in: true,
            courses: vec![info("horizonte")],
        };
        assert_eq!(
            serde_json::to_value(&out).unwrap(),
            serde_json::json!({"loggedIn": true, "courses": [{
                "id":"horizonte","name":"HORIZONTE","seller":"Seller","coverUrl":null,
                "purchaseDate":null,"folder":"HORIZONTE - Seller"}]})
        );
        let e = CommandError::new(ErrorCode::InvalidToken, "bad");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"code":"invalid_token","message":"bad"})
        );
    }
}
