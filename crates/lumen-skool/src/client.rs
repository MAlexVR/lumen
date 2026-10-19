//! HTTP client for Skool: cookie auth, browser-like headers, retry/backoff.
//! Never touches `_next/data` (blocked by the WAF): only HTML pages and the
//! `self/groups` API are used.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lumen_core::PlatformGroup;
use lumen_download::http::{redact_message, send_with_retry, write_stream, FetchError};
pub use lumen_download::RetryPolicy;
use reqwest::header::{HeaderValue, ACCEPT, COOKIE, ORIGIN, REFERER, USER_AGENT};
use url::Url;

use crate::error::SkoolError;
use crate::links::{file_extension, is_skool_file_host};
use crate::parse::{
    extract_page_props, parse_course_tree, parse_courses, parse_groups, parse_module_page,
    CourseListing, CourseTree, ModuleDetail,
};

pub const WEB_BASE: &str = "https://www.skool.com";
pub const API_BASE: &str = "https://api2.skool.com";
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub web_base: String,
    pub api_base: String,
    pub retry: RetryPolicy,
    /// Minimum gap between module page requests.
    pub module_interval: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            web_base: WEB_BASE.into(),
            api_base: API_BASE.into(),
            retry: RetryPolicy::default(),
            module_interval: Duration::from_millis(400),
        }
    }
}

/// Result of fetching a linked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchedFile {
    Saved(PathBuf),
    /// The link is an HTML page, not a file: nothing was written.
    NotAFile,
}

pub struct SkoolClient {
    http: reqwest::Client,
    cookie: HeaderValue,
    config: ClientConfig,
    trusted_hosts: Vec<String>,
    last_module: tokio::sync::Mutex<Option<Instant>>,
}

impl std::fmt::Debug for SkoolClient {
    // Deliberately omits the cookie.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkoolClient")
            .field("web_base", &self.config.web_base)
            .field("api_base", &self.config.api_base)
            .finish_non_exhaustive()
    }
}

fn host_of(base: &str) -> Option<String> {
    Url::parse(base)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

impl SkoolClient {
    pub fn new(cookie_header: &str) -> Result<Self, SkoolError> {
        Self::with_config(cookie_header, ClientConfig::default())
    }

    pub fn with_config(cookie_header: &str, config: ClientConfig) -> Result<Self, SkoolError> {
        let mut cookie = HeaderValue::from_str(cookie_header)
            .map_err(|_| SkoolError::Cookie("the cookie header has invalid characters".into()))?;
        cookie.set_sensitive(true);
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| SkoolError::Network(e.without_url().to_string()))?;
        let trusted_hosts = [&config.web_base, &config.api_base]
            .into_iter()
            .filter_map(|b| host_of(b))
            .collect();
        Ok(Self {
            http,
            cookie,
            config,
            trusted_hosts,
            last_module: tokio::sync::Mutex::new(None),
        })
    }

    pub async fn groups(&self) -> Result<Vec<PlatformGroup>, SkoolError> {
        let url = format!("{}/self/groups?limit=50&prefs=false", self.config.api_base);
        let resp = self
            .send(&url, "application/json, text/plain, */*", false)
            .await?;
        let text = resp
            .text()
            .await
            .map_err(|e| SkoolError::Network(e.without_url().to_string()))?;
        let body: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| SkoolError::Json(e.to_string()))?;
        Ok(parse_groups(&body))
    }

    pub async fn classroom(&self, slug: &str) -> Result<CourseListing, SkoolError> {
        let pp = self
            .page_props(&format!("{}/{slug}/classroom", self.config.web_base))
            .await?;
        Ok(parse_courses(&pp))
    }

    pub async fn course(&self, slug: &str, course: &str) -> Result<CourseTree, SkoolError> {
        let url = format!("{}/{slug}/classroom/{course}", self.config.web_base);
        parse_course_tree(&self.page_props(&url).await?)
    }

    /// Module page (`?md=<id>`), spaced by `module_interval`.
    pub async fn module(
        &self,
        slug: &str,
        course: &str,
        module_id: &str,
    ) -> Result<ModuleDetail, SkoolError> {
        {
            let mut last = self.last_module.lock().await;
            if let Some(at) = *last {
                let wait = self.config.module_interval.saturating_sub(at.elapsed());
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
            *last = Some(Instant::now());
        }
        let url = format!(
            "{}/{slug}/classroom/{course}?md={module_id}",
            self.config.web_base
        );
        parse_module_page(&self.page_props(&url).await?, module_id)
    }

    /// Streams `url` to `<dest_base><ext>` through a `.part` file.
    pub async fn fetch_file(&self, url: &str, dest_base: &Path) -> Result<FetchedFile, SkoolError> {
        let resp = self.send(url, "*/*", true).await?;
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let content_type = header("content-type");
        let disposition = header("content-disposition");
        if content_type
            .as_deref()
            .is_some_and(|c| c.trim().to_ascii_lowercase().starts_with("text/html"))
        {
            return Ok(FetchedFile::NotAFile);
        }
        let ext = file_extension(url, content_type.as_deref(), disposition.as_deref());
        let mut name = dest_base.as_os_str().to_owned();
        name.push(&ext);
        let dest = PathBuf::from(name);
        write_stream(resp, &dest).await.map_err(|e| match e {
            FetchError::Network(m) => SkoolError::Network(redact_message(&m, url)),
            other => other.into(),
        })?;
        Ok(FetchedFile::Saved(dest))
    }

    async fn page_props(&self, url: &str) -> Result<serde_json::Value, SkoolError> {
        let resp = self
            .send(url, "text/html,application/xhtml+xml,*/*", true)
            .await?;
        let html = resp
            .text()
            .await
            .map_err(|e| SkoolError::Network(e.without_url().to_string()))?;
        extract_page_props(&html)
    }

    fn sends_cookie(&self, url: &Url) -> bool {
        let host = url.host_str().unwrap_or("").to_ascii_lowercase();
        is_skool_file_host(&host) || self.trusted_hosts.contains(&host)
    }

    /// GET with retry. 401 never retries; 403 retries only when `retry_403`
    /// (the WAF sometimes blocks a page once). Errors carry redacted URLs.
    async fn send(
        &self,
        url: &str,
        accept: &str,
        retry_403: bool,
    ) -> Result<reqwest::Response, SkoolError> {
        let parsed = Url::parse(url).map_err(|_| SkoolError::Shape("invalid URL".into()))?;
        let build = || {
            let mut req = self
                .http
                .get(parsed.clone())
                .header(USER_AGENT, BROWSER_UA)
                .header(ACCEPT, accept)
                .header(ORIGIN, WEB_BASE)
                .header(REFERER, format!("{WEB_BASE}/"));
            if self.sends_cookie(&parsed) {
                req = req.header(COOKIE, self.cookie.clone());
            }
            req
        };
        Ok(send_with_retry(&self.config.retry, retry_403, url, &build).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_download::testsrv::{Reply, TestServer};
    use std::time::Instant;

    const CLASSROOM: &str = include_str!("../tests/fixtures/classroom.html");
    const COURSE: &str = include_str!("../tests/fixtures/course.html");
    const MODULE: &str = include_str!("../tests/fixtures/module.html");
    const GROUPS: &str = include_str!("../tests/fixtures/groups.json");
    const COOKIE: &str = "auth_token=SECRETTOKEN; aws-waf-token=w";

    fn fast_config(base: &str) -> ClientConfig {
        ClientConfig {
            web_base: base.into(),
            api_base: base.into(),
            retry: RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(5),
            },
            module_interval: Duration::ZERO,
        }
    }

    fn client(srv: &TestServer) -> SkoolClient {
        SkoolClient::with_config(COOKIE, fast_config(&srv.base())).unwrap()
    }

    fn site() -> impl Fn(&lumen_download::testsrv::Recorded, usize) -> Reply + Send + Sync + 'static
    {
        |req, _| {
            let p = req.path_and_query.as_str();
            if p.starts_with("/self/groups") {
                Reply::json(GROUPS)
            } else if p == "/g/classroom" {
                Reply::html(CLASSROOM)
            } else if p == "/g/classroom/c" {
                Reply::html(COURSE)
            } else if p == "/g/classroom/c?md=m1" {
                Reply::html(MODULE)
            } else {
                Reply::status(404)
            }
        }
    }

    #[tokio::test]
    async fn groups_send_cookie_and_browser_headers() {
        let srv = TestServer::start(site()).await;
        let groups = client(&srv).groups().await.unwrap();
        assert_eq!(groups[0].slug, "growth-lab");
        let req = &srv.requests()[0];
        assert_eq!(req.path_and_query, "/self/groups?limit=50&prefs=false");
        assert_eq!(req.header("cookie").as_deref(), Some(COOKIE));
        assert_eq!(req.header("user-agent").as_deref(), Some(BROWSER_UA));
        assert_eq!(
            req.header("referer").as_deref(),
            Some("https://www.skool.com/")
        );
        assert_eq!(
            req.header("origin").as_deref(),
            Some("https://www.skool.com")
        );
    }

    #[tokio::test]
    async fn expired_session_is_reported_without_retrying() {
        for status in [401u16, 403] {
            let srv = TestServer::start(move |_, _| Reply::status(status)).await;
            let err = client(&srv).groups().await.unwrap_err();
            assert_eq!(err, SkoolError::Unauthorized(status));
            assert_eq!(srv.count(), 1, "groups must not retry {status}");
        }
    }

    #[tokio::test]
    async fn pages_parse_and_never_use_next_data_endpoints() {
        let srv = TestServer::start(site()).await;
        let c = client(&srv);
        assert_eq!(c.classroom("g").await.unwrap().courses.len(), 2);
        assert_eq!(c.course("g", "c").await.unwrap().modules.len(), 4);
        let m = c.module("g", "c", "m1").await.unwrap();
        assert!(m.native.is_some());
        for r in srv.requests() {
            assert!(!r.path_and_query.contains("_next"), "{}", r.path_and_query);
        }
        assert!(srv.requests()[2].path_and_query.ends_with("?md=m1"));
    }

    #[tokio::test]
    async fn html_without_next_data_is_session_blocked() {
        let srv = TestServer::start(|_, _| Reply::html("<html>please log in</html>")).await;
        assert_eq!(
            client(&srv).classroom("g").await.unwrap_err(),
            SkoolError::SessionBlocked
        );
    }

    #[tokio::test]
    async fn rate_limit_and_server_errors_are_retried_with_backoff() {
        let srv = TestServer::start(|_, n| {
            if n < 2 {
                Reply::status(if n == 0 { 429 } else { 503 })
            } else {
                Reply::html(CLASSROOM)
            }
        })
        .await;
        assert!(client(&srv).classroom("g").await.is_ok());
        assert_eq!(srv.count(), 3);
    }

    #[tokio::test]
    async fn retries_are_bounded_and_the_final_status_is_reported() {
        let srv = TestServer::start(|_, _| Reply::status(500)).await;
        let err = client(&srv).classroom("g").await.unwrap_err();
        assert_eq!(
            err,
            SkoolError::Http {
                status: 500,
                url: format!("{}/g/classroom", srv.base())
            }
        );
        assert_eq!(srv.count(), 3);
        let waf = TestServer::start(|_, _| Reply::status(403)).await;
        assert_eq!(
            client(&waf).classroom("g").await.unwrap_err(),
            SkoolError::Unauthorized(403)
        );
        assert_eq!(waf.count(), 3, "pages retry a WAF 403");
        let missing = TestServer::start(|_, _| Reply::status(404)).await;
        assert!(client(&missing).classroom("g").await.is_err());
        assert_eq!(missing.count(), 1, "404 is not retried");
    }

    #[tokio::test]
    async fn module_pages_are_spaced_by_the_configured_interval() {
        let srv = TestServer::start(site()).await;
        let mut cfg = fast_config(&srv.base());
        cfg.module_interval = Duration::from_millis(80);
        let c = SkoolClient::with_config(COOKIE, cfg).unwrap();
        let t = Instant::now();
        c.module("g", "c", "m1").await.unwrap();
        c.module("g", "c", "m1").await.unwrap();
        c.module("g", "c", "m1").await.unwrap();
        assert!(
            t.elapsed() >= Duration::from_millis(160),
            "{:?}",
            t.elapsed()
        );
    }

    #[tokio::test]
    async fn debug_output_and_errors_never_contain_the_cookie() {
        let srv = TestServer::start(|_, _| Reply::status(500)).await;
        let c = client(&srv);
        assert!(!format!("{c:?}").contains("SECRETTOKEN"));
        let err = c.module("g", "c", "m1").await.unwrap_err();
        assert!(!err.to_string().contains("SECRETTOKEN"));
        assert!(
            !err.to_string().contains("md=m1"),
            "query is redacted: {err}"
        );
    }

    #[test]
    fn invalid_cookie_header_is_rejected() {
        assert!(SkoolClient::new("auth_token=a\nb").is_err());
        assert!(SkoolClient::new("auth_token=ok").is_ok());
    }

    #[tokio::test]
    async fn files_are_saved_with_the_extension_from_the_headers() {
        let srv = TestServer::start(|req, _| match req.path_and_query.as_str() {
            "/f/abc" => Reply::bytes(200, "application/pdf", b"%PDF-1".to_vec()),
            "/f/named" => Reply::bytes(200, "application/octet-stream", b"zz".to_vec())
                .with_header("content-disposition", "attachment; filename=\"Plan.XLSX\""),
            "/page" => Reply::html("<html>not a file</html>"),
            "/empty" => Reply::bytes(200, "application/pdf", vec![]),
            _ => Reply::status(404),
        })
        .await;
        let c = client(&srv);
        let tmp = tempfile::tempdir().unwrap();
        let url = |p: &str| format!("{}{p}", srv.foreign_base());

        let got = c
            .fetch_file(&url("/f/abc"), &tmp.path().join("Guide"))
            .await
            .unwrap();
        assert_eq!(got, FetchedFile::Saved(tmp.path().join("Guide.pdf")));
        assert_eq!(
            std::fs::read(tmp.path().join("Guide.pdf")).unwrap(),
            b"%PDF-1"
        );

        let got = c
            .fetch_file(&url("/f/named"), &tmp.path().join("Plan"))
            .await
            .unwrap();
        assert_eq!(got, FetchedFile::Saved(tmp.path().join("Plan.xlsx")));

        assert_eq!(
            c.fetch_file(&url("/page"), &tmp.path().join("P"))
                .await
                .unwrap(),
            FetchedFile::NotAFile
        );
        assert!(c
            .fetch_file(&url("/empty"), &tmp.path().join("E"))
            .await
            .is_err());
        assert!(c
            .fetch_file(&url("/missing"), &tmp.path().join("M"))
            .await
            .is_err());

        let mut left: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["Guide.pdf", "Plan.xlsx"],
            "no .part or empty files remain"
        );
    }

    #[tokio::test]
    async fn cookie_is_only_sent_to_skool_hosts() {
        let srv =
            TestServer::start(|_, _| Reply::bytes(200, "application/pdf", b"x".to_vec())).await;
        let c = client(&srv);
        let tmp = tempfile::tempdir().unwrap();
        // 127.0.0.1 is not a Skool host (the configured bases use localhost).
        c.fetch_file(
            &format!("{}/a.pdf", srv.foreign_base()),
            &tmp.path().join("A"),
        )
        .await
        .unwrap();
        c.fetch_file(&format!("{}/b.pdf", srv.base()), &tmp.path().join("B"))
            .await
            .unwrap();
        let reqs = srv.requests();
        assert_eq!(reqs[0].header("cookie"), None);
        assert_eq!(reqs[1].header("cookie").as_deref(), Some(COOKIE));
    }

    #[tokio::test]
    async fn fetch_file_creates_missing_directories() {
        let srv = TestServer::start(|_, _| Reply::bytes(200, "image/png", b"png".to_vec())).await;
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("Attachments/pic");
        client(&srv)
            .fetch_file(&format!("{}/x", srv.foreign_base()), &base)
            .await
            .unwrap();
        assert!(tmp.path().join("Attachments/pic.png").is_file());
    }
}
