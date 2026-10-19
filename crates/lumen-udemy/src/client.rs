//! HTTP client for Udemy's web API. EVERYTHING here is a HYPOTHESIS (unverified
//! until a real diagnostic run): the endpoints, the query fields, the header
//! set and the bearer scheme. Adjust the constants below if the diagnostic
//! shows otherwise. Credentials are only sent to udemy.com hosts (or the
//! configured base host), retries cover 429/5xx/network, and errors never
//! carry tokens or URL queries.

use std::collections::HashSet;
use std::time::Duration;

use lumen_core::redact_url;
use lumen_download::http::{backoff, redact_message};
use lumen_download::RetryPolicy;
use reqwest::header::{
    HeaderValue, ACCEPT, ACCEPT_LANGUAGE, AUTHORIZATION, COOKIE, REFERER, USER_AGENT,
};
use reqwest::StatusCode;
use serde_json::Value;
use url::Url;

use crate::credentials::Credentials;
use crate::error::UdemyError;
use crate::parse::{parse_courses_page, CourseSummary};

pub const BASE_URL: &str = "https://www.udemy.com";
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";
/// HYPOTHESIS: the web app's own referer.
pub const REFERER_URL: &str = "https://www.udemy.com/home/my-courses/learning/";

/// HYPOTHESIS: `GET /api-2.0/users/me/subscribed-courses/` lists the owner's courses.
pub const COURSES_PATH: &str = "/api-2.0/users/me/subscribed-courses/";
pub const COURSES_FIELDS: &str =
    "fields[course]=title,image_480x270,url,num_published_lectures,visible_instructors";
/// HYPOTHESIS: curriculum items (chapters, lectures, quizzes, practices) of a course.
pub const CURRICULUM_PAGE_SIZE: u32 = 200;
pub const CURRICULUM_FIELDS: &str = "fields[lecture]=title,object_index,is_published,sort_order,asset,supplementary_assets,description&fields[quiz]=title,object_index,type&fields[chapter]=title,object_index&fields[asset]=title,filename,asset_type,status,time_estimation,is_external,stream_urls,download_urls,captions,media_sources,media_license_token,course_is_drmed";
/// HYPOTHESIS: per-lecture detail with the asset's media description.
pub const LECTURE_FIELDS: &str = "fields[lecture]=asset,description,download_url&fields[asset]=asset_type,stream_urls,download_urls,captions,title,filename,body,media_sources,media_license_token,course_is_drmed,external_url";

/// Upper bound of pages followed, a guard against endless pagination.
const MAX_PAGES: usize = 500;

/// Substrings that mark a 403 body as a bot/captcha challenge page.
const CHALLENGE_MARKERS: [&str; 7] = [
    "cf-chl",
    "just a moment",
    "captcha",
    "challenge-platform",
    "attention required",
    "_incapsula_",
    "px-captcha",
];

/// Where the API lives (a loopback server in tests).
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub base: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self::under(BASE_URL)
    }
}

/// Ids that end up inside a URL path must be plain.
fn path_segment(value: &str, what: &str) -> Result<(), UdemyError> {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if plain {
        Ok(())
    } else {
        Err(UdemyError::Shape(format!("unexpected {what} format")))
    }
}

impl Endpoints {
    pub fn under(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
        }
    }

    pub fn courses_url(&self, page_size: u32) -> String {
        format!(
            "{}{COURSES_PATH}?page_size={page_size}&{COURSES_FIELDS}",
            self.base
        )
    }

    pub fn curriculum_url(&self, course_id: &str) -> Result<String, UdemyError> {
        path_segment(course_id, "course id")?;
        Ok(format!(
            "{}/api-2.0/courses/{course_id}/subscriber-curriculum-items/?page_size={CURRICULUM_PAGE_SIZE}&{CURRICULUM_FIELDS}",
            self.base
        ))
    }

    pub fn lecture_url(&self, course_id: &str, lecture_id: &str) -> Result<String, UdemyError> {
        path_segment(course_id, "course id")?;
        path_segment(lecture_id, "lecture id")?;
        Ok(format!(
            "{}{COURSES_PATH}{course_id}/lectures/{lecture_id}/?{LECTURE_FIELDS}",
            self.base
        ))
    }

    fn host(&self) -> Option<String> {
        Url::parse(&self.base).ok()?.host_str().map(str::to_string)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClientConfig {
    pub endpoints: Endpoints,
    pub retry: RetryPolicy,
}

/// Pages fetched so far and the error that stopped pagination, if any.
#[derive(Debug)]
pub struct Pages {
    pub values: Vec<Value>,
    pub error: Option<UdemyError>,
}

pub struct UdemyClient {
    http: reqwest::Client,
    creds: Credentials,
    config: ClientConfig,
}

impl std::fmt::Debug for UdemyClient {
    // Deliberately omits the credentials.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdemyClient").finish_non_exhaustive()
    }
}

/// Credentials may only travel to udemy.com or the configured base host.
pub fn host_allowed(host: &str, base_host: &str) -> bool {
    host == base_host || host == "udemy.com" || host.ends_with(".udemy.com")
}

fn sensitive(value: &str) -> Option<HeaderValue> {
    let mut v = HeaderValue::from_str(value).ok()?;
    v.set_sensitive(true);
    Some(v)
}

fn looks_like_challenge(body: &str) -> bool {
    let lower = body.to_lowercase();
    CHALLENGE_MARKERS.iter().any(|m| lower.contains(m))
}

impl UdemyClient {
    pub fn new(creds: Credentials) -> Self {
        Self::with_config(creds, ClientConfig::default())
    }

    pub fn with_config(creds: Credentials, config: ClientConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            http,
            creds,
            config,
        }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.config.endpoints
    }

    fn request(&self, url: &Url) -> Result<reqwest::RequestBuilder, UdemyError> {
        let bearer = format!("Bearer {}", self.creds.access_token());
        let invalid = || UdemyError::InvalidCookies("the cookies cannot be sent as headers".into());
        // HYPOTHESIS: this header set; adjust here if the diagnostic says otherwise.
        Ok(self
            .http
            .get(url.clone())
            .header(USER_AGENT, BROWSER_UA)
            .header(ACCEPT, "application/json, text/plain, */*")
            .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .header(REFERER, REFERER_URL)
            .header("x-requested-with", "XMLHttpRequest")
            .header(AUTHORIZATION, sensitive(&bearer).ok_or_else(invalid)?)
            .header(
                "x-udemy-authorization",
                sensitive(&bearer).ok_or_else(invalid)?,
            )
            .header(
                COOKIE,
                sensitive(self.creds.cookie_header()).ok_or_else(invalid)?,
            ))
    }

    fn checked_url(&self, raw: &str) -> Result<Url, UdemyError> {
        let url = Url::parse(raw).map_err(|_| UdemyError::Shape("invalid URL".into()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(UdemyError::Shape("unsupported URL scheme".into()));
        }
        let host = url.host_str().unwrap_or_default();
        let base = self.config.endpoints.host().unwrap_or_default();
        if !host_allowed(host, &base) {
            return Err(UdemyError::Shape(format!(
                "refusing to send credentials to {host}"
            )));
        }
        Ok(url)
    }

    /// One JSON GET with retry on 429/5xx/network. 401/403/404 are final.
    pub async fn get_json(&self, raw_url: &str) -> Result<Value, UdemyError> {
        let url = self.checked_url(raw_url)?;
        let shown = redact_url(raw_url);
        let policy = &self.config.retry;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let last = attempt >= policy.max_attempts;
            match self.request(&url)?.send().await {
                Ok(resp) if resp.status().is_success() => {
                    let text = resp
                        .text()
                        .await
                        .map_err(|e| UdemyError::Network(e.without_url().to_string()))?;
                    return serde_json::from_str(&text)
                        .map_err(|e| UdemyError::Json(e.to_string()));
                }
                Ok(resp) => {
                    let status = resp.status();
                    let retryable =
                        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
                    if retryable && !last {
                        let hint = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.trim().parse::<u64>().ok())
                            .map(Duration::from_secs);
                        tokio::time::sleep(backoff(policy, attempt, hint)).await;
                        continue;
                    }
                    return Err(match status.as_u16() {
                        401 => UdemyError::Unauthorized,
                        403 => {
                            let body = resp.text().await.unwrap_or_default();
                            if looks_like_challenge(&body) {
                                UdemyError::BotChallenge
                            } else {
                                UdemyError::Forbidden
                            }
                        }
                        404 => UdemyError::NotFound { url: shown },
                        s => UdemyError::Http {
                            status: s,
                            url: shown,
                        },
                    });
                }
                Err(e) => {
                    let message = redact_message(&e.without_url().to_string(), raw_url);
                    if last {
                        return Err(UdemyError::Network(message));
                    }
                    tokio::time::sleep(backoff(policy, attempt, None)).await;
                }
            }
        }
    }

    /// Follows `next` links from `first_url`, keeping every page fetched even
    /// when a later one fails.
    pub async fn pages(&self, first_url: &str) -> Pages {
        let mut values = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(first_url.to_string());
        while let Some(url) = next.take() {
            if values.len() >= MAX_PAGES || !seen.insert(url.clone()) {
                return Pages {
                    values,
                    error: Some(UdemyError::Shape(
                        "pagination does not end (loop or too many pages)".into(),
                    )),
                };
            }
            match self.get_json(&url).await {
                Ok(page) => {
                    next = page
                        .get("next")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty())
                        .map(str::to_string);
                    values.push(page);
                }
                Err(error) => {
                    return Pages {
                        values,
                        error: Some(error),
                    }
                }
            }
        }
        Pages {
            values,
            error: None,
        }
    }

    /// Minimal request proving the session works (`page_size=1`); returns the
    /// total course count when the response says it.
    pub async fn verify_session(&self) -> Result<Option<u64>, UdemyError> {
        let page = self.get_json(&self.config.endpoints.courses_url(1)).await?;
        Ok(parse_courses_page(&page)?.count)
    }

    /// Every course of the account, plus how many entries could not be read.
    pub async fn courses(&self) -> Result<(Vec<CourseSummary>, usize), UdemyError> {
        let pages = self.pages(&self.config.endpoints.courses_url(100)).await;
        if let Some(error) = pages.error {
            return Err(error);
        }
        let (mut courses, mut skipped) = (Vec::new(), 0);
        for page in &pages.values {
            let parsed = parse_courses_page(page)?;
            skipped += parsed.skipped;
            courses.extend(parsed.courses);
        }
        Ok((courses, skipped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_download::testsrv::{Recorded, Reply, TestServer};
    use serde_json::json;
    use std::time::Duration;

    const TOKEN: &str = "TOKENSECRET1";

    trait WithStatus {
        fn with_status(self, status: u16) -> Self;
    }

    impl WithStatus for Reply {
        fn with_status(mut self, status: u16) -> Self {
            self.status = status;
            self
        }
    }

    fn creds() -> Credentials {
        Credentials::from_cookie_text(&format!("client_id=cid12345; access_token={TOKEN}")).unwrap()
    }

    fn fast() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        }
    }

    fn client(base: &str) -> UdemyClient {
        UdemyClient::with_config(
            creds(),
            ClientConfig {
                endpoints: Endpoints::under(base),
                retry: fast(),
            },
        )
    }

    fn page(base: &str, ids: &[u64], next: Option<&str>) -> Reply {
        let results: Vec<_> = ids
            .iter()
            .map(|i| json!({"id": i, "title": format!("C{i}")}))
            .collect();
        Reply::json(
            &json!({"count": 3, "next": next.map(|n| format!("{base}{n}")), "results": results})
                .to_string(),
        )
    }

    #[test]
    fn endpoint_urls_follow_the_hypothesised_shapes_and_validate_ids() {
        let e = Endpoints::under("https://www.udemy.com");
        let u = e.courses_url(100);
        assert!(u.starts_with(
            "https://www.udemy.com/api-2.0/users/me/subscribed-courses/?page_size=100"
        ));
        assert!(u.contains(
            "fields[course]=title,image_480x270,url,num_published_lectures,visible_instructors"
        ));
        let c = e.curriculum_url("1001").unwrap();
        assert!(c.starts_with(
            "https://www.udemy.com/api-2.0/courses/1001/subscriber-curriculum-items/?page_size=200"
        ));
        assert!(
            c.contains("fields[asset]=")
                && c.contains("media_license_token")
                && c.contains("fields[quiz]=")
        );
        let l = e.lecture_url("1001", "11").unwrap();
        assert!(l.starts_with("https://www.udemy.com/api-2.0/users/me/subscribed-courses/1001/lectures/11/?fields[lecture]=asset"));
        for bad in ["", "../x", "a/b", "1?x=2", "a b"] {
            assert!(
                matches!(e.curriculum_url(bad), Err(UdemyError::Shape(_))),
                "{bad}"
            );
            assert!(
                matches!(e.lecture_url("1", bad), Err(UdemyError::Shape(_))),
                "{bad}"
            );
        }
        assert_eq!(Endpoints::default().base, "https://www.udemy.com");
    }

    #[tokio::test]
    async fn the_session_check_sends_browser_headers_and_the_bearer_token() {
        let srv =
            TestServer::start(|_, _| Reply::json(r#"{"count": 12, "next": null, "results": []}"#))
                .await;
        let n = client(&srv.base()).verify_session().await.unwrap();
        assert_eq!(n, Some(12));
        let req = &srv.requests()[0];
        assert!(
            req.path_and_query.contains("page_size=1&")
                || req.path_and_query.contains("page_size=1")
        );
        assert_eq!(
            req.header("authorization").as_deref(),
            Some(&*format!("Bearer {TOKEN}"))
        );
        assert_eq!(
            req.header("x-udemy-authorization").as_deref(),
            Some(&*format!("Bearer {TOKEN}"))
        );
        assert_eq!(
            req.header("cookie").as_deref(),
            Some(creds().cookie_header())
        );
        assert!(req.header("user-agent").unwrap().contains("Mozilla"));
        assert_eq!(
            req.header("x-requested-with").as_deref(),
            Some("XMLHttpRequest")
        );
    }

    #[tokio::test]
    async fn pagination_follows_next_until_the_end_and_counts_skipped_entries() {
        let srv = TestServer::start(|req: &Recorded, _| {
            let base = format!(
                "http://localhost:{}",
                req.header("host").unwrap().rsplit(':').next().unwrap()
            );
            if req.path_and_query.contains("page=2") {
                Reply::json(
                    &json!({"next": null, "results": [{"id": 3}, {"title": "no id"}]}).to_string(),
                )
            } else {
                page(
                    &base,
                    &[1, 2],
                    Some("/api-2.0/users/me/subscribed-courses/?page=2"),
                )
            }
        })
        .await;
        let (courses, skipped) = client(&srv.base()).courses().await.unwrap();
        let ids: Vec<_> = courses.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["1", "2", "3"]);
        assert_eq!(skipped, 1);
        assert_eq!(srv.count(), 2);
    }

    #[tokio::test]
    async fn a_repeating_next_link_cannot_loop_forever() {
        let srv = TestServer::start(|req: &Recorded, _| {
            let base = format!(
                "http://localhost:{}",
                req.header("host").unwrap().rsplit(':').next().unwrap()
            );
            page(&base, &[1], Some("/same"))
        })
        .await;
        let pages = client(&srv.base())
            .pages(&format!("{}/same", srv.base()))
            .await;
        assert_eq!(pages.values.len(), 1);
        assert!(matches!(pages.error, Some(UdemyError::Shape(m)) if m.contains("loop")));
    }

    #[tokio::test]
    async fn status_codes_map_to_typed_errors_without_retrying_the_final_ones() {
        let srv = TestServer::start(|req: &Recorded, _| match req.path_and_query.as_str() {
            p if p.starts_with("/401") => Reply::status(401),
            p if p.starts_with("/403c") => Reply::html(
                "<html><title>Just a moment...</title><div id=\"cf-chl-widget\"></div></html>",
            )
            .with_status(403),
            p if p.starts_with("/403") => Reply::json(r#"{"detail":"nope"}"#).with_status(403),
            p if p.starts_with("/404") => Reply::status(404),
            p if p.starts_with("/418") => Reply::status(418),
            p if p.starts_with("/bad") => Reply::json("<<not json SECRETBODY>>"),
            _ => Reply::status(500),
        })
        .await;
        let c = client(&srv.base());
        let get = |p: &str| {
            let url = format!("{}{p}", srv.base());
            let c = &c;
            async move { c.get_json(&url).await }
        };
        assert_eq!(get("/401").await.unwrap_err(), UdemyError::Unauthorized);
        assert_eq!(get("/403c").await.unwrap_err(), UdemyError::BotChallenge);
        assert_eq!(get("/403").await.unwrap_err(), UdemyError::Forbidden);
        assert!(matches!(
            get("/404").await.unwrap_err(),
            UdemyError::NotFound { .. }
        ));
        assert!(matches!(
            get("/418").await.unwrap_err(),
            UdemyError::Http { status: 418, .. }
        ));
        let bad = get("/bad").await.unwrap_err();
        assert!(matches!(bad, UdemyError::Json(_)));
        assert!(!bad.to_string().contains("SECRETBODY"));
        let reqs = srv.requests();
        let hits = |p: &str| {
            reqs.iter()
                .filter(|r| r.path_and_query.starts_with(p))
                .count()
        };
        assert_eq!(
            (hits("/401"), hits("/403"), hits("/404"), hits("/418")),
            (1, 2, 1, 1)
        );
        assert_eq!(
            get("/500").await.unwrap_err(),
            UdemyError::Http {
                status: 500,
                url: format!("{}/500", srv.base())
            }
        );
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_are_retried_with_backoff() {
        let srv = TestServer::start(|_, n| match n {
            0 => Reply::status(429).with_header("retry-after", "0"),
            1 => Reply::status(503),
            _ => Reply::json(r#"{"results": []}"#),
        })
        .await;
        let v = client(&srv.base())
            .get_json(&format!("{}/x", srv.base()))
            .await
            .unwrap();
        assert_eq!(v["results"], json!([]));
        assert_eq!(srv.count(), 3);
        let always = TestServer::start(|_, _| Reply::status(500)).await;
        let err = client(&always.base())
            .get_json(&format!("{}/x", always.base()))
            .await
            .unwrap_err();
        assert!(matches!(err, UdemyError::Http { status: 500, .. }));
        assert_eq!(always.count(), 3);
    }

    #[tokio::test]
    async fn the_cookie_and_token_never_go_to_a_foreign_host() {
        let srv = TestServer::start(|_, _| Reply::json(r#"{"results": []}"#)).await;
        let c = client(&srv.base());
        let err = c
            .get_json(&format!("{}/x", srv.foreign_base()))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, UdemyError::Shape(m) if m.contains("127.0.0.1")),
            "{err:?}"
        );
        assert_eq!(srv.count(), 0, "nothing was sent");
        let err = c.get_json("https://evil.example.com/x").await.unwrap_err();
        assert!(matches!(err, UdemyError::Shape(_)));
        let err = c.get_json("ftp://www.udemy.com/x").await.unwrap_err();
        assert!(matches!(err, UdemyError::Shape(_)));
        let err = c
            .get_json("https://udemy.com.evil.com/x")
            .await
            .unwrap_err();
        assert!(matches!(err, UdemyError::Shape(_)));
        assert!(
            host_allowed("www.udemy.com", "x")
                && host_allowed("udemy.com", "x")
                && host_allowed("api.udemy.com", "x")
        );
        assert!(!host_allowed("notudemy.com", "x") && host_allowed("same.test", "same.test"));
    }

    #[tokio::test]
    async fn a_next_link_to_another_host_stops_pagination_with_the_pages_so_far() {
        let srv = TestServer::start(|req: &Recorded, _| {
            let port = req
                .header("host")
                .unwrap()
                .rsplit(':')
                .next()
                .unwrap()
                .to_string();
            Reply::json(
                &json!({"next": format!("http://127.0.0.1:{port}/p2"), "results": [{"id": 1}]})
                    .to_string(),
            )
        })
        .await;
        let pages = client(&srv.base())
            .pages(&format!("{}/p1", srv.base()))
            .await;
        assert_eq!(pages.values.len(), 1);
        assert!(matches!(pages.error, Some(UdemyError::Shape(_))));
        assert_eq!(srv.count(), 1);
    }

    #[tokio::test]
    async fn network_failures_are_retried_and_reported_without_the_url_query() {
        let c = client("http://127.0.0.1:1");
        let err = c
            .get_json("http://127.0.0.1:1/x?token=SECRETQ")
            .await
            .unwrap_err();
        assert!(matches!(&err, UdemyError::Network(_)));
        assert!(!err.to_string().contains("SECRETQ"));
    }

    #[test]
    fn debug_never_shows_credentials_and_errors_label_their_status() {
        let c = client("http://localhost:1");
        let shown = format!("{c:?}");
        assert!(
            !shown.contains(TOKEN) && !shown.contains("cid12345"),
            "{shown}"
        );
        assert_eq!(UdemyError::Unauthorized.status_label(), "http_401");
        assert_eq!(UdemyError::BotChallenge.status_label(), "http_403");
        assert_eq!(UdemyError::Json("x".into()).status_label(), "parse_error");
        assert_eq!(UdemyError::Network("x".into()).status_label(), "network");
        assert_eq!(
            UdemyError::Http {
                status: 502,
                url: String::new()
            }
            .status_label(),
            "http_502"
        );
    }
}
