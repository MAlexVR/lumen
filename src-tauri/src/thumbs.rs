//! Thumbnail commands' application layer: confined requests against the
//! current library root, replies with loopback URLs and a `thumbnails://ready`
//! event per finished item. It holds no Tauri types (the event goes through a
//! callback), so it is tested without a window.

use std::path::PathBuf;
use std::sync::Arc;

use lumen_media::{ThumbConfig, ThumbDone, ThumbItem, ThumbReply, ThumbService, ThumbState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::AppCore;

/// Event name the UI listens to.
pub const READY_EVENT: &str = "thumbnails://ready";

/// Sends an event (name, payload); the app shell forwards it to the webview.
pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbItemArg {
    /// Library-relative path of the video.
    pub path: String,
    /// Moment to capture; absent or null means the default (8 s).
    #[serde(default)]
    pub at_secs: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbsRequest {
    pub items: Vec<ThumbItemArg>,
    #[serde(default)]
    pub width: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbInfo {
    pub path: String,
    pub at_secs: f64,
    pub state: ThumbState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbsStatus {
    pub ffmpeg_available: bool,
    pub cache_bytes: u64,
    pub queued: usize,
    pub running: usize,
}

fn url_of(base: &str, file: &str) -> String {
    format!("{base}/{file}")
}

fn info(base: &str, r: ThumbReply) -> ThumbInfo {
    ThumbInfo {
        url: r.file.as_deref().map(|f| url_of(base, f)),
        path: r.path,
        at_secs: r.at_secs,
        state: r.state,
        reason: r.reason,
    }
}

/// Payload of `thumbnails://ready`.
fn done_payload(base: &str, d: &ThumbDone) -> Value {
    match &d.result {
        Ok(file) => json!({"path": d.path, "atSecs": d.at_secs, "url": url_of(base, file)}),
        Err(reason) => {
            json!({"path": d.path, "atSecs": d.at_secs, "state": "unavailable", "reason": reason})
        }
    }
}

pub struct Thumbnails {
    core: Arc<AppCore>,
    service: ThumbService,
}

impl Thumbnails {
    /// Starts the service over `cache_dir` (cleaned if oversized) and points
    /// the media server's thumbnail route at it. Needs a Tokio runtime.
    pub fn new(
        core: Arc<AppCore>,
        cache_dir: PathBuf,
        ffmpeg: Option<PathBuf>,
        emit: Emit,
    ) -> Self {
        core.set_thumb_dir(&cache_dir);
        let announcer = Arc::clone(&core);
        let service = ThumbService::new(
            ThumbConfig::new(cache_dir, ffmpeg),
            Arc::new(move |done| emit(READY_EVENT, done_payload(&announcer.thumb_base(), &done))),
        );
        Self { core, service }
    }

    pub async fn request(&self, req: ThumbsRequest) -> Vec<ThumbInfo> {
        let items: Vec<ThumbItem> = req
            .items
            .into_iter()
            .map(|i| ThumbItem {
                path: i.path,
                at_secs: i.at_secs,
            })
            .collect();
        let service = self.service.clone();
        let root = self.core.library_root();
        let base = self.core.thumb_base();
        tokio::task::spawn_blocking(move || {
            service
                .request(&root, &items, req.width)
                .into_iter()
                .map(|r| info(&base, r))
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    pub fn cancel(&self, req: CancelRequest) {
        self.service.cancel(&req.paths);
    }

    pub async fn status(&self) -> ThumbsStatus {
        let service = self.service.clone();
        let s = tokio::task::spawn_blocking(move || service.status()).await;
        s.map_or(
            ThumbsStatus {
                ffmpeg_available: false,
                cache_bytes: 0,
                queued: 0,
                running: 0,
            },
            |s| ThumbsStatus {
                ffmpeg_available: s.ffmpeg_available,
                cache_bytes: s.cache_bytes,
                queued: s.queued,
                running: s.running,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;
    use std::time::Duration;

    struct Fixture {
        _tmp: tempfile::TempDir,
        core: Arc<AppCore>,
        thumbs: Thumbnails,
        events: Arc<Mutex<Vec<Value>>>,
    }

    async fn fixture(ffmpeg_body: Option<&str>) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(lib.join("Skool/G")).unwrap();
        std::fs::write(lib.join("Skool/G/a.mp4"), "video").unwrap();
        let core = Arc::new(AppCore::new(&tmp.path().join("p.db"), lib).await.unwrap());
        let ffmpeg = ffmpeg_body.map(|body| {
            let bin = tmp.path().join("ffmpeg");
            std::fs::write(
                &bin,
                format!("#!/bin/sh\nfor a in \"$@\"; do out=\"$a\"; done\n{body}\n"),
            )
            .unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            bin
        });
        let events: Arc<Mutex<Vec<Value>>> = Arc::default();
        let sink = Arc::clone(&events);
        let thumbs = Thumbnails::new(
            Arc::clone(&core),
            tmp.path().join("cache/thumbs"),
            ffmpeg,
            Arc::new(move |name, payload| {
                sink.lock()
                    .unwrap()
                    .push(json!({"event": name, "payload": payload}))
            }),
        );
        Fixture {
            _tmp: tmp,
            core,
            thumbs,
            events,
        }
    }

    fn req(paths: &[&str]) -> ThumbsRequest {
        ThumbsRequest {
            items: paths
                .iter()
                .map(|p| ThumbItemArg {
                    path: (*p).into(),
                    at_secs: None,
                })
                .collect(),
            width: None,
        }
    }

    async fn wait_events(f: &Fixture, n: usize) -> Vec<Value> {
        for _ in 0..200 {
            if f.events.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        f.events.lock().unwrap().clone()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pending_item_announces_its_url_and_the_route_serves_the_image() {
        let f = fixture(Some("printf 'JPEGDATA' > \"$out\"")).await;
        let first = f.thumbs.request(req(&["Skool/G/a.mp4"])).await;
        let v = serde_json::to_value(&first).unwrap();
        assert_eq!(v[0]["path"], "Skool/G/a.mp4");
        assert_eq!(v[0]["atSecs"], 8.0);
        assert_eq!(v[0]["state"], "pending");
        assert!(v[0].get("url").is_none());
        let ev = wait_events(&f, 1).await;
        assert_eq!(ev[0]["event"], "thumbnails://ready");
        let p = &ev[0]["payload"];
        assert_eq!(
            (p["path"].clone(), p["atSecs"].clone()),
            (json!("Skool/G/a.mp4"), json!(8.0))
        );
        let url = p["url"].as_str().unwrap().to_string();
        assert!(
            url.starts_with(&format!("{}/", f.core.thumb_base())) && url.ends_with(".jpg"),
            "{url}"
        );
        let again = serde_json::to_value(f.thumbs.request(req(&["Skool/G/a.mp4"])).await).unwrap();
        assert_eq!(
            (again[0]["state"].clone(), again[0]["url"].clone()),
            (json!("ready"), json!(url))
        );
        let body = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let rest = url.trim_start_matches("http://");
            let (host, path) = rest.split_once('/').unwrap();
            let mut s = std::net::TcpStream::connect(host).unwrap();
            write!(
                s,
                "GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert!(
            body.starts_with("HTTP/1.1 200") && body.ends_with("JPEGDATA"),
            "{body}"
        );
        assert!(body.to_lowercase().contains("image/jpeg"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failures_and_a_missing_ffmpeg_are_unavailable_with_a_reason() {
        let f = fixture(Some("exit 1")).await;
        f.thumbs.request(req(&["Skool/G/a.mp4"])).await;
        let ev = wait_events(&f, 1).await;
        let p = &ev[0]["payload"];
        assert_eq!(
            (p["state"].clone(), p["reason"].clone()),
            (json!("unavailable"), json!("ffmpeg_failed"))
        );
        assert!(p.get("url").is_none());
        let none = fixture(None).await;
        let r = serde_json::to_value(
            none.thumbs
                .request(req(&["Skool/G/a.mp4", "../x.mp4"]))
                .await,
        )
        .unwrap();
        assert_eq!(
            (r[0]["state"].clone(), r[0]["reason"].clone()),
            (json!("unavailable"), json!("ffmpeg_missing"))
        );
        assert_eq!(r[1]["reason"], "invalid_path");
        let st = serde_json::to_value(none.thumbs.status().await).unwrap();
        assert_eq!(
            st,
            json!({"ffmpegAvailable": false, "cacheBytes": 0, "queued": 0, "running": 0})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn requests_follow_the_current_library_root_and_cancel_is_accepted() {
        let f = fixture(Some("printf 'J' > \"$out\"")).await;
        let other = f._tmp.path().join("other");
        std::fs::create_dir_all(other.join("Skool/H")).unwrap();
        std::fs::write(other.join("Skool/H/b.mp4"), "v").unwrap();
        f.core.library_scan(other.to_str().unwrap()).unwrap();
        let r = serde_json::to_value(
            f.thumbs
                .request(req(&["Skool/G/a.mp4", "Skool/H/b.mp4"]))
                .await,
        )
        .unwrap();
        assert_eq!(
            r[0]["reason"], "invalid_path",
            "the old root no longer applies"
        );
        assert_eq!(r[1]["state"], "pending");
        f.thumbs.cancel(CancelRequest {
            paths: vec!["Skool/H/b.mp4".into()],
        });
        wait_events(&f, 1).await;
    }

    #[test]
    fn request_arguments_are_camel_case() {
        let r: ThumbsRequest = serde_json::from_value(json!({"items": [{"path": "a.mp4", "atSecs": 3.5}, {"path": "b.mp4", "atSecs": null}, {"path": "c.mp4"}], "width": 320})).unwrap();
        assert_eq!(r.width, Some(320));
        assert_eq!(r.items[0].at_secs, Some(3.5));
        assert_eq!((r.items[1].at_secs, r.items[2].at_secs), (None, None));
        let c: CancelRequest = serde_json::from_value(json!({"paths": ["a"]})).unwrap();
        assert_eq!(c.paths, ["a"]);
    }
}
