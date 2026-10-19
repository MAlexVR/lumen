use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{ALLOW, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::confine::resolve_confined;
use crate::error::MediaError;

const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

struct Shared {
    root: RwLock<PathBuf>,
    /// Exact `Host` header value accepted (`127.0.0.1:<port>`).
    host: String,
    /// `/m/<token>/`
    prefix: String,
    /// `/t/<token>/`: generated thumbnails, from `thumb_dir` only.
    thumb_prefix: String,
    thumb_dir: RwLock<Option<PathBuf>>,
}

/// Loopback media server. Dropping it also stops the listener.
pub struct MediaServer {
    addr: SocketAddr,
    token: String,
    shared: Arc<Shared>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl MediaServer {
    /// Binds `127.0.0.1:0` and serves files under `root`.
    pub async fn start(root: impl Into<PathBuf>) -> Result<Self, MediaError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(MediaError::Bind)?;
        let addr = listener.local_addr().map_err(MediaError::Bind)?;
        let token = random_token()?;
        let shared = Arc::new(Shared {
            root: RwLock::new(root.into()),
            host: addr.to_string(),
            prefix: format!("/m/{token}/"),
            thumb_prefix: format!("/t/{token}/"),
            thumb_dir: RwLock::new(None),
        });
        let app = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&shared));
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await;
        });
        Ok(Self {
            addr,
            token,
            shared,
            stop: Some(stop),
            task,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// URL prefix to which a percent-encoded root-relative path is appended
    /// after a `/`.
    pub fn base_url(&self) -> String {
        format!("http://{}/m/{}", self.addr, self.token)
    }

    /// URL prefix of the thumbnail route; append `/<hash>.jpg`.
    pub fn thumb_base_url(&self) -> String {
        format!("http://{}/t/{}", self.addr, self.token)
    }

    /// Folder the thumbnail route serves from (only `<64 hex>.jpg` files
    /// directly inside it).
    pub fn set_thumb_dir(&self, dir: impl AsRef<Path>) {
        let mut guard = self
            .shared
            .thumb_dir
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(dir.as_ref().to_path_buf());
    }

    /// Changes the served library root; applies to the next request.
    pub fn set_root(&self, root: impl AsRef<Path>) {
        let mut guard = self.shared.root.write().unwrap_or_else(|e| e.into_inner());
        *guard = root.as_ref().to_path_buf();
    }

    /// Stops accepting connections and waits briefly for in-flight requests.
    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
    }
}

fn random_token() -> Result<String, MediaError> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| MediaError::Token(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Length-dependent but content-independent comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn handle(State(shared): State<Arc<Shared>>, req: Request) -> Response {
    let mut res = respond(&shared, req).await;
    res.headers_mut()
        .entry(CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    res
}

async fn respond(shared: &Shared, req: Request) -> Response {
    let host_ok = req
        .headers()
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| h == shared.host);
    if !host_ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = req.uri().path();
    let is_thumb = path
        .as_bytes()
        .get(..shared.thumb_prefix.len())
        .is_some_and(|head| constant_time_eq(head, shared.thumb_prefix.as_bytes()));
    if is_thumb {
        return respond_thumb(shared, req).await;
    }
    let token_ok = path
        .as_bytes()
        .get(..shared.prefix.len())
        .is_some_and(|head| constant_time_eq(head, shared.prefix.as_bytes()));
    if !token_ok || path.len() == shared.prefix.len() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let mut res = StatusCode::METHOD_NOT_ALLOWED.into_response();
        res.headers_mut()
            .insert(ALLOW, HeaderValue::from_static("GET, HEAD"));
        return res;
    }
    let Ok(rel) = percent_encoding::percent_decode_str(&path[shared.prefix.len()..]).decode_utf8()
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let root = shared
        .root
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let rel = rel.into_owned();
    let resolved = tokio::task::spawn_blocking(move || resolve_confined(&root, &rel))
        .await
        .ok()
        .flatten();
    let Some(full) = resolved else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = content_type_for(&full);
    let Ok(served) = ServeFile::new(&full).oneshot(req).await;
    let mut res = served.map(Body::new);
    if res.status().is_success() {
        let headers = res.headers_mut();
        if let Some(ct) = content_type {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static(ct));
        }
        headers.insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
        if content_type.is_some_and(|c| c.starts_with("text/html")) {
            headers.insert(
                CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("script-src 'none'"),
            );
        }
    }
    res
}

/// `<64 lowercase hex>.jpg`: the only names the cache folder may serve.
fn is_cache_name(name: &str) -> bool {
    name.strip_suffix(".jpg").is_some_and(|stem| {
        stem.len() == 64
            && stem
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

async fn respond_thumb(shared: &Shared, req: Request) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let mut res = StatusCode::METHOD_NOT_ALLOWED.into_response();
        res.headers_mut()
            .insert(ALLOW, HeaderValue::from_static("GET, HEAD"));
        return res;
    }
    let Ok(name) =
        percent_encoding::percent_decode_str(&req.uri().path()[shared.thumb_prefix.len()..])
            .decode_utf8()
            .map(|n| n.into_owned())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let dir = shared
        .thumb_dir
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(dir) = dir.filter(|_| is_cache_name(&name)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = dir.join(&name);
    // A symlink or anything but a regular file is never served.
    let regular = tokio::fs::symlink_metadata(&file)
        .await
        .is_ok_and(|m| m.is_file());
    if !regular {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(served) = ServeFile::new(&file).oneshot(req).await;
    let mut res = served.map(Body::new);
    if res.status().is_success() {
        let headers = res.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
        headers.insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=86400"));
    }
    res
}

fn content_type_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "opus" | "ogg" => "audio/ogg",
        "aac" => "audio/aac",
        "html" => "text/html; charset=utf-8",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" => "text/plain; charset=utf-8",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_downloads_have_audio_content_types() {
        for (file, want) in [
            ("a.m4a", "audio/mp4"),
            ("a.mp3", "audio/mpeg"),
            ("a.opus", "audio/ogg"),
            ("a.ogg", "audio/ogg"),
            ("a.aac", "audio/aac"),
        ] {
            assert_eq!(content_type_for(Path::new(file)), Some(want), "{file}");
        }
    }
}
