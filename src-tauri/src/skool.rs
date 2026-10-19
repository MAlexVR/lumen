//! Skool commands' application layer. It holds no Tauri types, so it is
//! tested without a window; `lib.rs` only adapts it to commands and events.

use std::path::PathBuf;
use std::sync::Arc;

use lumen_core::{CoursePlatform, PlatformError, PlatformGroup};
use lumen_download::{VideoRunner, YtDlp};
use lumen_skool::session::{cookie_header_from_pairs, has_auth_token, normalize_cookie_header};
use lumen_skool::{SkoolClient, SkoolPlatform};
use serde::Serialize;

use crate::downloads::{
    CommandError, DownloadEvents, DownloadManager, ErrorCode, RunningDownload, SessionStatus,
};
use crate::session::SessionStore;

pub type SkoolCommandError = CommandError;

/// Header built from the webview cookie jar, once a logged-in session exists.
pub fn header_from_jar<'a>(
    cookies: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    let header = cookie_header_from_pairs(cookies);
    has_auth_token(&header).then_some(header)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginOutcome {
    pub logged_in: bool,
    pub groups: Vec<PlatformGroup>,
}

/// Builds a platform for a cookie header (replaced by a fake in tests).
pub trait PlatformFactory: Send + Sync {
    fn build(&self, cookie_header: &str) -> Result<Arc<dyn CoursePlatform>, PlatformError>;
}

/// Real platform: Skool over HTTP plus the yt-dlp sidecar.
pub struct DefaultFactory {
    videos: Arc<dyn VideoRunner>,
}

impl DefaultFactory {
    pub fn new(ytdlp: YtDlp) -> Self {
        Self {
            videos: Arc::new(ytdlp),
        }
    }
}

impl PlatformFactory for DefaultFactory {
    fn build(&self, cookie_header: &str) -> Result<Arc<dyn CoursePlatform>, PlatformError> {
        let client = SkoolClient::new(cookie_header)?;
        Ok(Arc::new(SkoolPlatform::new(
            Arc::new(client),
            Arc::clone(&self.videos),
        )))
    }
}

pub struct SkoolService {
    session: SessionStore,
    factory: Arc<dyn PlatformFactory>,
    downloads: DownloadManager,
}

impl SkoolService {
    pub fn new(session: SessionStore, factory: Arc<dyn PlatformFactory>) -> Self {
        Self {
            session,
            factory,
            downloads: DownloadManager::new("skool"),
        }
    }

    fn cookie(&self) -> Result<String, SkoolCommandError> {
        self.session
            .load()
            .map_err(|e| SkoolCommandError::internal("cannot read the stored session", e))?
            .filter(|c| has_auth_token(c))
            .ok_or_else(|| SkoolCommandError::new(ErrorCode::NotLoggedIn, "not logged in to Skool"))
    }

    /// Validates a pasted cookie header against Skool and stores it.
    pub async fn login_with_cookies(&self, text: &str) -> Result<LoginOutcome, SkoolCommandError> {
        let header = normalize_cookie_header(text)
            .map_err(|e| SkoolCommandError::new(ErrorCode::InvalidCookies, e.to_string()))?;
        let groups = self.factory.build(&header)?.groups().await?;
        self.session
            .save(&header)
            .map_err(|e| SkoolCommandError::internal("cannot store the session", e))?;
        Ok(LoginOutcome {
            logged_in: true,
            groups,
        })
    }

    pub fn status(&self) -> Result<SessionStatus, SkoolCommandError> {
        match self.cookie() {
            Ok(_) => Ok(SessionStatus { logged_in: true }),
            Err(e) if e.code == ErrorCode::NotLoggedIn => Ok(SessionStatus { logged_in: false }),
            Err(e) => Err(e),
        }
    }

    /// Forgets the session and cancels running downloads.
    pub fn logout(&self) -> Result<(), SkoolCommandError> {
        self.downloads.cancel_all();
        self.session
            .clear()
            .map_err(|e| SkoolCommandError::internal("cannot remove the stored session", e))
    }

    pub async fn groups(&self) -> Result<Vec<PlatformGroup>, SkoolCommandError> {
        let cookie = self.cookie()?;
        Ok(self.factory.build(&cookie)?.groups().await?)
    }

    /// Starts a background download and returns its id. Needs a Tokio runtime.
    pub fn start_download(
        &self,
        group_slug: &str,
        library_root: PathBuf,
        events: Arc<dyn DownloadEvents>,
    ) -> Result<String, SkoolCommandError> {
        let cookie = self.cookie()?;
        if library_root.as_os_str().is_empty() {
            return Err(SkoolCommandError::new(
                ErrorCode::NoLibrary,
                "choose a library folder first",
            ));
        }
        let platform = self.factory.build(&cookie)?;
        self.downloads
            .start(group_slug, platform, library_root, events)
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.downloads.cancel(id)
    }

    /// Downloads in progress, so a reloaded UI can pick them up again.
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
        CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Issue,
        IssueKind,
    };
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::mpsc;

    const COOKIE: &str = "auth_token=SECRET; aws-waf-token=w";

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Quick,
        UntilCancelled,
        Fails,
    }

    struct FakePlatform {
        groups: Result<Vec<PlatformGroup>, PlatformError>,
        mode: Mode,
    }

    #[async_trait]
    impl CoursePlatform for FakePlatform {
        fn id(&self) -> &'static str {
            "skool"
        }
        async fn groups(&self) -> Result<Vec<PlatformGroup>, PlatformError> {
            self.groups.clone()
        }
        async fn download_group(
            &self,
            group: &str,
            _root: &Path,
            _o: &DownloadOptions,
            sink: &dyn CourseProgressSink,
            cancel: &CancelFlag,
        ) -> Result<DownloadReport, PlatformError> {
            if self.mode == Mode::Fails {
                return Err(PlatformError::Network("boom".into()));
            }
            let progress = |percent: f32, completed: u32| DownloadProgress {
                group: group.into(),
                course: "C".into(),
                lesson: "L".into(),
                percent,
                completed,
                total: 1,
            };
            sink.on_progress(&progress(0.0, 0));
            sink.on_progress(&progress(0.2, 0));
            sink.on_progress(&progress(40.0, 0));
            if self.mode == Mode::UntilCancelled {
                while !cancel.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            let mut report = DownloadReport::new("skool", group);
            report.cancelled = cancel.is_cancelled();
            report.finish();
            sink.on_report(&report);
            Ok(report)
        }
    }

    struct FakeFactory {
        platform: Arc<FakePlatform>,
        cookies: Mutex<Vec<String>>,
    }

    impl PlatformFactory for FakeFactory {
        fn build(&self, header: &str) -> Result<Arc<dyn CoursePlatform>, PlatformError> {
            self.cookies.lock().unwrap().push(header.to_string());
            Ok(self.platform.clone())
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

    fn group(slug: &str) -> PlatformGroup {
        PlatformGroup {
            slug: slug.into(),
            name: slug.to_uppercase(),
            cover_url: None,
            num_courses: 2,
        }
    }

    struct Fixture {
        service: SkoolService,
        factory: Arc<FakeFactory>,
        dir: tempfile::TempDir,
    }

    fn fixture(groups: Result<Vec<PlatformGroup>, PlatformError>, mode: Mode) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let factory = Arc::new(FakeFactory {
            platform: Arc::new(FakePlatform { groups, mode }),
            cookies: Mutex::default(),
        });
        let service = SkoolService::new(
            SessionStore::new(dir.path().join("skool-session")),
            factory.clone(),
        );
        Fixture {
            service,
            factory,
            dir,
        }
    }

    fn ok_fixture(mode: Mode) -> Fixture {
        fixture(Ok(vec![group("growth")]), mode)
    }

    async fn logged_in(mode: Mode) -> Fixture {
        let f = ok_fixture(mode);
        f.service.login_with_cookies(COOKIE).await.unwrap();
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
    async fn valid_cookies_are_verified_then_persisted_privately() {
        let f = ok_fixture(Mode::Quick);
        let out = f
            .service
            .login_with_cookies("Cookie: auth_token=SECRET;  aws-waf-token=w\n")
            .await
            .unwrap();
        assert!(out.logged_in);
        assert_eq!(out.groups[0].slug, "growth");
        assert_eq!(*f.factory.cookies.lock().unwrap(), [COOKIE]);
        let stored = std::fs::read_to_string(f.dir.path().join("skool-session")).unwrap();
        assert_eq!(stored, COOKIE);
        assert!(f.service.status().unwrap().logged_in);
    }

    #[tokio::test]
    async fn invalid_cookies_never_reach_skool_or_disk() {
        let f = ok_fixture(Mode::Quick);
        let err = f
            .service
            .login_with_cookies("a=SECRETVALUE")
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidCookies);
        assert!(!err.message.contains("SECRETVALUE"));
        assert!(f.factory.cookies.lock().unwrap().is_empty());
        assert!(!f.service.status().unwrap().logged_in);
    }

    #[tokio::test]
    async fn rejected_cookies_are_not_persisted() {
        let f = fixture(Err(PlatformError::SessionExpired), Mode::Quick);
        let err = f.service.login_with_cookies(COOKIE).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SessionExpired);
        assert!(!f.dir.path().join("skool-session").exists());
        assert!(!f.service.status().unwrap().logged_in);
    }

    #[tokio::test]
    async fn groups_require_a_session_and_report_expiry() {
        let f = ok_fixture(Mode::Quick);
        assert_eq!(
            f.service.groups().await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
        f.service.login_with_cookies(COOKIE).await.unwrap();
        assert_eq!(f.service.groups().await.unwrap()[0].slug, "growth");
        assert_eq!(f.factory.cookies.lock().unwrap().len(), 2);

        let stale = fixture(Err(PlatformError::SessionExpired), Mode::Quick);
        std::fs::write(stale.dir.path().join("skool-session"), COOKIE).unwrap();
        assert_eq!(
            stale.service.groups().await.unwrap_err().code,
            ErrorCode::SessionExpired
        );
    }

    #[tokio::test]
    async fn logout_forgets_the_session_and_is_idempotent() {
        let f = logged_in(Mode::Quick).await;
        f.service.logout().unwrap();
        f.service.logout().unwrap();
        assert!(!f.service.status().unwrap().logged_in);
        assert_eq!(
            f.service.groups().await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
    }

    #[tokio::test]
    async fn download_needs_a_session_and_a_library_folder() {
        let f = ok_fixture(Mode::Quick);
        let (tx, _rx) = mpsc::unbounded_channel();
        let events: Arc<dyn DownloadEvents> = Arc::new(Channel(tx));
        let e = f
            .service
            .start_download("growth", f.dir.path().into(), events.clone())
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NotLoggedIn);
        f.service.login_with_cookies(COOKIE).await.unwrap();
        let e = f
            .service
            .start_download("growth", PathBuf::new(), events)
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NoLibrary);
    }

    #[tokio::test]
    async fn download_emits_throttled_progress_then_one_report_and_cleans_up() {
        let f = logged_in(Mode::Quick).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = f
            .service
            .start_download("growth", f.dir.path().into(), Arc::new(Channel(tx)))
            .unwrap();
        let (report, progress) = next_report(&mut rx).await;
        assert_eq!(report.id, id);
        assert_eq!(report.report.group, "growth");
        assert!(report.report.complete);
        let percents: Vec<f32> = progress.iter().map(|p| p.progress.percent).collect();
        assert_eq!(percents, [0.0, 40.0], "0.2 is below the 1% throttle step");
        assert!(progress.iter().all(|p| p.id == id && p.platform == "skool"));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !f.service.running().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_same_group_cannot_run_twice_and_cancel_ends_it() {
        let f = logged_in(Mode::UntilCancelled).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let events: Arc<dyn DownloadEvents> = Arc::new(Channel(tx));
        let id = f
            .service
            .start_download("growth", f.dir.path().into(), events.clone())
            .unwrap();
        assert_eq!(
            f.service.running(),
            [RunningDownload {
                id: id.clone(),
                platform: "skool".into(),
                group: "growth".into()
            }]
        );
        assert_eq!(
            serde_json::to_value(f.service.running()).unwrap(),
            serde_json::json!([{"id": id, "platform": "skool", "group": "growth"}])
        );
        let dup = f
            .service
            .start_download("growth", f.dir.path().into(), events.clone())
            .unwrap_err();
        assert_eq!(dup.code, ErrorCode::AlreadyRunning);
        assert!(!f.service.cancel("dl-unknown"));
        assert!(f.service.cancel(&id));
        let (report, _) = next_report(&mut rx).await;
        assert!(report.report.cancelled && !report.report.complete);
    }

    #[tokio::test]
    async fn a_fatal_platform_error_still_produces_a_failed_report() {
        let f = logged_in(Mode::Fails).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = f
            .service
            .start_download("growth", f.dir.path().into(), Arc::new(Channel(tx)))
            .unwrap();
        let (report, _) = next_report(&mut rx).await;
        assert_eq!(report.id, id);
        assert!(!report.report.complete);
        let issue: &Issue = &report.report.issues[0];
        assert_eq!(issue.kind, IssueKind::Course);
        assert!(issue.message.contains("boom"));
    }

    #[tokio::test]
    async fn logout_cancels_running_downloads() {
        let f = logged_in(Mode::UntilCancelled).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        f.service
            .start_download("growth", f.dir.path().into(), Arc::new(Channel(tx)))
            .unwrap();
        f.service.logout().unwrap();
        let (report, _) = next_report(&mut rx).await;
        assert!(report.report.cancelled);
    }

    #[test]
    fn jar_becomes_a_header_only_once_logged_in() {
        assert_eq!(
            header_from_jar([("aws-waf-token", "w"), ("AWSALB", "x")]),
            None
        );
        assert_eq!(header_from_jar([("auth_token", "")]), None);
        assert_eq!(
            header_from_jar([("aws-waf-token", "w"), ("auth_token", "t")]).as_deref(),
            Some("aws-waf-token=w; auth_token=t")
        );
    }

    #[test]
    fn platform_errors_map_to_stable_codes() {
        let code = |e: PlatformError| SkoolCommandError::from(e).code;
        assert_eq!(
            code(PlatformError::SessionExpired),
            ErrorCode::SessionExpired
        );
        assert_eq!(
            code(PlatformError::InvalidInput("x".into())),
            ErrorCode::InvalidCookies
        );
        assert_eq!(code(PlatformError::Network("x".into())), ErrorCode::Network);
        assert_eq!(code(PlatformError::Parse("x".into())), ErrorCode::Internal);
        assert_eq!(code(PlatformError::Io("x".into())), ErrorCode::Internal);
    }

    #[test]
    fn errors_and_events_have_the_documented_json_shape() {
        let e = SkoolCommandError::new(ErrorCode::NotLoggedIn, "not logged in");
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"code":"not_logged_in","message":"not logged in"})
        );
        let p = ProgressEvent {
            id: "dl-1".into(),
            platform: "skool".into(),
            progress: DownloadProgress {
                group: "g".into(),
                course: "c".into(),
                lesson: "l".into(),
                percent: 50.0,
                completed: 1,
                total: 4,
            },
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::json!({"id":"dl-1","platform":"skool","group":"g","course":"c","lesson":"l",
                               "percent":50.0,"completed":1,"total":4})
        );
        let mut report = DownloadReport::new("skool", "g");
        report.finish();
        let v = serde_json::to_value(&ReportEvent {
            id: "dl-1".into(),
            report,
        })
        .unwrap();
        assert_eq!(v["id"], "dl-1");
        assert_eq!(v["platform"], "skool");
        assert_eq!(v["complete"], true);
        assert_eq!(v["totalLessons"], 0);
        assert!(v["issues"].as_array().unwrap().is_empty());
        assert_eq!(
            serde_json::to_value(SessionStatus { logged_in: true }).unwrap(),
            serde_json::json!({"loggedIn": true})
        );
    }
}
