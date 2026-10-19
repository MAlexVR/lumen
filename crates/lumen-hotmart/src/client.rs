//! HTTP client for Hotmart: bearer-token auth, the exact headers the previous
//! Go implementation sent, retry/backoff, and credentials that never leave
//! Hotmart hosts.

use std::path::Path;
use std::time::Duration;

use lumen_download::http::{send_with_retry, write_stream};
use lumen_download::RetryPolicy;
use reqwest::header::{
    HeaderValue, ACCEPT, ACCEPT_LANGUAGE, AUTHORIZATION, CACHE_CONTROL, COOKIE, ORIGIN, REFERER,
    USER_AGENT,
};
use serde_json::Value;
use url::Url;

use crate::credentials::Credentials;
use crate::error::HotmartError;
use crate::parse::{
    parse_attachment_link, parse_check_token, parse_lesson, parse_navigation,
    parse_product_referer, parse_purchases, AttachmentLink, ClubResource, HotmartCourse,
    LessonData, Navigation, Purchases,
};

pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
pub const PURCHASE_API: &str = "https://api-hub.cb.hotmart.com/club-drive-api/rest/v2/purchase/";
pub const CHECK_TOKEN_API: &str = "https://api-sec-vlc.hotmart.com/security/oauth/check_token";
pub const NAVIGATION_API: &str =
    "https://api-club-course-consumption-gateway.hotmart.com/v1/navigation";
pub const LESSON_API: &str = "https://api-club-course-consumption-gateway.hotmart.com/v1/lesson";
pub const ATTACHMENT_API: &str = "https://api-club-hot-club-api.cb.hotmart.com/rest/v3/attachment";

/// Where each API lives (replaced by a loopback server in tests).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub check_token: String,
    /// Ends with `/`: product details are `<purchases>products/<id>`.
    pub purchases: String,
    pub navigation: String,
    pub lesson: String,
    pub attachment: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            check_token: CHECK_TOKEN_API.into(),
            purchases: PURCHASE_API.into(),
            navigation: NAVIGATION_API.into(),
            lesson: LESSON_API.into(),
            attachment: ATTACHMENT_API.into(),
        }
    }
}

impl Endpoints {
    /// Every API under one base URL (`/check_token`, `/purchase/`, ...).
    pub fn under(base: &str) -> Self {
        Self {
            check_token: format!("{base}/check_token"),
            purchases: format!("{base}/purchase/"),
            navigation: format!("{base}/navigation"),
            lesson: format!("{base}/lesson"),
            attachment: format!("{base}/attachment"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClientConfig {
    pub endpoints: Endpoints,
    pub retry: RetryPolicy,
}

pub struct HotmartClient {
    http: reqwest::Client,
    creds: Credentials,
    config: ClientConfig,
}

impl std::fmt::Debug for HotmartClient {
    // Deliberately omits the credentials.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotmartClient").finish_non_exhaustive()
    }
}

/// Which headers a request carries.
enum Kind<'a> {
    /// Consumer API (`Origin: consumer.hotmart.com`).
    Consumer,
    /// Course gateway and attachments: adds `slug` and `x-product-id`.
    Course {
        course: &'a HotmartCourse,
        attachment: bool,
    },
    /// Player pages and file hosts: browser headers only, never credentials.
    Public { referer: Option<&'static str> },
    /// DRM lambda: its own `token` plus the bearer token over https.
    Drm { token: &'a str },
}

/// Ids that end up inside a URL path must be plain.
fn path_segment(value: &str, what: &str) -> Result<(), HotmartError> {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && value != "."
        && value != "..";
    if plain {
        Ok(())
    } else {
        Err(HotmartError::Shape(format!("unexpected {what} format")))
    }
}

fn header_value(value: &str, what: &str) -> Result<HeaderValue, HotmartError> {
    HeaderValue::from_str(value)
        .map_err(|_| HotmartError::Shape(format!("{what} has invalid characters")))
}

impl HotmartClient {
    pub fn new(creds: Credentials) -> Result<Self, HotmartError> {
        Self::with_config(creds, ClientConfig::default())
    }

    pub fn with_config(creds: Credentials, config: ClientConfig) -> Result<Self, HotmartError> {
        // Fail early when the credentials cannot be sent as headers.
        header_value(creds.token(), "the token")?;
        if let Some(c) = creds.cookies() {
            header_value(c, "the cookies")?;
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| HotmartError::Network(e.without_url().to_string()))?;
        Ok(Self {
            http,
            creds,
            config,
        })
    }

    fn is_endpoint_host(&self, url: &Url) -> bool {
        let e = &self.config.endpoints;
        [
            &e.check_token,
            &e.purchases,
            &e.navigation,
            &e.lesson,
            &e.attachment,
        ]
        .into_iter()
        .filter_map(|u| Url::parse(u).ok())
        .any(|u| u.host_str() == url.host_str())
    }

    fn request(&self, url: &Url, kind: &Kind<'_>) -> Result<reqwest::RequestBuilder, HotmartError> {
        let mut req = self.http.get(url.clone()).header(USER_AGENT, BROWSER_UA);
        let bearer = || -> Result<HeaderValue, HotmartError> {
            let mut v = header_value(&format!("Bearer {}", self.creds.token()), "the token")?;
            v.set_sensitive(true);
            Ok(v)
        };
        let with_cookies = |mut req: reqwest::RequestBuilder| -> Result<_, HotmartError> {
            if let Some(c) = self.creds.cookies() {
                let mut v = header_value(c, "the cookies")?;
                v.set_sensitive(true);
                req = req.header(COOKIE, v);
            }
            Ok(req)
        };
        let browser = |req: reqwest::RequestBuilder, origin: &str, referer: &str, lang: &str| {
            req.header(ACCEPT, "application/json, text/plain, */*")
                .header(ORIGIN, origin)
                .header(REFERER, referer)
                .header("pragma", "no-cache")
                .header(CACHE_CONTROL, "no-cache")
                .header(ACCEPT_LANGUAGE, lang)
                .header("sec-fetch-site", "same-site")
                .header("sec-fetch-mode", "cors")
                .header("sec-fetch-dest", "empty")
        };
        match kind {
            Kind::Consumer => {
                req = browser(
                    req,
                    "https://consumer.hotmart.com",
                    "https://consumer.hotmart.com/",
                    "pt-BR,pt;q=0.9",
                );
                req = with_cookies(req.header(AUTHORIZATION, bearer()?))?;
            }
            Kind::Course { course, attachment } => {
                let (origin, referer, lang) = if *attachment {
                    (
                        "https://consumer.hotmart.com",
                        "https://consumer.hotmart.com/",
                        "pt-BR,pt;q=0.8,en-US;q=0.5,en;q=0.3",
                    )
                } else {
                    (
                        "https://hotmart.com",
                        "https://hotmart.com",
                        "pt-BR,pt;q=0.9",
                    )
                };
                req = browser(req, origin, referer, lang)
                    .header("slug", header_value(&course.slug, "the club slug")?)
                    .header("x-product-id", header_value(&course.id, "the product id")?);
                req = with_cookies(req.header(AUTHORIZATION, bearer()?))?;
            }
            Kind::Public { referer } => {
                if let Some(referer) = referer {
                    req = req.header(REFERER, *referer);
                }
            }
            Kind::Drm { token } => {
                req = req.header("token", header_value(token, "the DRM token")?);
                // The bearer token never travels over plain http to a foreign host.
                if url.scheme() == "https" || self.is_endpoint_host(url) {
                    req = req.header(AUTHORIZATION, bearer()?);
                }
            }
        }
        Ok(req)
    }

    async fn send(&self, url: &Url, kind: Kind<'_>) -> Result<reqwest::Response, HotmartError> {
        // Validates the headers once; the closure only repeats the build.
        drop(self.request(url, &kind)?);
        // The same inputs were just validated, so rebuilding cannot fail.
        let build = || {
            self.request(url, &kind)
                .unwrap_or_else(|_| self.http.get(url.clone()))
        };
        Ok(send_with_retry(&self.config.retry, false, url.as_str(), &build).await?)
    }

    async fn json(&self, url: &Url, kind: Kind<'_>) -> Result<Value, HotmartError> {
        let text = self.text(url, kind).await?;
        serde_json::from_str(&text).map_err(|e| HotmartError::Json(e.to_string()))
    }

    async fn text(&self, url: &Url, kind: Kind<'_>) -> Result<String, HotmartError> {
        let resp = self.send(url, kind).await?;
        resp.text()
            .await
            .map_err(|e| HotmartError::Network(e.without_url().to_string()))
    }

    fn url(raw: &str) -> Result<Url, HotmartError> {
        Url::parse(raw).map_err(|_| HotmartError::Shape("invalid URL".into()))
    }

    /// `check_token` resources (product to club subdomain).
    pub async fn check_token(&self) -> Result<Vec<ClubResource>, HotmartError> {
        let mut url = Self::url(&self.config.endpoints.check_token)?;
        url.query_pairs_mut()
            .append_pair("token", self.creds.token());
        Ok(parse_check_token(&self.json(&url, Kind::Consumer).await?))
    }

    async fn purchases(&self) -> Result<Value, HotmartError> {
        let mut url = Self::url(&self.config.endpoints.purchases)?;
        url.query_pairs_mut().append_pair("archived", "UNARCHIVED");
        self.json(&url, Kind::Consumer).await
    }

    /// Validates the credentials: `check_token`, or the purchases list when
    /// `check_token` fails (the original fallback). Returns the club resources
    /// when they are available.
    pub async fn validate(&self) -> Result<Vec<ClubResource>, HotmartError> {
        match self.check_token().await {
            Ok(clubs) => Ok(clubs),
            Err(_) => {
                let body = self.purchases().await?;
                parse_purchases(&body, &[])?;
                Ok(Vec::new())
            }
        }
    }

    /// Purchased products that have a members area. A failing `check_token`
    /// is tolerated (products then need their own club slug) except for an
    /// expired session.
    pub async fn courses(&self) -> Result<Purchases, HotmartError> {
        let clubs = match self.check_token().await {
            Ok(clubs) => clubs,
            Err(e @ HotmartError::Unauthorized(_)) => return Err(e),
            Err(_) => Vec::new(),
        };
        parse_purchases(&self.purchases().await?, &clubs)
    }

    /// Registration address of the product, used as the Referer for embedded
    /// players; `None` when Hotmart does not provide one.
    pub async fn product_referer(&self, product_id: &str) -> Result<Option<String>, HotmartError> {
        path_segment(product_id, "product id")?;
        let url = Self::url(&format!(
            "{}products/{product_id}",
            self.config.endpoints.purchases
        ))?;
        match self.json(&url, Kind::Consumer).await {
            Ok(body) => Ok(parse_product_referer(&body)),
            Err(e @ HotmartError::Unauthorized(_)) => Err(e),
            Err(_) => Ok(None),
        }
    }

    pub async fn navigation(&self, course: &HotmartCourse) -> Result<Navigation, HotmartError> {
        let url = Self::url(&self.config.endpoints.navigation)?;
        let body = self
            .json(
                &url,
                Kind::Course {
                    course,
                    attachment: false,
                },
            )
            .await?;
        parse_navigation(&body)
    }

    pub async fn lesson(
        &self,
        course: &HotmartCourse,
        hash: &str,
    ) -> Result<LessonData, HotmartError> {
        path_segment(hash, "lesson hash")?;
        let url = Self::url(&format!("{}/{hash}", self.config.endpoints.lesson))?;
        let body = self
            .json(
                &url,
                Kind::Course {
                    course,
                    attachment: false,
                },
            )
            .await?;
        parse_lesson(&body)
    }

    pub async fn attachment_link(
        &self,
        course: &HotmartCourse,
        attachment_id: &str,
    ) -> Result<AttachmentLink, HotmartError> {
        path_segment(attachment_id, "attachment id")?;
        let url = Self::url(&format!(
            "{}/{attachment_id}/download",
            self.config.endpoints.attachment
        ))?;
        let body = self
            .text(
                &url,
                Kind::Course {
                    course,
                    attachment: true,
                },
            )
            .await?;
        parse_attachment_link(&body)
    }

    /// DRM attachments: the lambda answers with the URL of the real file.
    pub async fn drm_file_url(
        &self,
        lambda_url: &str,
        token: &str,
    ) -> Result<String, HotmartError> {
        let url = Self::url(lambda_url)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(HotmartError::Shape("DRM lambda URL is not http(s)".into()));
        }
        let body = self.text(&url, Kind::Drm { token }).await?;
        let file = body.trim().trim_matches('"').trim();
        if file.starts_with("http://") || file.starts_with("https://") {
            Ok(file.to_string())
        } else {
            Err(HotmartError::Shape(
                "the DRM lambda did not return a file URL".into(),
            ))
        }
    }

    /// A native player page (HTML), fetched without credentials.
    pub async fn player_page(&self, url: &str) -> Result<String, HotmartError> {
        self.text(
            &Self::url(url)?,
            Kind::Public {
                referer: Some("https://hotmart.com"),
            },
        )
        .await
    }

    /// Streams a file to `dest` through a `.part` file, without credentials.
    pub async fn download_file(&self, url: &str, dest: &Path) -> Result<(), HotmartError> {
        let resp = self
            .send(&Self::url(url)?, Kind::Public { referer: None })
            .await?;
        write_stream(resp, dest).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_download::testsrv::{Recorded, Reply, TestServer};

    const CHECK: &str = include_str!("../tests/fixtures/check_token.json");
    const PURCHASES: &str = include_str!("../tests/fixtures/purchases.json");
    const NAV: &str = include_str!("../tests/fixtures/navigation.json");
    const LESSON: &str = include_str!("../tests/fixtures/lesson_full.json");
    const PRODUCT: &str = include_str!("../tests/fixtures/product_detail.json");
    const PLAYER: &str = include_str!("../tests/fixtures/player.html");
    const DRM: &str = include_str!("../tests/fixtures/attachment_drm.json");
    const DIRECT: &str = include_str!("../tests/fixtures/attachment_direct.json");
    const TOKEN: &str = "SECRETTOKEN.abc-123";
    const COOKIES: &str = "aws-waf-token=waf; extra=1";

    fn fast(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        }
    }

    fn client_with(srv: &TestServer, attempts: u32, cookies: Option<&str>) -> HotmartClient {
        let cfg = ClientConfig {
            endpoints: Endpoints::under(&srv.base()),
            retry: fast(attempts),
        };
        HotmartClient::with_config(Credentials::new(TOKEN, cookies).unwrap(), cfg).unwrap()
    }

    fn client(srv: &TestServer) -> HotmartClient {
        client_with(srv, 2, Some(COOKIES))
    }

    fn course() -> HotmartCourse {
        HotmartCourse {
            id: "1001".into(),
            name: "C".into(),
            slug: "horizonte-club".into(),
            seller: None,
            cover_url: None,
            purchase_date: None,
        }
    }

    fn api(req: &Recorded, _: usize) -> Reply {
        let p = req.path_and_query.as_str();
        if p.starts_with("/check_token") {
            Reply::json(CHECK)
        } else if p.starts_with("/purchase/products/") {
            Reply::json(PRODUCT)
        } else if p.starts_with("/purchase/") {
            Reply::json(PURCHASES)
        } else if p == "/navigation" {
            Reply::json(NAV)
        } else if p.starts_with("/lesson/") {
            Reply::json(LESSON)
        } else if p == "/attachment/direct/download" {
            Reply::json(DIRECT)
        } else if p == "/attachment/drm/download" {
            Reply::json(DRM)
        } else {
            Reply::status(404)
        }
    }

    #[tokio::test]
    async fn check_token_sends_the_bearer_token_and_the_original_headers() {
        let srv = TestServer::start(api).await;
        let r = client(&srv).check_token().await.unwrap();
        assert_eq!(r[0].subdomain, "horizonte-club");
        let req = &srv.requests()[0];
        assert_eq!(req.path_and_query, format!("/check_token?token={TOKEN}"));
        assert_eq!(
            req.header("authorization").as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(req.header("cookie").as_deref(), Some(COOKIES));
        assert_eq!(req.header("user-agent").as_deref(), Some(BROWSER_UA));
        assert_eq!(
            req.header("origin").as_deref(),
            Some("https://consumer.hotmart.com")
        );
        assert_eq!(
            req.header("referer").as_deref(),
            Some("https://consumer.hotmart.com/")
        );
        assert_eq!(
            req.header("accept-language").as_deref(),
            Some("pt-BR,pt;q=0.9")
        );
        assert_eq!(req.header("sec-fetch-site").as_deref(), Some("same-site"));
        assert_eq!(req.header("cache-control").as_deref(), Some("no-cache"));
    }

    #[tokio::test]
    async fn the_token_is_url_encoded_in_the_query() {
        let srv = TestServer::start(api).await;
        let cfg = ClientConfig {
            endpoints: Endpoints::under(&srv.base()),
            retry: fast(1),
        };
        let c =
            HotmartClient::with_config(Credentials::new("a+b/c=d", None).unwrap(), cfg).unwrap();
        c.check_token().await.unwrap();
        assert_eq!(
            srv.requests()[0].path_and_query,
            "/check_token?token=a%2Bb%2Fc%3Dd"
        );
        assert_eq!(
            srv.requests()[0].header("cookie"),
            None,
            "no cookies were supplied"
        );
    }

    #[tokio::test]
    async fn validate_uses_check_token_and_falls_back_to_the_purchases_list() {
        let srv = TestServer::start(api).await;
        assert_eq!(client(&srv).validate().await.unwrap().len(), 3);
        assert_eq!(srv.count(), 1, "check_token alone is enough");

        let srv = TestServer::start(|req, n| {
            if req.path_and_query.starts_with("/check_token") {
                Reply::status(500)
            } else {
                api(req, n)
            }
        })
        .await;
        assert!(client_with(&srv, 1, None)
            .validate()
            .await
            .unwrap()
            .is_empty());
        assert!(srv.requests()[1]
            .path_and_query
            .starts_with("/purchase/?archived=UNARCHIVED"));

        let srv = TestServer::start(|req, n| {
            if req.path_and_query.starts_with("/purchase/") {
                Reply::json(r#"{"message":"nope"}"#)
            } else {
                let _ = n;
                Reply::status(500)
            }
        })
        .await;
        assert!(
            client_with(&srv, 1, None).validate().await.is_err(),
            "no data array"
        );
    }

    #[tokio::test]
    async fn validate_reports_an_expired_token_as_unauthorized() {
        let srv = TestServer::start(|_, _| Reply::status(401)).await;
        assert_eq!(
            client(&srv).validate().await.unwrap_err(),
            HotmartError::Unauthorized(401)
        );
        assert_eq!(
            srv.count(),
            2,
            "check_token then the fallback, neither retried"
        );
    }

    #[tokio::test]
    async fn courses_combine_check_token_and_purchases() {
        let srv = TestServer::start(api).await;
        let p = client(&srv).courses().await.unwrap();
        assert_eq!(p.courses.len(), 4);
        assert_eq!(p.courses[1].slug, "flexibilidad");
        let reqs = srv.requests();
        assert!(reqs[1]
            .path_and_query
            .starts_with("/purchase/?archived=UNARCHIVED"));
    }

    #[tokio::test]
    async fn courses_survive_a_failing_check_token_but_not_an_expired_session() {
        let srv = TestServer::start(|req, n| {
            if req.path_and_query.starts_with("/check_token") {
                Reply::status(500)
            } else {
                api(req, n)
            }
        })
        .await;
        let p = client_with(&srv, 1, None).courses().await.unwrap();
        assert_eq!(
            p.courses.len(),
            3,
            "the product without its own slug needs check_token"
        );
        let expired = TestServer::start(|_, _| Reply::status(401)).await;
        assert_eq!(
            client(&expired).courses().await.unwrap_err(),
            HotmartError::Unauthorized(401)
        );
    }

    #[tokio::test]
    async fn the_product_referer_comes_from_the_product_detail() {
        let srv = TestServer::start(api).await;
        assert_eq!(
            client(&srv)
                .product_referer("1001")
                .await
                .unwrap()
                .as_deref(),
            Some("https://area.example.test/horizonte/")
        );
        assert_eq!(srv.requests()[0].path_and_query, "/purchase/products/1001");
        let missing = TestServer::start(|_, _| Reply::status(404)).await;
        assert_eq!(client(&missing).product_referer("1").await.unwrap(), None);
        let expired = TestServer::start(|_, _| Reply::status(401)).await;
        assert!(client(&expired).product_referer("1").await.is_err());
    }

    #[tokio::test]
    async fn course_calls_carry_the_slug_and_product_headers() {
        let srv = TestServer::start(api).await;
        let c = client(&srv);
        let nav = c.navigation(&course()).await.unwrap();
        assert_eq!(nav.modules.len(), 3);
        let lesson = c.lesson(&course(), "h-welcome").await.unwrap();
        assert!(lesson.has_player_media);
        for req in srv.requests() {
            assert_eq!(req.header("slug").as_deref(), Some("horizonte-club"));
            assert_eq!(req.header("x-product-id").as_deref(), Some("1001"));
            assert_eq!(req.header("origin").as_deref(), Some("https://hotmart.com"));
            assert_eq!(
                req.header("referer").as_deref(),
                Some("https://hotmart.com")
            );
            assert_eq!(
                req.header("authorization").as_deref(),
                Some(format!("Bearer {TOKEN}").as_str())
            );
        }
        assert_eq!(srv.requests()[1].path_and_query, "/lesson/h-welcome");
    }

    #[tokio::test]
    async fn lesson_api_messages_are_errors_and_statuses_are_mapped() {
        let srv = TestServer::start(|_, _| Reply::json(r#"{"message":"locked"}"#)).await;
        assert_eq!(
            client(&srv).lesson(&course(), "h").await.unwrap_err(),
            HotmartError::Api("locked".into())
        );
        let nf = TestServer::start(|_, _| Reply::status(404)).await;
        assert!(matches!(
            client(&nf).lesson(&course(), "h").await.unwrap_err(),
            HotmartError::Http { status: 404, .. }
        ));
        let bad = TestServer::start(|_, _| Reply::json("not json")).await;
        assert!(matches!(
            client(&bad).navigation(&course()).await.unwrap_err(),
            HotmartError::Json(_)
        ));
    }

    #[tokio::test]
    async fn attachment_links_use_the_attachment_headers() {
        let srv = TestServer::start(api).await;
        let c = client(&srv);
        let direct = c.attachment_link(&course(), "direct").await.unwrap();
        assert!(matches!(direct, AttachmentLink::Direct { .. }));
        let drm = c.attachment_link(&course(), "drm").await.unwrap();
        assert!(matches!(drm, AttachmentLink::Drm { .. }));
        let req = &srv.requests()[0];
        assert_eq!(req.path_and_query, "/attachment/direct/download");
        assert_eq!(
            req.header("accept-language").as_deref(),
            Some("pt-BR,pt;q=0.8,en-US;q=0.5,en;q=0.3")
        );
        assert_eq!(req.header("slug").as_deref(), Some("horizonte-club"));
        assert_eq!(req.header("x-product-id").as_deref(), Some("1001"));
    }

    #[tokio::test]
    async fn drm_files_are_resolved_through_the_lambda_with_token_and_bearer() {
        let srv = TestServer::start(|req, _| match req.path_and_query.as_str() {
            "/lambda" => Reply::bytes(
                200,
                "text/plain",
                b"  \"https://files.example.test/x.pdf\"\n".to_vec(),
            ),
            "/broken" => Reply::status(500),
            "/blank" => Reply::bytes(200, "text/plain", b"  ".to_vec()),
            _ => Reply::status(404),
        })
        .await;
        let c = client_with(&srv, 1, Some(COOKIES));
        let url = c
            .drm_file_url(&format!("{}/lambda", srv.base()), "DRMTOKEN")
            .await
            .unwrap();
        assert_eq!(url, "https://files.example.test/x.pdf");
        let req = &srv.requests()[0];
        assert_eq!(req.header("token").as_deref(), Some("DRMTOKEN"));
        assert_eq!(
            req.header("authorization").as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        let foreign = c
            .drm_file_url(&format!("{}/lambda", srv.foreign_base()), "DRMTOKEN")
            .await
            .unwrap();
        assert_eq!(foreign, "https://files.example.test/x.pdf");
        let req = &srv.requests()[1];
        assert_eq!(req.header("token").as_deref(), Some("DRMTOKEN"));
        assert_eq!(
            req.header("authorization"),
            None,
            "no bearer over http to a foreign host"
        );
        assert_eq!(req.header("cookie"), None);
        assert!(c
            .drm_file_url(&format!("{}/broken", srv.base()), "t")
            .await
            .is_err());
        assert!(matches!(
            c.drm_file_url(&format!("{}/blank", srv.base()), "t")
                .await
                .unwrap_err(),
            HotmartError::Shape(_)
        ));
    }

    #[tokio::test]
    async fn credentials_never_reach_foreign_hosts() {
        let srv = TestServer::start(|req, _| match req.path_and_query.as_str() {
            "/player" => Reply::html(PLAYER),
            _ => Reply::bytes(200, "application/pdf", b"%PDF".to_vec()),
        })
        .await;
        let c = client(&srv);
        let tmp = tempfile::tempdir().unwrap();
        let html = c
            .player_page(&format!("{}/player", srv.foreign_base()))
            .await
            .unwrap();
        assert!(html.contains("__NEXT_DATA__"));
        c.download_file(
            &format!("{}/f.pdf", srv.foreign_base()),
            &tmp.path().join("f.pdf"),
        )
        .await
        .unwrap();
        for req in srv.requests() {
            assert_eq!(req.header("authorization"), None);
            assert_eq!(req.header("cookie"), None);
            assert_eq!(req.header("user-agent").as_deref(), Some(BROWSER_UA));
        }
        assert_eq!(
            srv.requests()[0].header("referer").as_deref(),
            Some("https://hotmart.com")
        );
        assert_eq!(std::fs::read(tmp.path().join("f.pdf")).unwrap(), b"%PDF");
    }

    #[tokio::test]
    async fn downloads_leave_no_partial_or_empty_files() {
        let srv = TestServer::start(|req, _| match req.path_and_query.as_str() {
            "/empty" => Reply::bytes(200, "application/pdf", vec![]),
            _ => Reply::status(404),
        })
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let c = client(&srv);
        assert!(c
            .download_file(
                &format!("{}/empty", srv.base()),
                &tmp.path().join("a/e.pdf")
            )
            .await
            .is_err());
        assert!(c
            .download_file(
                &format!("{}/missing", srv.base()),
                &tmp.path().join("a/m.pdf")
            )
            .await
            .is_err());
        let left: Vec<_> = std::fs::read_dir(tmp.path().join("a")).unwrap().collect();
        assert!(left.is_empty());
    }

    #[tokio::test]
    async fn transient_errors_are_retried_and_auth_errors_are_not() {
        let srv = TestServer::start(|req, n| {
            if n < 2 {
                Reply::status(429)
            } else {
                api(req, n)
            }
        })
        .await;
        assert!(client_with(&srv, 3, None)
            .navigation(&course())
            .await
            .is_ok());
        assert_eq!(srv.count(), 3);
        let forbidden = TestServer::start(|_, _| Reply::status(403)).await;
        assert_eq!(
            client_with(&forbidden, 3, None)
                .navigation(&course())
                .await
                .unwrap_err(),
            HotmartError::Unauthorized(403)
        );
        assert_eq!(forbidden.count(), 1);
    }

    #[tokio::test]
    async fn errors_and_debug_output_never_contain_the_token() {
        let srv = TestServer::start(|_, _| Reply::status(500)).await;
        let c = client_with(&srv, 1, Some(COOKIES));
        assert!(!format!("{c:?}").contains("SECRETTOKEN"));
        let err = c.check_token().await.unwrap_err();
        assert!(!err.to_string().contains("SECRETTOKEN"), "{err}");
        assert_eq!(
            err,
            HotmartError::Http {
                status: 500,
                url: format!("{}/check_token", srv.base())
            }
        );
    }
}
