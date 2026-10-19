//! Retrying HTTP helper with URL redaction and `.part` streaming.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use lumen_core::redact_url;
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// 401/403 (session expired, blocked or no permission).
    #[error("session expired or invalid (HTTP {0})")]
    Unauthorized(u16),
    #[error("HTTP {status} for {url}")]
    Http { status: u16, url: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("file error: {0}")]
    Io(String),
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
        }
    }
}

/// Delay before retry number `attempt` (1-based): exponential from
/// `base_delay`, or the server's `Retry-After` hint, capped at `max_delay`.
pub fn backoff(policy: &RetryPolicy, attempt: u32, hint: Option<Duration>) -> Duration {
    let exp = policy
        .base_delay
        .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)));
    hint.unwrap_or(exp).min(policy.max_delay)
}

/// Replaces `url` inside an error message with its redacted form.
pub fn redact_message(message: &str, url: &str) -> String {
    message.replace(url, &redact_url(url))
}

/// GET-style request with retry. Retries 429, 5xx, network errors and (when
/// `retry_403`) 403; 401 never retries. `build` creates a fresh request for
/// every attempt. Errors carry redacted URLs only.
pub async fn send_with_retry(
    policy: &RetryPolicy,
    retry_403: bool,
    url: &str,
    build: &(dyn Fn() -> reqwest::RequestBuilder + Send + Sync),
) -> Result<reqwest::Response, FetchError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let last = attempt >= policy.max_attempts;
        match build().send().await {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status();
                let retryable = status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
                    || (status == StatusCode::FORBIDDEN && retry_403);
                if !retryable || last {
                    return Err(match status.as_u16() {
                        s @ (401 | 403) => FetchError::Unauthorized(s),
                        s => FetchError::Http {
                            status: s,
                            url: redact_url(url),
                        },
                    });
                }
                let hint = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
                tokio::time::sleep(backoff(policy, attempt, hint)).await;
            }
            Err(e) => {
                if last {
                    return Err(FetchError::Network(redact_message(
                        &e.without_url().to_string(),
                        url,
                    )));
                }
                tokio::time::sleep(backoff(policy, attempt, None)).await;
            }
        }
    }
}

/// Streams the body to `<dest>.part` and renames it; empty bodies fail and
/// leave nothing behind.
pub async fn write_stream(resp: reqwest::Response, dest: &Path) -> Result<(), FetchError> {
    let io = |e: std::io::Error| FetchError::Io(e.to_string());
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(io)?;
    }
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let result = async {
        let mut file = tokio::fs::File::create(&part).await.map_err(io)?;
        let mut stream = resp.bytes_stream();
        let mut written = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| FetchError::Network(e.without_url().to_string()))?;
            written += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(io)?;
        }
        file.flush().await.map_err(io)?;
        if written == 0 {
            return Err(FetchError::Io("empty download".into()));
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => tokio::fs::rename(&part, dest).await.map_err(io),
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsrv::{Reply, TestServer};
    use reqwest::header::{HeaderName, HeaderValue};
    use std::time::Instant;

    fn fast(max_attempts: u32) -> RetryPolicy {
        RetryPolicy {
            max_attempts,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        }
    }

    async fn get(
        srv: &TestServer,
        policy: &RetryPolicy,
        retry_403: bool,
        path: &str,
    ) -> Result<reqwest::Response, FetchError> {
        let http = reqwest::Client::new();
        let url = format!("{}{path}", srv.base());
        let u = url.clone();
        send_with_retry(policy, retry_403, &url, &move || http.get(&u)).await
    }

    #[test]
    fn backoff_is_exponential_hinted_and_capped() {
        let p = RetryPolicy::default();
        assert_eq!(backoff(&p, 1, None), Duration::from_millis(500));
        assert_eq!(backoff(&p, 2, None), Duration::from_secs(1));
        assert_eq!(backoff(&p, 3, None), Duration::from_secs(2));
        assert_eq!(backoff(&p, 30, None), Duration::from_secs(8));
        assert_eq!(
            backoff(&p, 1, Some(Duration::from_secs(3))),
            Duration::from_secs(3)
        );
        assert_eq!(
            backoff(&p, 1, Some(Duration::from_secs(99))),
            Duration::from_secs(8)
        );
    }

    #[test]
    fn redact_message_hides_the_query_only() {
        assert_eq!(
            redact_message(
                "error sending https://h.test/a?token=S failed",
                "https://h.test/a?token=S"
            ),
            "error sending https://h.test/a failed"
        );
    }

    #[tokio::test]
    async fn rate_limits_and_server_errors_are_retried() {
        let srv = TestServer::start(|_, n| match n {
            0 => Reply::status(429),
            1 => Reply::status(503),
            _ => Reply::json("{}"),
        })
        .await;
        assert!(get(&srv, &fast(3), false, "/x").await.is_ok());
        assert_eq!(srv.count(), 3);
    }

    #[tokio::test]
    async fn retries_are_bounded_and_the_error_is_redacted() {
        let srv = TestServer::start(|_, _| Reply::status(500)).await;
        let err = get(&srv, &fast(3), false, "/x?token=SECRET")
            .await
            .unwrap_err();
        assert_eq!(
            err,
            FetchError::Http {
                status: 500,
                url: format!("{}/x", srv.base())
            }
        );
        assert_eq!(srv.count(), 3);
    }

    #[tokio::test]
    async fn unauthorized_never_retries_and_403_retries_only_on_request() {
        let srv = TestServer::start(|_, _| Reply::status(401)).await;
        assert_eq!(
            get(&srv, &fast(3), true, "/x").await.unwrap_err(),
            FetchError::Unauthorized(401)
        );
        assert_eq!(srv.count(), 1);
        let waf = TestServer::start(|_, _| Reply::status(403)).await;
        assert_eq!(
            get(&waf, &fast(3), false, "/x").await.unwrap_err(),
            FetchError::Unauthorized(403)
        );
        assert_eq!(waf.count(), 1);
        let waf = TestServer::start(|_, _| Reply::status(403)).await;
        assert!(get(&waf, &fast(3), true, "/x").await.is_err());
        assert_eq!(waf.count(), 3);
        let nf = TestServer::start(|_, _| Reply::status(404)).await;
        assert!(matches!(
            get(&nf, &fast(3), true, "/x").await.unwrap_err(),
            FetchError::Http { status: 404, .. }
        ));
        assert_eq!(nf.count(), 1);
    }

    #[tokio::test]
    async fn retry_after_is_honoured_within_the_cap() {
        let srv = TestServer::start(|_, n| {
            if n == 0 {
                Reply::status(429).with_header("retry-after", "0")
            } else {
                Reply::json("{}")
            }
        })
        .await;
        let t = Instant::now();
        assert!(get(&srv, &fast(2), false, "/x").await.is_ok());
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_fresh_request_is_built_for_every_attempt() {
        let srv = TestServer::start(|_, n| {
            if n == 0 {
                Reply::status(500)
            } else {
                Reply::json("{}")
            }
        })
        .await;
        let http = reqwest::Client::new();
        let url = format!("{}/x", srv.base());
        let calls = std::sync::atomic::AtomicU32::new(0);
        let u = url.clone();
        let build = || {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            http.get(&u)
                .header(HeaderName::from_static("x-attempt"), HeaderValue::from(n))
        };
        send_with_retry(&fast(3), false, &url, &build)
            .await
            .unwrap();
        let reqs = srv.requests();
        assert_eq!(reqs[0].header("x-attempt").as_deref(), Some("0"));
        assert_eq!(reqs[1].header("x-attempt").as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn stream_writes_through_part_and_renames() {
        let srv =
            TestServer::start(|_, _| Reply::bytes(200, "application/pdf", b"%PDF-1".to_vec()))
                .await;
        let resp = get(&srv, &fast(1), false, "/f").await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("nested/Guide.pdf");
        write_stream(resp, &dest).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1");
        assert!(!tmp.path().join("nested/Guide.pdf.part").exists());
    }

    #[tokio::test]
    async fn empty_bodies_fail_and_leave_nothing_behind() {
        let srv = TestServer::start(|_, _| Reply::bytes(200, "application/pdf", vec![])).await;
        let resp = get(&srv, &fast(1), false, "/f").await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("E.pdf");
        assert_eq!(
            write_stream(resp, &dest).await.unwrap_err(),
            FetchError::Io("empty download".into())
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }
}
