//! Authenticated Pluralsight API client with bounded retry and redacted errors.

use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use lumen_download::http::send_with_retry;
use lumen_download::{FetchError, RetryPolicy};
use reqwest::header::{
    HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, COOKIE, ORIGIN, REFERER, USER_AGENT,
};
use reqwest::Method;
use serde_json::Value;
use url::Url;

use crate::error::PluralsightError;
use crate::parse::{ClipDelivery, CourseInfo, ModuleInfo};

const API_BASE: &str = "https://app.pluralsight.com";
const USER_AGENT_VALUE: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
const HISTORY_PATH: [&str; 3] = ["learner", "user", "user-content-history"];
const DELIVERY_BODY: &str =
    r#"{"online":true,"boundedContext":"course","preferredAudioLanguage":"en"}"#;

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
            base_url: API_BASE.into(),
            connect_timeout: Duration::from_secs(15),
            request_timeout: Duration::from_secs(60),
            retry: RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::from_secs(3),
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
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("retry", &self.retry)
            .finish()
    }
}

#[async_trait]
pub trait PluralsightApi: Send + Sync {
    async fn validate(&self) -> Result<(), PluralsightError>;
    async fn courses(&self) -> Result<Vec<CourseInfo>, PluralsightError>;
    async fn toc(&self, course_id: &str) -> Result<Vec<ModuleInfo>, PluralsightError>;
    async fn clip_delivery(
        &self,
        clip_id: &str,
        version_id: &str,
    ) -> Result<ClipDelivery, PluralsightError>;
}

pub struct PluralsightClient {
    http: reqwest::Client,
    base_url: Url,
    origin: String,
    referer: String,
    cookies: HeaderValue,
    xsrf_token: Option<HeaderValue>,
    retry: RetryPolicy,
}

impl std::fmt::Debug for PluralsightClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluralsightClient")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl PluralsightClient {
    pub fn new(
        credentials: crate::Credentials,
        config: ClientConfig,
    ) -> Result<Self, PluralsightError> {
        let base_url = Url::parse(&config.base_url)
            .map_err(|_| PluralsightError::InvalidClientConfig("base URL is invalid"))?;
        let host = base_url
            .host_str()
            .ok_or(PluralsightError::InvalidClientConfig(
                "base URL has no host",
            ))?;
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
            return Err(PluralsightError::InvalidClientConfig(
                "base URL must be an HTTPS origin (HTTP is allowed only for loopback tests)",
            ));
        }

        let mut cookies = HeaderValue::from_str(credentials.cookie_header())
            .map_err(|_| PluralsightError::InvalidCredentials("cookie header is invalid".into()))?;
        cookies.set_sensitive(true);
        let xsrf_token = credentials
            .xsrf_token()
            .map(HeaderValue::from_str)
            .transpose()
            .map_err(|_| PluralsightError::InvalidCredentials("XSRF token is invalid".into()))?
            .map(|mut value| {
                value.set_sensitive(true);
                value
            });
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| PluralsightError::InvalidClientConfig("HTTP client could not be built"))?;
        let origin = base_url.origin().ascii_serialization();
        let referer = format!("{origin}/");
        Ok(Self {
            http,
            base_url,
            origin,
            referer,
            cookies,
            xsrf_token,
            retry: config.retry,
        })
    }

    fn url(&self, segments: &[&str]) -> Result<Url, PluralsightError> {
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
        {
            return Err(PluralsightError::InvalidResponse(
                "API path contains an invalid identifier",
            ));
        }
        let mut url = self.base_url.clone();
        let mut path = url
            .path_segments_mut()
            .map_err(|_| PluralsightError::InvalidClientConfig("base URL cannot contain paths"))?;
        path.clear();
        for segment in segments {
            path.push(segment);
        }
        drop(path);
        Ok(url)
    }

    fn request(
        &self,
        method: Method,
        url: Url,
        body: Option<&'static str>,
    ) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .request(method, url)
            .header(USER_AGENT, USER_AGENT_VALUE)
            .header(ACCEPT, "application/json, text/plain, */*")
            .header(ACCEPT_LANGUAGE, "pt-BR,pt;q=0.9")
            .header(ORIGIN, &self.origin)
            .header(REFERER, &self.referer)
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-dest", "empty")
            .header(COOKIE, self.cookies.clone());
        if let Some(xsrf_token) = &self.xsrf_token {
            request = request.header("x-xsrf-token", xsrf_token.clone());
        }
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .header("x-team", "video-services")
                .body(body.to_string());
        }
        request
    }

    async fn json(
        &self,
        method: Method,
        url: Url,
        body: Option<&'static str>,
    ) -> Result<Value, PluralsightError> {
        let url_text = url.as_str().to_string();
        let build = || self.request(method.clone(), url.clone(), body);
        let response = send_with_retry(&self.retry, false, &url_text, &build).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| PluralsightError::Network(error.without_url().to_string()))?;
        serde_json::from_slice(&bytes)
            .map_err(|_| PluralsightError::InvalidResponse("response body is not valid JSON"))
    }

    fn history_url(&self, limit: &str) -> Result<Url, PluralsightError> {
        let mut url = self.url(&HISTORY_PATH)?;
        url.query_pairs_mut()
            .append_pair("dataSource", "v2")
            .append_pair("limit", limit)
            .append_pair("labSort", "progress");
        Ok(url)
    }
}

#[async_trait]
impl PluralsightApi for PluralsightClient {
    async fn validate(&self) -> Result<(), PluralsightError> {
        let body = self.json(Method::GET, self.history_url("1")?, None).await?;
        crate::parse::parse_history(&body)?;
        Ok(())
    }

    async fn courses(&self) -> Result<Vec<CourseInfo>, PluralsightError> {
        let body = self
            .json(Method::GET, self.history_url("300")?, None)
            .await?;
        crate::parse::parse_history(&body)
    }

    async fn toc(&self, course_id: &str) -> Result<Vec<ModuleInfo>, PluralsightError> {
        let url = self.url(&[
            "course-player",
            "api",
            "v1",
            "table-of-contents",
            "course",
            course_id,
        ])?;
        let body = self.json(Method::GET, url, None).await?;
        crate::parse::parse_toc(&body)
    }

    async fn clip_delivery(
        &self,
        clip_id: &str,
        version_id: &str,
    ) -> Result<ClipDelivery, PluralsightError> {
        let url = self.url(&[
            "video", "delivery", "api", "v1", "clips", clip_id, "versions", version_id,
        ])?;
        let body = self.json(Method::POST, url, Some(DELIVERY_BODY)).await?;
        crate::parse::parse_clip_delivery(&body)
    }
}

impl From<FetchError> for PluralsightError {
    fn from(error: FetchError) -> Self {
        match error {
            FetchError::Unauthorized(status) => Self::Unauthorized(status),
            FetchError::Http { status, .. } => Self::HttpStatus(status),
            FetchError::Network(message) | FetchError::Io(message) => Self::Network(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use lumen_download::testsrv::{Reply, TestServer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    const COOKIES: &str = "session=fake-cookie; XSRF-TOKEN=fake-xsrf";
    const HISTORY: &str = include_str!("../tests/fixtures/history.json");
    const TOC: &str = include_str!("../tests/fixtures/toc.json");
    const DELIVERY: &str = include_str!("../tests/fixtures/delivery.json");

    fn client(server: &TestServer, attempts: u32) -> PluralsightClient {
        let mut config = ClientConfig::under(server.base());
        config.retry.max_attempts = attempts;
        config.retry.base_delay = Duration::ZERO;
        config.retry.max_delay = Duration::ZERO;
        PluralsightClient::new(crate::Credentials::new(COOKIES).unwrap(), config).unwrap()
    }

    #[test]
    fn defaults_use_production_https_and_mark_auth_headers_sensitive() {
        let config = ClientConfig::default();
        assert_eq!(config.base_url, API_BASE);
        assert_eq!(config.retry.max_attempts, 3);
        assert_eq!(config.retry.base_delay, Duration::from_secs(3));
        assert_eq!(config.retry.max_delay, Duration::from_secs(6));
        let custom = ClientConfig::under("https://custom.example.test");
        assert_eq!(custom.retry.base_delay, Duration::from_secs(3));
        assert_eq!(custom.retry.max_delay, Duration::from_secs(6));

        let client = PluralsightClient::new(
            crate::Credentials::new(COOKIES).unwrap(),
            ClientConfig::default(),
        )
        .unwrap();
        let request = client
            .request(Method::GET, client.history_url("1").unwrap(), None)
            .build()
            .unwrap();
        assert!(request.headers().get("cookie").unwrap().is_sensitive());
        assert!(request
            .headers()
            .get("x-xsrf-token")
            .unwrap()
            .is_sensitive());
    }

    #[tokio::test]
    async fn history_validation_courses_and_toc_use_expected_paths_and_headers() {
        let server = TestServer::start(|request, _| {
            if request.path_and_query.starts_with("/course-player/") {
                Reply::json(TOC)
            } else {
                Reply::json(HISTORY)
            }
        })
        .await;
        let client = client(&server, 1);
        let origin = server.base();
        let referer = format!("{origin}/");

        assert!(client.validate().await.is_ok());
        assert_eq!(client.courses().await.unwrap().len(), 3);
        assert_eq!(client.toc("course/a").await.unwrap().len(), 2);

        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests[0].path_and_query,
            "/learner/user/user-content-history?dataSource=v2&limit=1&labSort=progress"
        );
        assert_eq!(
            requests[1].path_and_query,
            "/learner/user/user-content-history?dataSource=v2&limit=300&labSort=progress"
        );
        assert_eq!(
            requests[2].path_and_query,
            "/course-player/api/v1/table-of-contents/course/course%2Fa"
        );
        for request in requests {
            assert_eq!(request.header("cookie").as_deref(), Some(COOKIES));
            assert_eq!(request.header("x-xsrf-token").as_deref(), Some("fake-xsrf"));
            assert_eq!(
                request.header("accept").as_deref(),
                Some("application/json, text/plain, */*")
            );
            assert_eq!(
                request.header("accept-language").as_deref(),
                Some("pt-BR,pt;q=0.9")
            );
            assert!(request.header("user-agent").unwrap().contains("Chrome/145"));
            assert_eq!(request.header("referer").as_deref(), Some(referer.as_str()));
            assert_eq!(request.header("origin").as_deref(), Some(origin.as_str()));
            assert_eq!(
                request.header("sec-fetch-site").as_deref(),
                Some("same-origin")
            );
        }
        assert!(!format!("{client:?}").contains("fake-cookie"));
        assert!(!format!("{client:?}").contains("fake-xsrf"));
    }

    #[tokio::test]
    async fn delivery_is_posted_with_escaped_ids_and_reference_body() {
        let (base, requests) = capture_server(DELIVERY).await;
        let mut config = ClientConfig::under(base);
        config.retry.max_attempts = 1;
        config.retry.base_delay = Duration::ZERO;
        config.retry.max_delay = Duration::ZERO;
        let client =
            PluralsightClient::new(crate::Credentials::new(COOKIES).unwrap(), config).unwrap();

        let result = client.clip_delivery("clip/id", "version 1").await.unwrap();
        assert!(result.stream_url.contains("/en.m3u8"));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(request.starts_with(
            "POST /video/delivery/api/v1/clips/clip%2Fid/versions/version%201 HTTP/1.1"
        ));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-team: video-services\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("content-type: application/json\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("cookie: session=fake-cookie; xsrf-token=fake-xsrf\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-xsrf-token: fake-xsrf\r\n"));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            serde_json::json!({
                "online": true,
                "boundedContext": "course",
                "preferredAudioLanguage": "en"
            })
        );
    }

    #[tokio::test]
    async fn auth_errors_are_typed_and_never_retried() {
        for status in [401, 403] {
            let server = TestServer::start(move |_, _| Reply::status(status)).await;
            assert_eq!(
                client(&server, 3).validate().await.unwrap_err(),
                PluralsightError::Unauthorized(status)
            );
            assert_eq!(server.count(), 1);
        }
    }

    #[tokio::test]
    async fn retries_server_and_rate_limit_errors_then_stops_after_three_attempts() {
        let server = TestServer::start(|_, attempt| match attempt {
            0 => Reply::status(500),
            1 => Reply::status(429),
            _ => Reply::json(r#"{"collection":[]}"#),
        })
        .await;
        assert!(client(&server, 3).validate().await.is_ok());
        assert_eq!(server.count(), 3);

        let server = TestServer::start(|_, _| Reply::status(503)).await;
        let error = client(&server, 3).validate().await.unwrap_err();
        assert_eq!(error, PluralsightError::HttpStatus(503));
        assert!(!error.to_string().contains(&server.base()));
        assert!(!error.to_string().contains("user-content-history"));
        assert_eq!(server.count(), 3);
    }

    #[tokio::test]
    async fn redirect_is_not_followed_and_credentials_do_not_reach_target() {
        let target = TestServer::start(|_, _| Reply::json(r#"{"collection":[]}"#)).await;
        let target_url = format!("{}/capture", target.base());
        let source =
            TestServer::start(move |_, _| Reply::status(302).with_header("location", &target_url))
                .await;
        let error = client(&source, 1).validate().await.unwrap_err();
        assert_eq!(error, PluralsightError::HttpStatus(302));
        assert_eq!(source.count(), 1);
        assert_eq!(target.count(), 0);
    }

    #[tokio::test]
    async fn malformed_json_and_client_configuration_errors_are_safe() {
        let server = TestServer::start(|_, _| Reply::json("fake-cookie fake-xsrf")).await;
        let error = client(&server, 1).validate().await.unwrap_err();
        assert!(matches!(error, PluralsightError::InvalidResponse(_)));
        assert!(!error.to_string().contains("fake-cookie"));
        assert!(!error.to_string().contains("fake-xsrf"));

        let config = ClientConfig::under("http://example.test");
        assert!(matches!(
            PluralsightClient::new(crate::Credentials::new(COOKIES).unwrap(), config),
            Err(PluralsightError::InvalidClientConfig(_))
        ));

        let config = ClientConfig::under("https://user:password@example.test/?token=fake-token");
        let debug = format!("{config:?}");
        assert!(!debug.contains("password"));
        assert!(!debug.contains("fake-token"));
    }

    async fn capture_server(body: &str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let body = body.as_bytes().to_vec();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let recorded = Arc::clone(&recorded);
                let body = body.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut chunk = [0_u8; 4096];
                    loop {
                        let Ok(read) = stream.read(&mut chunk).await else {
                            return;
                        };
                        if read == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..read]);
                        if request_complete(&bytes) {
                            break;
                        }
                    }
                    recorded
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(String::from_utf8_lossy(&bytes).into_owned());
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        (format!("http://localhost:{port}"), requests)
    }

    fn request_complete(bytes: &[u8]) -> bool {
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        bytes.len() >= header_end + 4 + length
    }
}
