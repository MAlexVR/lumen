//! Udemy commands' application layer (phase 1: session and diagnostic, no
//! downloader yet). It holds no Tauri types, so it is tested without a
//! window; `lib.rs` only adapts it to commands and events.

use lumen_core::CancelFlag;
use lumen_udemy::classify::ClassCounts;
use lumen_udemy::credentials::header_from_pairs;
use lumen_udemy::diagnose::{
    iso8601_utc, run, DiagnoseOptions, DiagnoseSink, Pacing, RunMeta, VerdictCounts,
};
use lumen_udemy::parse::CourseSummary;
use lumen_udemy::{ClientConfig, Credentials, UdemyClient, UdemyError};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::downloads::{CommandError, ErrorCode, SessionStatus};
use crate::session::SessionStore;

pub type UdemyCommandError = CommandError;

/// File name of the sanitized diagnostic inside the app data folder.
pub const DIAGNOSTIC_FILE: &str = "udemy-diagnostico.json";

/// Header built from the webview cookie jar, once a session with a token exists.
pub fn header_from_jar<'a>(
    cookies: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    header_from_pairs(cookies)
}

fn map(e: UdemyError) -> UdemyCommandError {
    let code = match &e {
        UdemyError::InvalidCookies(_) => ErrorCode::InvalidCookies,
        UdemyError::Unauthorized | UdemyError::Forbidden => ErrorCode::SessionExpired,
        UdemyError::BotChallenge => ErrorCode::BotChallenge,
        UdemyError::Network(_) | UdemyError::Http { .. } => ErrorCode::Network,
        _ => ErrorCode::Internal,
    };
    CommandError::new(code, e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginOutcome {
    pub logged_in: bool,
    pub total_courses: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointBrief {
    pub name: String,
    pub status: String,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseSummary {
    /// `complete`, `cancelled` or `aborted`.
    pub status: String,
    pub courses: u32,
    pub lectures: u32,
    pub probed: u32,
    pub classification: ClassCounts,
    pub verdicts: VerdictCounts,
    pub endpoints: Vec<EndpointBrief>,
    pub issues: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseOutcome {
    pub path: String,
    pub summary: DiagnoseSummary,
}

pub struct UdemyService {
    session: SessionStore,
    data_dir: PathBuf,
    app_version: String,
    config: ClientConfig,
    pacing: Pacing,
    running: Mutex<Option<CancelFlag>>,
}

/// Frees the diagnostic slot however the run ends.
struct Slot<'a>(&'a Mutex<Option<CancelFlag>>);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl UdemyService {
    pub fn new(session: SessionStore, data_dir: PathBuf, app_version: &str) -> Self {
        Self {
            session,
            data_dir,
            app_version: app_version.to_string(),
            config: ClientConfig::default(),
            pacing: Pacing::default(),
            running: Mutex::new(None),
        }
    }

    /// Points the client at another server and sets the pause between lecture
    /// probes (tests use a loopback server and no pause).
    #[cfg(test)]
    #[must_use]
    pub fn with_endpoints(
        mut self,
        endpoints: lumen_udemy::Endpoints,
        detail_delay: std::time::Duration,
    ) -> Self {
        use lumen_download::RetryPolicy;
        use std::time::Duration;
        self.config = ClientConfig {
            endpoints,
            retry: RetryPolicy {
                max_attempts: 2,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(2),
            },
        };
        self.pacing.detail_delay = detail_delay;
        self
    }

    fn credentials(&self) -> Result<Credentials, UdemyCommandError> {
        let stored = self
            .session
            .load()
            .map_err(|e| UdemyCommandError::internal("cannot read the stored session", e))?;
        stored
            .and_then(|text| Credentials::from_cookie_text(&text).ok())
            .ok_or_else(|| UdemyCommandError::new(ErrorCode::NotLoggedIn, "not logged in to Udemy"))
    }

    fn client(&self, creds: Credentials) -> UdemyClient {
        UdemyClient::with_config(creds, self.config.clone())
    }

    /// Validates a pasted cookie header with a minimal courses request and
    /// stores it only when Udemy accepts it.
    pub async fn login_with_cookies(&self, text: &str) -> Result<LoginOutcome, UdemyCommandError> {
        let creds = Credentials::from_cookie_text(text).map_err(map)?;
        let total_courses = self
            .client(creds.clone())
            .verify_session()
            .await
            .map_err(map)?;
        self.session
            .save(creds.cookie_header())
            .map_err(|e| UdemyCommandError::internal("cannot store the session", e))?;
        Ok(LoginOutcome {
            logged_in: true,
            total_courses,
        })
    }

    pub fn status(&self) -> Result<SessionStatus, UdemyCommandError> {
        match self.credentials() {
            Ok(_) => Ok(SessionStatus { logged_in: true }),
            Err(e) if e.code == ErrorCode::NotLoggedIn => Ok(SessionStatus { logged_in: false }),
            Err(e) => Err(e),
        }
    }

    /// Forgets the session and stops a running diagnostic.
    pub fn logout(&self) -> Result<(), UdemyCommandError> {
        self.cancel_diagnose();
        self.session
            .clear()
            .map_err(|e| UdemyCommandError::internal("cannot remove the stored session", e))
    }

    pub async fn courses(&self) -> Result<Vec<CourseSummary>, UdemyCommandError> {
        let client = self.client(self.credentials()?);
        client
            .courses()
            .await
            .map(|(courses, _)| courses)
            .map_err(map)
    }

    pub fn cancel_diagnose(&self) -> bool {
        match self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            Some(flag) => {
                flag.cancel();
                true
            }
            None => false,
        }
    }

    /// Runs the diagnostic and writes its sanitized report to the app data
    /// folder. Only one runs at a time.
    pub async fn diagnose(
        &self,
        options: DiagnoseOptions,
        sink: &dyn DiagnoseSink,
    ) -> Result<DiagnoseOutcome, UdemyCommandError> {
        let creds = self.credentials()?;
        let cancel = CancelFlag::new();
        {
            let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
            if running.is_some() {
                return Err(UdemyCommandError::new(
                    ErrorCode::DiagnoseRunning,
                    "a diagnostic is already running",
                ));
            }
            *running = Some(cancel.clone());
        }
        let _slot = Slot(&self.running);
        let secrets = creds.secret_values();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let meta = RunMeta {
            app_version: self.app_version.clone(),
            generated_at: iso8601_utc(now),
            secrets: secrets.clone(),
        };
        let client = self.client(creds);
        let diagnostic = run(&client, &options, &self.pacing, sink, &cancel, &meta)
            .await
            .map_err(map)?;
        let path = self.data_dir.join(DIAGNOSTIC_FILE);
        diagnostic
            .write_to(&path, &secrets)
            .map_err(|e| UdemyCommandError::internal("cannot write the diagnostic", e))?;
        let status = serde_json::to_value(diagnostic.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        Ok(DiagnoseOutcome {
            path: path.to_string_lossy().into_owned(),
            summary: DiagnoseSummary {
                status,
                courses: diagnostic.totals.courses,
                lectures: diagnostic.totals.lectures,
                probed: diagnostic.totals.probed,
                classification: diagnostic.totals.classification,
                verdicts: diagnostic.totals.verdicts,
                endpoints: diagnostic
                    .endpoints
                    .iter()
                    .map(|e| EndpointBrief {
                        name: e.name.clone(),
                        status: e.status.clone(),
                        verified: e.verified,
                    })
                    .collect(),
                issues: diagnostic.issues.len(),
            },
        })
    }

    /// The last diagnostic written, if any.
    pub fn last_diagnostic(&self) -> Result<Option<Value>, UdemyCommandError> {
        match std::fs::read_to_string(self.data_dir.join(DIAGNOSTIC_FILE)) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| UdemyCommandError::internal("the diagnostic file is unreadable", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(UdemyCommandError::internal("cannot read the diagnostic", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_download::testsrv::{Recorded, Reply, TestServer};
    use lumen_udemy::diagnose::{DiagnoseProgress, DiagnoseSink};
    use lumen_udemy::Endpoints;
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use std::time::Duration;

    const TOKEN: &str = "TOKENSECRET1";
    const COOKIES: &str = "client_id=cid12345; access_token=TOKENSECRET1";
    const EMAIL: &str = "ana.owner@example.com";

    #[derive(Default)]
    struct Events(Mutex<Vec<DiagnoseProgress>>);

    impl DiagnoseSink for Events {
        fn progress(&self, p: &DiagnoseProgress) {
            self.0.lock().unwrap().push(p.clone());
        }
    }

    fn mock(req: &Recorded, _: usize) -> Reply {
        let p = req.path_and_query.as_str();
        if p.contains("/lectures/") {
            return Reply::json(
                r#"{"id":11,"asset":{"asset_type":"Video","stream_urls":{"Video":[{"type":"video/mp4","file":"https://a.udemycdn.com/v.mp4?token=TOKENSECRET1"}]}}}"#,
            );
        }
        if p.contains("/subscriber-curriculum-items/") {
            return Reply::json(
                r#"{"next":null,"results":[{"_class":"chapter","id":1,"title":"S"},{"_class":"lecture","id":11,"title":"L","asset":{"asset_type":"Video"}}]}"#,
            );
        }
        let instructors =
            json!([{"_class": "user", "id": 7, "display_name": "Ana", "email": EMAIL}]);
        Reply::json(
            &json!({"count": 1, "next": null, "results": [
                {"_class": "course", "id": 1001, "title": "Curso Uno", "num_published_lectures": 1,
                 "image_480x270": "https://img.udemycdn.com/c.jpg", "visible_instructors": instructors}]})
            .to_string(),
        )
    }

    struct Fixture {
        srv: TestServer,
        service: UdemyService,
        dir: tempfile::TempDir,
    }

    async fn fixture(
        handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static,
    ) -> Fixture {
        let srv = TestServer::start(handler).await;
        let dir = tempfile::tempdir().unwrap();
        let service = UdemyService::new(
            SessionStore::for_platform(dir.path(), "udemy"),
            dir.path().into(),
            "9.9.9",
        )
        .with_endpoints(Endpoints::under(&srv.base()), Duration::ZERO);
        Fixture { srv, service, dir }
    }

    #[tokio::test]
    async fn valid_cookies_are_verified_with_a_minimal_request_then_persisted_privately() {
        let f = fixture(mock).await;
        let out = f
            .service
            .login_with_cookies(&format!("Cookie: {COOKIES}\n"))
            .await
            .unwrap();
        assert!(out.logged_in);
        assert_eq!(out.total_courses, Some(1));
        assert_eq!(f.srv.count(), 1);
        assert!(f.srv.requests()[0].path_and_query.contains("page_size=1"));
        let stored = std::fs::read_to_string(f.dir.path().join("udemy-session")).unwrap();
        assert_eq!(stored, COOKIES);
        assert!(f.service.status().unwrap().logged_in);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(f.dir.path().join("udemy-session"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn bad_cookies_never_reach_udemy_or_disk_and_rejections_are_not_persisted() {
        let f = fixture(mock).await;
        let e = f
            .service
            .login_with_cookies("a=SECRETVALUE")
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidCookies);
        assert!(!e.message.contains("SECRETVALUE"));
        assert_eq!(f.srv.count(), 0);
        assert!(!f.service.status().unwrap().logged_in);

        let f = fixture(|_, _| Reply::status(401)).await;
        let e = f.service.login_with_cookies(COOKIES).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::SessionExpired);
        assert!(!f.dir.path().join("udemy-session").exists());

        let f = fixture(|_, _| {
            let mut r = Reply::html("<html>Just a moment... captcha</html>");
            r.status = 403;
            r
        })
        .await;
        let e = f.service.login_with_cookies(COOKIES).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::BotChallenge);
        assert!(!e.message.contains(TOKEN));
        assert!(!f.dir.path().join("udemy-session").exists());
    }

    #[tokio::test]
    async fn without_a_session_commands_say_not_logged_in_and_logout_forgets_it() {
        let f = fixture(mock).await;
        assert_eq!(
            f.service.courses().await.unwrap_err().code,
            ErrorCode::NotLoggedIn
        );
        let events = Events::default();
        assert_eq!(
            f.service
                .diagnose(Default::default(), &events)
                .await
                .unwrap_err()
                .code,
            ErrorCode::NotLoggedIn
        );
        assert_eq!(f.srv.count(), 0);
        f.service.login_with_cookies(COOKIES).await.unwrap();
        f.service.logout().unwrap();
        assert!(!f.service.status().unwrap().logged_in);
        assert!(!f.dir.path().join("udemy-session").exists());
        f.service.logout().unwrap();
    }

    #[tokio::test]
    async fn the_course_list_has_the_ui_shape() {
        let f = fixture(mock).await;
        f.service.login_with_cookies(COOKIES).await.unwrap();
        let courses = f.service.courses().await.unwrap();
        let v = serde_json::to_value(&courses).unwrap();
        assert_eq!(v[0]["id"], "1001");
        assert_eq!(v[0]["title"], "Curso Uno");
        assert_eq!(v[0]["coverUrl"], "https://img.udemycdn.com/c.jpg");
        assert_eq!(v[0]["lectures"], 1);
        assert_eq!(v[0]["instructors"][0], "Ana");
        assert_eq!(
            f.srv
                .requests()
                .last()
                .unwrap()
                .header("authorization")
                .as_deref(),
            Some("Bearer TOKENSECRET1")
        );
    }

    #[tokio::test]
    async fn the_diagnostic_writes_a_sanitized_private_file_and_returns_its_summary() {
        let f = fixture(mock).await;
        f.service.login_with_cookies(COOKIES).await.unwrap();
        assert!(f.service.last_diagnostic().unwrap().is_none());
        let events = Events::default();
        let out = f
            .service
            .diagnose(Default::default(), &events)
            .await
            .unwrap();
        assert_eq!(
            out.path,
            f.dir
                .path()
                .join("udemy-diagnostico.json")
                .to_string_lossy()
        );
        assert_eq!(out.summary.status, "complete");
        assert_eq!(out.summary.courses, 1);
        assert_eq!(out.summary.probed, 1);
        assert!(
            out.summary.endpoints.iter().all(|e| e.status == "ok"),
            "{:?}",
            out.summary.endpoints
        );
        let text = std::fs::read_to_string(&out.path).unwrap();
        for leak in [TOKEN, "cid12345", EMAIL, "Curso Uno", "v.mp4?token"] {
            assert!(
                !text.contains(leak),
                "{leak} leaked into the diagnostic file"
            );
        }
        assert_eq!(
            events.0.lock().unwrap().last().unwrap().step,
            lumen_udemy::diagnose::Step::Finish
        );
        let last: Value = f.service.last_diagnostic().unwrap().unwrap();
        assert_eq!(last["accountKind"], "personal");
        assert_eq!(last["appVersion"], "9.9.9");
        assert!(last["generatedAt"].as_str().unwrap().ends_with('Z'));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_one_diagnostic_runs_at_a_time_and_it_can_be_cancelled() {
        let f = fixture(|req, n| {
            std::thread::sleep(Duration::from_millis(150));
            mock(req, n)
        })
        .await;
        f.service.login_with_cookies(COOKIES).await.unwrap();
        let service = std::sync::Arc::new(f.service);
        let first = {
            let s = service.clone();
            tokio::spawn(async move { s.diagnose(Default::default(), &Events::default()).await })
        };
        tokio::time::sleep(Duration::from_millis(60)).await;
        let second = service
            .diagnose(Default::default(), &Events::default())
            .await
            .unwrap_err();
        assert_eq!(second.code, ErrorCode::DiagnoseRunning);
        assert!(service.cancel_diagnose());
        let out = first.await.unwrap().unwrap();
        assert_eq!(out.summary.status, "cancelled");
        assert!(!service.cancel_diagnose(), "nothing is running any more");
        let again = service
            .diagnose(Default::default(), &Events::default())
            .await;
        assert!(again.is_ok(), "the slot is free again");
    }

    #[test]
    fn the_new_error_codes_serialize_as_snake_case() {
        for (code, want) in [
            (ErrorCode::DrmProtected, "drm_protected"),
            (ErrorCode::DiagnoseRunning, "diagnose_running"),
            (ErrorCode::BotChallenge, "bot_challenge"),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), want);
        }
    }

    #[test]
    fn the_webview_jar_gives_a_header_only_with_a_token() {
        assert_eq!(
            header_from_jar([("a", "1"), ("access_token", "T")]).as_deref(),
            Some("a=1; access_token=T")
        );
        assert_eq!(header_from_jar([("a", "1")]), None);
    }
}
