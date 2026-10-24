//! YouTube commands' application layer: resolves a pasted link into a preview
//! and runs downloads through the shared [`DownloadManager`]. It holds no
//! Tauri types, so it is tested without a window.

use std::path::PathBuf;
use std::sync::Arc;

use lumen_core::{CancelFlag, PlatformError};
use lumen_download::VideoError;
use lumen_youtube::url::parse_target;
use lumen_youtube::{Request, Resolved, Youtube, YoutubeError};

use crate::downloads::{CommandError, DownloadEvents, DownloadManager, ErrorCode, RunningDownload};

/// Maps the adapter's errors to the stable codes the UI switches on.
fn map(e: YoutubeError) -> CommandError {
    let code = match &e {
        YoutubeError::InvalidUrl => ErrorCode::InvalidUrl,
        YoutubeError::UnsupportedUrl => ErrorCode::UnsupportedUrl,
        YoutubeError::Unavailable(_) | YoutubeError::EmptyPlaylist => ErrorCode::Unavailable,
        YoutubeError::Tool(VideoError::NotFound(_) | VideoError::Spawn(_)) => {
            ErrorCode::YtdlpMissing
        }
        YoutubeError::Tool(_) | YoutubeError::Parse(_) | YoutubeError::Io(_) => ErrorCode::Internal,
    };
    CommandError::new(code, e.to_string())
}

/// Download task failure as the shared manager reports it.
fn platform_error(e: YoutubeError) -> PlatformError {
    match e {
        YoutubeError::Io(m) => PlatformError::Io(m),
        YoutubeError::Parse(p) => PlatformError::Parse(p.to_string()),
        other => PlatformError::InvalidInput(other.to_string()),
    }
}

pub struct YoutubeService {
    youtube: Arc<Youtube>,
    downloads: DownloadManager,
}

impl YoutubeService {
    pub fn new(youtube: Arc<Youtube>) -> Self {
        Self {
            youtube,
            downloads: DownloadManager::new("youtube"),
        }
    }

    pub async fn resolve(&self, url: &str) -> Result<Resolved, CommandError> {
        self.youtube
            .resolve(url, &CancelFlag::new())
            .await
            .map_err(map)
    }

    /// Starts a background download and returns its id. The link is
    /// validated first, so a bad one never spawns anything. Needs a Tokio
    /// runtime.
    pub fn start_download(
        &self,
        request: Request,
        library_root: PathBuf,
        events: Arc<dyn DownloadEvents>,
    ) -> Result<String, CommandError> {
        let target = parse_target(&request.url).map_err(|e| map(e.into()))?;
        if library_root.as_os_str().is_empty() {
            return Err(CommandError::new(
                ErrorCode::NoLibrary,
                "choose a library folder first",
            ));
        }
        crate::downloads::ensure_library_ready(&library_root)?;
        let youtube = Arc::clone(&self.youtube);
        self.downloads.start_task(
            &target.canonical_url(),
            events,
            move |sink, cancel| async move {
                youtube
                    .download(&request, &library_root, sink.as_ref(), &cancel)
                    .await
                    .map_err(platform_error)
            },
        )
    }

    pub fn cancel(&self, id: &str) -> bool {
        self.downloads.cancel(id)
    }

    pub fn running(&self) -> Vec<RunningDownload> {
        self.downloads.running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::{ProgressEvent, ReportEvent};
    use async_trait::async_trait;
    use lumen_core::CancelFlag;
    use lumen_download::{MediaJob, MediaTool, VideoError};
    use serde_json::json;
    use std::time::Duration;
    use tokio::sync::mpsc;

    const V: &str = "dQw4w9WgXcQ";

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Quick,
        UntilCancelled,
        MissingBinary,
    }

    struct FakeTool(Mode);

    #[async_trait]
    impl MediaTool for FakeTool {
        async fn probe(
            &self,
            url: &str,
            _flat: bool,
            _: &CancelFlag,
        ) -> Result<serde_json::Value, VideoError> {
            if self.0 == Mode::MissingBinary {
                return Err(VideoError::NotFound("/x/yt-dlp".into()));
            }
            if url.contains("private0000") {
                return Err(VideoError::Failed(
                    "Video unavailable. This video is private".into(),
                ));
            }
            Ok(json!({"id": V, "title": "Canción", "channel": "Canal", "duration": 60}))
        }

        async fn fetch(
            &self,
            job: &MediaJob,
            cancel: &CancelFlag,
            on_percent: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            on_percent(40.0);
            if self.0 == Mode::UntilCancelled {
                while !cancel.is_cancelled() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                return Err(VideoError::Cancelled);
            }
            let file = job
                .output_template
                .to_string_lossy()
                .replace("%(ext)s", "mp4");
            std::fs::write(file, "v").map_err(|e| VideoError::Failed(e.to_string()))
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

    fn service(mode: Mode) -> YoutubeService {
        YoutubeService::new(Arc::new(Youtube::new(Arc::new(FakeTool(mode)), None)))
    }

    fn request(url: &str) -> Request {
        Request {
            url: url.into(),
            items: None,
            quality: lumen_youtube::Quality::P720,
        }
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
    async fn resolve_returns_the_preview_and_maps_errors_to_stable_codes() {
        let s = service(Mode::Quick);
        let r = s.resolve(&format!("https://youtu.be/{V}")).await.unwrap();
        assert_eq!((r.id.as_str(), r.title.as_str()), (V, "Canción"));
        let e = s.resolve("hello world").await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidUrl);
        let e = s.resolve("https://vimeo.com/1").await.unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedUrl);
        let e = s
            .resolve("https://www.youtube.com/watch?v=private0000")
            .await
            .unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (ErrorCode::Unavailable, "private video")
        );
        let e = service(Mode::MissingBinary)
            .resolve(&format!("https://youtu.be/{V}"))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::YtdlpMissing);
        let json = serde_json::to_value(ErrorCode::UnsupportedUrl).unwrap();
        assert_eq!(json, "unsupported_url");
    }

    #[tokio::test]
    async fn start_validates_before_spawning_anything() {
        let s = service(Mode::Quick);
        let (tx, _rx) = mpsc::unbounded_channel();
        let events: Arc<dyn DownloadEvents> = Arc::new(Channel(tx));
        let dir = tempfile::tempdir().unwrap();
        let e = s
            .start_download(request("not a link"), dir.path().into(), events.clone())
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidUrl);
        let e = s
            .start_download(
                request("https://vimeo.com/1"),
                dir.path().into(),
                events.clone(),
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedUrl);
        let e = s
            .start_download(
                request(&format!("https://youtu.be/{V}")),
                PathBuf::new(),
                events,
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NoLibrary);
        assert!(s.running().is_empty());
    }

    #[tokio::test]
    async fn a_download_emits_youtube_tagged_progress_then_one_report() {
        let s = service(Mode::Quick);
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = s
            .start_download(
                request(&format!("https://youtu.be/{V}?si=x")),
                dir.path().into(),
                Arc::new(Channel(tx)),
            )
            .unwrap();
        let (report, progress) = next_report(&mut rx).await;
        assert_eq!(report.id, id);
        assert_eq!(report.report.platform, "youtube");
        assert_eq!(
            report.report.group,
            format!("https://www.youtube.com/watch?v={V}")
        );
        assert!(report.report.complete, "{:?}", report.report.issues);
        assert!(progress
            .iter()
            .all(|p| p.id == id && p.platform == "youtube"));
        assert!(dir
            .path()
            .join("YouTube/Videos sueltos/01 - Canción/Canción.mp4")
            .is_file());
    }

    #[tokio::test]
    async fn the_same_link_cannot_run_twice_and_cancel_ends_it() {
        let s = service(Mode::UntilCancelled);
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let events: Arc<dyn DownloadEvents> = Arc::new(Channel(tx));
        let id = s
            .start_download(
                request(&format!("https://youtu.be/{V}")),
                dir.path().into(),
                events.clone(),
            )
            .unwrap();
        assert_eq!(
            s.running(),
            [RunningDownload {
                id: id.clone(),
                platform: "youtube".into(),
                group: format!("https://www.youtube.com/watch?v={V}"),
            }]
        );
        let again = s
            .start_download(
                request(&format!("https://www.youtube.com/watch?v={V}&t=3")),
                dir.path().into(),
                events,
            )
            .unwrap_err();
        assert_eq!(again.code, ErrorCode::AlreadyRunning);
        assert!(s.cancel(&id));
        let (report, _) = next_report(&mut rx).await;
        assert!(report.report.cancelled);
        assert!(!s.cancel("nope"));
    }

    #[tokio::test]
    async fn a_fatal_failure_still_produces_a_report() {
        let s = service(Mode::MissingBinary);
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        s.start_download(
            request(&format!("https://youtu.be/{V}")),
            dir.path().into(),
            Arc::new(Channel(tx)),
        )
        .unwrap();
        let (report, _) = next_report(&mut rx).await;
        assert!(!report.report.complete);
        assert!(report.report.issues[0].message.contains("yt-dlp"));
    }
}
