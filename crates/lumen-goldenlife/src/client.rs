//! Cademí web client: form login, cookie jar kept across requests, manual
//! same-origin redirects, bounded retry and redacted errors.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use lumen_download::http::{backoff, write_stream};
use lumen_download::{FetchError, RetryPolicy};
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, COOKIE, LOCATION, ORIGIN,
    REFERER, SET_COOKIE, USER_AGENT,
};
use reqwest::{Method, StatusCode};
use url::Url;

use crate::credentials::{Login, Session};
use crate::error::GoldenlifeError;
use crate::parse::{
    is_login_page, parse_home, parse_lesson, parse_login_token, parse_outline, CourseInfo,
    LessonPage, Outline,
};

const SITE: &str = "https://my.goldenlife-academy.com";
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
const MAX_REDIRECTS: usize = 6;
const LOGIN_PATH: &str = "/auth/login";
const ATTACHMENT_PATH: &str = "/area/arquivo/anexo";

#[derive(Clone)]
pub struct ClientConfig {
    pub base_url: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub retry: RetryPolicy,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: SITE.into(),
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(60),
            retry: RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(6),
            },
        }
    }
}

impl ClientConfig {
    pub fn under(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let origin = Url::parse(&self.base_url)
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_else(|_| "<invalid>".into());
        f.debug_struct("ClientConfig")
            .field("origin", &origin)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

/// What the app needs from the academy (implemented by [`GoldenlifeClient`]).
#[async_trait]
pub trait GoldenlifeApi: Send + Sync {
    /// Fails with `Unauthorized` when the session is no longer valid.
    async fn validate(&self) -> Result<(), GoldenlifeError>;
    async fn courses(&self) -> Result<Vec<CourseInfo>, GoldenlifeError>;
    /// Sections and lessons of a course; empty when the course has no content
    /// of its own (an alias of another course).
    async fn outline(&self, course_id: &str) -> Result<Outline, GoldenlifeError>;
    async fn lesson(&self, lesson_id: &str) -> Result<LessonPage, GoldenlifeError>;
    /// Saves a lesson attachment (`/area/arquivo/anexo?...` as parsed).
    async fn download_attachment(&self, path: &str, dest: &Path) -> Result<(), GoldenlifeError>;
    /// Origin of the academy, used as Referer for the embedded players.
    fn referer(&self) -> String;
}

/// Cookies by name; the server rotates them on any response.
#[derive(Default)]
struct Jar(BTreeMap<String, String>);

impl Jar {
    fn from_session(session: &Session) -> Self {
        let cookies = session
            .cookie_header()
            .split("; ")
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        Self(cookies)
    }

    fn absorb(&mut self, headers: &HeaderMap) {
        for raw in headers.get_all(SET_COOKIE) {
            let Ok(raw) = raw.to_str() else { continue };
            let (pair, attributes) = raw.split_once(';').unwrap_or((raw, ""));
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            let attributes = attributes.to_ascii_lowercase();
            let expired = value == "deleted"
                || attributes.contains("max-age=0")
                || attributes.contains("max-age=-");
            if expired {
                self.0.remove(name);
            } else if !name.is_empty() {
                self.0.insert(name.to_string(), value.to_string());
            }
        }
    }

    fn header(&self) -> String {
        self.0
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

pub struct GoldenlifeClient {
    http: reqwest::Client,
    base_url: Url,
    jar: Mutex<Jar>,
    retry: RetryPolicy,
}

impl std::fmt::Debug for GoldenlifeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoldenlifeClient")
            .field("origin", &self.base_url.origin().ascii_serialization())
            .finish_non_exhaustive()
    }
}

fn network(error: reqwest::Error) -> GoldenlifeError {
    GoldenlifeError::Network(error.without_url().to_string())
}

fn numeric(id: &str) -> Result<&str, GoldenlifeError> {
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(id)
    } else {
        Err(GoldenlifeError::InvalidResponse(
            "identifier is not numeric",
        ))
    }
}

impl GoldenlifeClient {
    fn build(config: &ClientConfig, jar: Jar) -> Result<Self, GoldenlifeError> {
        let base_url = Url::parse(&config.base_url)
            .map_err(|_| GoldenlifeError::InvalidClientConfig("base URL is invalid"))?;
        let host = base_url
            .host_str()
            .ok_or(GoldenlifeError::InvalidClientConfig("base URL has no host"))?;
        let loopback = host == "localhost"
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        let secure = base_url.scheme() == "https" || (base_url.scheme() == "http" && loopback);
        if !secure
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.path() != "/"
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(GoldenlifeError::InvalidClientConfig(
                "base URL must be an HTTPS origin (HTTP is allowed only for loopback tests)",
            ));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| GoldenlifeError::InvalidClientConfig("HTTP client could not be built"))?;
        Ok(Self {
            http,
            base_url,
            jar: Mutex::new(jar),
            retry: config.retry.clone(),
        })
    }

    /// Client for a stored session.
    pub fn new(session: &Session, config: &ClientConfig) -> Result<Self, GoldenlifeError> {
        Self::build(config, Jar::from_session(session))
    }

    /// Signs in with the academy's form (asking for the long-lived "remember
    /// me" cookie) and returns the authenticated client.
    pub async fn login(config: &ClientConfig, login: &Login) -> Result<Self, GoldenlifeError> {
        let client = Self::build(config, Jar::default())?;
        let form_url = client.url(LOGIN_PATH)?;
        let page = client
            .send(&|| client.request(Method::GET, &form_url, true))
            .await?;
        client.absorb(page.headers());
        let html = page.text().await.map_err(network)?;
        let token = parse_login_token(&html).ok_or(GoldenlifeError::InvalidResponse(
            "the login form has no CSRF token",
        ))?;

        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("_token", &token)
            .append_pair("Acesso[email]", login.email())
            .append_pair("Acesso[senha]", login.password())
            .append_pair("Acesso[lembrar]", "1")
            .finish();
        let origin = client.base_url.origin().ascii_serialization();
        let response = client
            .send(&|| {
                client
                    .request(Method::POST, &form_url, true)
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(ORIGIN, &origin)
                    .header(REFERER, form_url.as_str())
                    .body(body.clone())
            })
            .await?;
        client.absorb(response.headers());
        let status = response.status();
        if status.is_redirection() {
            let back_to_login = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|location| form_url.join(location).ok())
                .is_none_or(|next| next.path().starts_with(LOGIN_PATH));
            return if back_to_login {
                Err(GoldenlifeError::InvalidCredentials)
            } else {
                Ok(client)
            };
        }
        match status {
            StatusCode::OK => {
                let html = response.text().await.map_err(network)?;
                if is_login_page(&html) {
                    Err(GoldenlifeError::InvalidCredentials)
                } else {
                    Ok(client)
                }
            }
            s if s.as_u16() == 419 => Err(GoldenlifeError::InvalidResponse(
                "the login form expired, try again",
            )),
            other => Err(GoldenlifeError::HttpStatus(other.as_u16())),
        }
    }

    /// The cookies to persist for the next launch.
    pub fn session(&self) -> Result<Session, GoldenlifeError> {
        Session::new(&self.jar().header())
    }

    fn jar(&self) -> MutexGuard<'_, Jar> {
        self.jar.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn absorb(&self, headers: &HeaderMap) {
        self.jar().absorb(headers);
    }

    fn url(&self, path_and_query: &str) -> Result<Url, GoldenlifeError> {
        let url = self
            .base_url
            .join(path_and_query)
            .map_err(|_| GoldenlifeError::InvalidResponse("request path is invalid"))?;
        if url.origin() != self.base_url.origin() {
            return Err(GoldenlifeError::InvalidResponse(
                "request path leaves the academy",
            ));
        }
        Ok(url)
    }

    /// Request with browser headers; cookies only when `with_cookies`.
    fn request(&self, method: Method, url: &Url, with_cookies: bool) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .request(method, url.clone())
            .header(USER_AGENT, BROWSER_UA)
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/pdf,*/*;q=0.8",
            )
            .header(ACCEPT_LANGUAGE, "es-ES,es;q=0.9,en;q=0.8")
            .header(REFERER, self.referer());
        if with_cookies {
            let header = self.jar().header();
            if let Ok(mut value) = HeaderValue::from_str(&header) {
                value.set_sensitive(true);
                request = request.header(COOKIE, value);
            }
        }
        request
    }

    /// Sends with bounded retry on transport errors, 429 and 5xx. Redirects
    /// are returned as they come (the callers follow them).
    async fn send(
        &self,
        build: &(dyn Fn() -> reqwest::RequestBuilder + Send + Sync),
    ) -> Result<reqwest::Response, GoldenlifeError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let last = attempt >= self.retry.max_attempts;
            match build().send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let transient =
                        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                    if !transient || last {
                        return Ok(resp);
                    }
                    let hint = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                        .map(Duration::from_secs);
                    tokio::time::sleep(backoff(&self.retry, attempt, hint)).await;
                }
                Err(e) if last => return Err(network(e)),
                Err(_) => tokio::time::sleep(backoff(&self.retry, attempt, None)).await,
            }
        }
    }

    /// GET following redirects. Same-origin hops carry the cookies; a hop to
    /// another host is only followed when `allow_external` (file downloads)
    /// and then without cookies. A hop to the login page is `Unauthorized`.
    async fn get(
        &self,
        path_and_query: &str,
        allow_external: bool,
    ) -> Result<(reqwest::Response, Url), GoldenlifeError> {
        let mut url = self.url(path_and_query)?;
        for _ in 0..=MAX_REDIRECTS {
            let same_origin = url.origin() == self.base_url.origin();
            let resp = self
                .send(&|| self.request(Method::GET, &url, same_origin))
                .await?;
            if same_origin {
                self.absorb(resp.headers());
            }
            let status = resp.status();
            if status.is_redirection() {
                let next = resp
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|location| url.join(location).ok())
                    .ok_or(GoldenlifeError::InvalidResponse(
                        "redirect without a usable location",
                    ))?;
                if next.origin() == self.base_url.origin() {
                    if next.path().starts_with(LOGIN_PATH) {
                        return Err(GoldenlifeError::Unauthorized);
                    }
                } else if !(allow_external && next.scheme() == "https"
                    || allow_external && same_loopback(&next, &self.base_url))
                {
                    return Err(GoldenlifeError::InvalidResponse(
                        "redirected outside the academy",
                    ));
                }
                url = next;
                continue;
            }
            return match status {
                s if s.is_success() => Ok((resp, url)),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    Err(GoldenlifeError::Unauthorized)
                }
                s => Err(GoldenlifeError::HttpStatus(s.as_u16())),
            };
        }
        Err(GoldenlifeError::InvalidResponse("too many redirects"))
    }

    async fn page(&self, path_and_query: &str) -> Result<(String, Url), GoldenlifeError> {
        let (resp, url) = self.get(path_and_query, false).await?;
        let html = resp.text().await.map_err(network)?;
        if is_login_page(&html) {
            return Err(GoldenlifeError::Unauthorized);
        }
        Ok((html, url))
    }
}

/// Test servers answer on `localhost` and `127.0.0.1`: both loopback.
fn same_loopback(next: &Url, base: &Url) -> bool {
    let loopback = |url: &Url| {
        url.host_str().is_some_and(|host| {
            host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
        })
    };
    loopback(next) && loopback(base)
}

impl From<FetchError> for GoldenlifeError {
    fn from(error: FetchError) -> Self {
        match error {
            FetchError::Unauthorized(_) => Self::Unauthorized,
            FetchError::Http { status, .. } => Self::HttpStatus(status),
            FetchError::Network(message) => Self::Network(message),
            FetchError::Io(message) => Self::Io(message),
        }
    }
}

#[async_trait]
impl GoldenlifeApi for GoldenlifeClient {
    async fn validate(&self) -> Result<(), GoldenlifeError> {
        self.page("/area/vitrine/home").await.map(|_| ())
    }

    async fn courses(&self) -> Result<Vec<CourseInfo>, GoldenlifeError> {
        let (html, _) = self.page("/area/vitrine/home").await?;
        Ok(parse_home(&html))
    }

    async fn outline(&self, course_id: &str) -> Result<Outline, GoldenlifeError> {
        let course_id = numeric(course_id)?;
        let (html, _) = self
            .page(&format!("/area/conteudo/produto/{course_id}"))
            .await?;
        let mut outline = parse_outline(&html);
        if outline.product_id.as_deref() != Some(course_id) {
            // The course redirected to a lesson of another course: it has no
            // content of its own.
            outline.sections.clear();
        }
        Ok(outline)
    }

    async fn lesson(&self, lesson_id: &str) -> Result<LessonPage, GoldenlifeError> {
        let lesson_id = numeric(lesson_id)?;
        let (html, url) = self
            .page(&format!("/area/conteudo/aula/{lesson_id}"))
            .await?;
        if !url.path().starts_with("/area/conteudo/aula/") {
            return Err(GoldenlifeError::InvalidResponse(
                "the lesson is not available for this account",
            ));
        }
        Ok(parse_lesson(&html))
    }

    async fn download_attachment(&self, path: &str, dest: &Path) -> Result<(), GoldenlifeError> {
        let url = self.url(path)?;
        if url.path() != ATTACHMENT_PATH {
            return Err(GoldenlifeError::InvalidResponse(
                "attachment path is not an academy attachment",
            ));
        }
        let (resp, _) = self.get(path, true).await?;
        write_stream(resp, dest).await?;
        Ok(())
    }

    fn referer(&self) -> String {
        format!("{}/", self.base_url.origin().ascii_serialization())
    }
}

#[cfg(test)]
mod tests {
    use lumen_download::testsrv::{Recorded, Reply, TestServer};

    use super::*;

    const LOGIN_PAGE: &str = include_str!("../tests/fixtures/login.html");
    const LESSON_PAGE: &str = include_str!("../tests/fixtures/lesson_video.html");
    const HOME_PAGE: &str = include_str!("../tests/fixtures/home.html");

    fn config(server: &TestServer) -> ClientConfig {
        let mut config = ClientConfig::under(server.base());
        config.retry.base_delay = Duration::ZERO;
        config.retry.max_delay = Duration::ZERO;
        config
    }

    fn redirect(to: &str) -> Reply {
        Reply::status(302).with_header("location", to)
    }

    fn cookie_of(req: &Recorded) -> String {
        req.header("cookie").unwrap_or_default()
    }

    /// Academy double: the form sets a session cookie; a correct post sets
    /// the remember cookie and redirects home, a wrong one back to login.
    fn academy(password: &'static str) -> impl Fn(&Recorded, usize) -> Reply {
        move |req, _| match (req.method.as_str(), req.path_and_query.as_str()) {
            ("GET", "/auth/login") => Reply::html(LOGIN_PAGE).with_header(
                "set-cookie",
                "app_session=s1; Max-Age=7200; path=/; secure; httponly",
            ),
            ("POST", "/auth/login") if req.body.contains(&format!("senha%5D={password}&")) => {
                redirect("/area/vitrine/home")
                    .with_header(
                        "set-cookie",
                        "remember_cademi_x=long-lived; Max-Age=34560000; path=/",
                    )
                    .with_header("set-cookie", "app_session=s2; Max-Age=7200; path=/")
            }
            ("POST", "/auth/login") => redirect("/auth/login"),
            ("GET", "/area/vitrine/home") if cookie_of(req).contains("app_session=") => {
                Reply::html(HOME_PAGE)
            }
            ("GET", "/area/vitrine/home") => redirect("/auth/login"),
            _ => Reply::status(404),
        }
    }

    #[tokio::test]
    async fn login_posts_the_csrf_form_and_keeps_the_rotated_cookies() {
        let server = TestServer::start(academy("pw1")).await;
        let login = Login::new("ana@example.com", "pw1").unwrap();
        let client = GoldenlifeClient::login(&config(&server), &login)
            .await
            .unwrap();

        let post = server
            .requests()
            .into_iter()
            .find(|r| r.method == "POST")
            .unwrap();
        assert!(
            post.body.contains("_token=csrf-token-fixture-123"),
            "{}",
            post.body
        );
        assert!(post.body.contains("Acesso%5Bemail%5D=ana%40example.com"));
        assert!(post.body.contains("Acesso%5Blembrar%5D=1"));
        assert_eq!(
            post.header("content-type").as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert!(
            cookie_of(&post).contains("app_session=s1"),
            "the form cookie is sent back"
        );

        let stored = client.session().unwrap();
        assert_eq!(
            stored.cookie_header(),
            "app_session=s2; remember_cademi_x=long-lived"
        );
        client.validate().await.unwrap();
        let home = server.requests().into_iter().last().unwrap();
        assert!(cookie_of(&home).contains("app_session=s2"));
    }

    #[tokio::test]
    async fn a_wrong_password_is_reported_as_invalid_credentials() {
        let server = TestServer::start(academy("right")).await;
        let login = Login::new("ana@example.com", "wrong").unwrap();
        let err = GoldenlifeClient::login(&config(&server), &login)
            .await
            .unwrap_err();
        assert_eq!(err, GoldenlifeError::InvalidCredentials);
    }

    #[tokio::test]
    async fn a_login_form_without_token_is_an_invalid_response() {
        let server = TestServer::start(|_, _| Reply::html("<html>maintenance</html>")).await;
        let login = Login::new("ana@example.com", "pw").unwrap();
        let err = GoldenlifeClient::login(&config(&server), &login)
            .await
            .unwrap_err();
        assert!(
            matches!(err, GoldenlifeError::InvalidResponse(_)),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_stored_session_lists_courses_and_expiry_is_unauthorized() {
        let server = TestServer::start(academy("pw1")).await;
        let live = Session::new("app_session=abc; remember_cademi_x=r").unwrap();
        let client = GoldenlifeClient::new(&live, &config(&server)).unwrap();
        assert_eq!(client.courses().await.unwrap().len(), 3);
        assert_eq!(client.referer(), format!("{}/", server.base()));

        let dead = Session::new("remember_cademi_x=r").unwrap();
        let client = GoldenlifeClient::new(&dead, &config(&server)).unwrap();
        assert_eq!(
            client.courses().await.unwrap_err(),
            GoldenlifeError::Unauthorized
        );
        assert_eq!(
            client.validate().await.unwrap_err(),
            GoldenlifeError::Unauthorized
        );
    }

    #[tokio::test]
    async fn a_login_page_served_with_200_is_unauthorized() {
        let server = TestServer::start(|_, _| Reply::html(LOGIN_PAGE)).await;
        let client =
            GoldenlifeClient::new(&Session::new("a=b").unwrap(), &config(&server)).unwrap();
        assert_eq!(
            client.validate().await.unwrap_err(),
            GoldenlifeError::Unauthorized
        );
    }

    #[tokio::test]
    async fn a_course_outline_follows_the_redirect_to_its_first_lesson() {
        let server = TestServer::start(|req, _| match req.path_and_query.as_str() {
            "/area/conteudo/produto/574182" => redirect("/area/conteudo/aula/8184471"),
            "/area/conteudo/aula/8184471" => Reply::html(LESSON_PAGE),
            // An alias course lands on a lesson of another course.
            "/area/conteudo/produto/999" => redirect("/area/conteudo/aula/8184471"),
            _ => Reply::status(404),
        })
        .await;
        let client =
            GoldenlifeClient::new(&Session::new("a=b").unwrap(), &config(&server)).unwrap();

        let outline = client.outline("574182").await.unwrap();
        assert_eq!(outline.sections.len(), 2);
        assert_eq!(outline.sections[0].lessons[0].id, "8184471");

        let alias = client.outline("999").await.unwrap();
        assert!(
            alias.sections.is_empty(),
            "content of another course is not this course's"
        );
    }

    #[tokio::test]
    async fn a_lesson_page_is_parsed_and_ids_must_be_numeric() {
        let server = TestServer::start(|req, _| match req.path_and_query.as_str() {
            "/area/conteudo/aula/8184471" => Reply::html(LESSON_PAGE),
            "/area/conteudo/aula/7" => redirect("/area/vitrine/home"),
            "/area/vitrine/home" => Reply::html(HOME_PAGE),
            _ => Reply::status(404),
        })
        .await;
        let client =
            GoldenlifeClient::new(&Session::new("a=b").unwrap(), &config(&server)).unwrap();
        assert_eq!(
            client.lesson("8184471").await.unwrap().title,
            "Bienvenida \"Golden\""
        );
        assert!(matches!(
            client.lesson("7").await.unwrap_err(),
            GoldenlifeError::InvalidResponse(_)
        ));
        let before = server.count();
        for bad in ["", "12a", "../x", "1/2"] {
            assert!(client.lesson(bad).await.is_err(), "{bad:?}");
            assert!(client.outline(bad).await.is_err(), "{bad:?}");
        }
        assert_eq!(server.count(), before, "bad ids never reach the network");
    }

    #[tokio::test]
    async fn transient_server_errors_are_retried() {
        let server = TestServer::start(|_, n| {
            if n < 2 {
                Reply::status(503)
            } else {
                Reply::html(HOME_PAGE)
            }
        })
        .await;
        let client =
            GoldenlifeClient::new(&Session::new("a=b").unwrap(), &config(&server)).unwrap();
        assert_eq!(client.courses().await.unwrap().len(), 3);
        assert_eq!(server.count(), 3);
    }

    #[tokio::test]
    async fn pages_never_follow_a_redirect_to_another_host() {
        let server = TestServer::start(|_, _| redirect("https://evil.example.test/steal")).await;
        let client =
            GoldenlifeClient::new(&Session::new("a=b").unwrap(), &config(&server)).unwrap();
        let err = client.courses().await.unwrap_err();
        assert!(
            matches!(err, GoldenlifeError::InvalidResponse(_)),
            "{err:?}"
        );
        assert_eq!(server.count(), 1);
    }

    #[tokio::test]
    async fn an_attachment_is_streamed_and_cookies_do_not_follow_it_elsewhere() {
        let server = TestServer::start(|req, _| match req.path_and_query.as_str() {
            p if p.starts_with("/area/arquivo/anexo?")
                && req
                    .header("host")
                    .is_some_and(|h| h.starts_with("localhost")) =>
            {
                assert!(cookie_of(req).contains("secret=1"));
                redirect(&format!(
                    "http://127.0.0.1:{}/cdn/file.pdf",
                    req.header("host")
                        .unwrap_or_default()
                        .rsplit(':')
                        .next()
                        .unwrap_or("0")
                ))
            }
            "/cdn/file.pdf" => {
                assert!(
                    req.header("cookie").is_none(),
                    "cookies stay on the academy origin"
                );
                Reply::bytes(200, "application/pdf", b"%PDF-fixture".to_vec())
            }
            _ => Reply::status(404),
        })
        .await;
        let client =
            GoldenlifeClient::new(&Session::new("secret=1").unwrap(), &config(&server)).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("Materials/guia.pdf");
        client
            .download_attachment("/area/arquivo/anexo?a=1&signature=x", &dest)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-fixture");
        assert!(!tmp.path().join("Materials/guia.pdf.part").exists());
    }

    #[tokio::test]
    async fn only_academy_attachment_paths_can_be_downloaded() {
        let server = TestServer::start(|_, _| Reply::status(200)).await;
        let client =
            GoldenlifeClient::new(&Session::new("a=b").unwrap(), &config(&server)).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        for bad in [
            "https://evil.example.test/area/arquivo/anexo?x=1",
            "/area/conteudo/aula/1",
            "//evil.example.test/area/arquivo/anexo?x=1",
        ] {
            assert!(
                client
                    .download_attachment(bad, &tmp.path().join("f"))
                    .await
                    .is_err(),
                "{bad}"
            );
        }
        assert_eq!(server.count(), 0);
    }

    #[test]
    fn the_base_url_must_be_https_or_loopback() {
        let session = Session::new("a=b").unwrap();
        for ok in ["https://my.goldenlife-academy.com", "http://localhost:1234"] {
            assert!(
                GoldenlifeClient::new(&session, &ClientConfig::under(ok)).is_ok(),
                "{ok}"
            );
        }
        for bad in [
            "http://my.goldenlife-academy.com",
            "https://user:pw@my.goldenlife-academy.com",
            "https://my.goldenlife-academy.com/x",
            "not a url",
        ] {
            assert!(
                GoldenlifeClient::new(&session, &ClientConfig::under(bad)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn debug_output_hides_cookies_and_passwords() {
        let session = Session::new("remember_x=tok-secret").unwrap();
        let client = GoldenlifeClient::new(&session, &ClientConfig::default()).unwrap();
        let shown = format!("{client:?} {:?}", ClientConfig::default());
        assert!(!shown.contains("tok-secret"), "{shown}");
    }
}
