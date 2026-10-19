//! Entry point of the adapter: resolves a pasted link into a preview and
//! (see `downloader`) downloads it. The yt-dlp binary sits behind
//! [`MediaTool`], so everything here is tested with fakes.

use std::path::PathBuf;
use std::sync::Arc;

use lumen_core::CancelFlag;
use lumen_download::{MediaTool, VideoError};
use serde::Serialize;

use crate::error::YoutubeError;
use crate::parse::{parse_playlist, parse_video, ItemInfo};
use crate::planner::{folder_name, SINGLES_FOLDER};
use crate::reason::classify;
use crate::url::{parse_target, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedKind {
    Video,
    Playlist,
}

/// What the UI previews before downloading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub kind: ResolvedKind,
    pub id: String,
    pub title: String,
    pub channel: String,
    pub thumbnail_url: Option<String>,
    pub duration_secs: Option<u64>,
    /// Folder under `YouTube/` the download will use.
    pub folder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<ItemInfo>>,
}

pub struct Youtube {
    pub(crate) tool: Arc<dyn MediaTool>,
    /// ffmpeg for merging streams and converting thumbnails, when installed.
    pub(crate) ffmpeg: Option<PathBuf>,
}

impl Youtube {
    pub fn new(tool: Arc<dyn MediaTool>, ffmpeg: Option<PathBuf>) -> Self {
        Self { tool, ffmpeg }
    }

    pub(crate) async fn probe(
        &self,
        target: &Target,
        cancel: &CancelFlag,
    ) -> Result<serde_json::Value, YoutubeError> {
        let flat = matches!(target, Target::Playlist(_));
        match self.tool.probe(&target.canonical_url(), flat, cancel).await {
            Ok(doc) => Ok(doc),
            Err(VideoError::Failed(msg)) => Err(match classify(&msg) {
                Some(reason) => YoutubeError::Unavailable(reason.text.to_string()),
                None => YoutubeError::Tool(VideoError::Failed(msg)),
            }),
            Err(e) => Err(e.into()),
        }
    }

    /// Normalises `input`, asks yt-dlp for metadata (no download) and shapes
    /// the preview. Playlists are listed flat: nothing per entry is resolved.
    pub async fn resolve(
        &self,
        input: &str,
        cancel: &CancelFlag,
    ) -> Result<Resolved, YoutubeError> {
        let target = parse_target(input)?;
        let doc = self.probe(&target, cancel).await?;
        match target {
            Target::Video(_) => {
                let v = parse_video(&doc)?;
                Ok(Resolved {
                    kind: ResolvedKind::Video,
                    id: v.id,
                    title: v.title,
                    channel: v.channel,
                    thumbnail_url: v.thumbnail_url,
                    duration_secs: v.duration_secs,
                    folder: SINGLES_FOLDER.to_string(),
                    items: None,
                })
            }
            Target::Playlist(id) => {
                let p = parse_playlist(&doc)?;
                if p.items.is_empty() {
                    return Err(YoutubeError::EmptyPlaylist);
                }
                Ok(Resolved {
                    kind: ResolvedKind::Playlist,
                    id: if p.id.is_empty() { id } else { p.id },
                    folder: folder_name(&p.title),
                    title: p.title,
                    channel: p.channel,
                    thumbnail_url: p.thumbnail_url,
                    duration_secs: None,
                    items: Some(p.items),
                })
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use lumen_core::CancelFlag;
    use lumen_download::{MediaJob, MediaTool, VideoError};
    use serde_json::Value;

    type FetchFn = dyn Fn(&MediaJob) -> Result<(), VideoError> + Send + Sync;

    /// yt-dlp stand-in: canned probe documents and a scripted fetch.
    pub struct FakeTool {
        pub probes: Mutex<HashMap<String, Result<Value, VideoError>>>,
        pub probed: Mutex<Vec<(String, bool)>>,
        pub jobs: Mutex<Vec<MediaJob>>,
        pub fetch: Box<FetchFn>,
        pub percents: Vec<f32>,
    }

    impl FakeTool {
        pub fn new() -> Self {
            Self {
                probes: Mutex::default(),
                probed: Mutex::default(),
                jobs: Mutex::default(),
                fetch: Box::new(write_media),
                percents: vec![10.0, 100.0],
            }
        }

        pub fn with_probe(self, url: &str, doc: Result<Value, VideoError>) -> Self {
            self.probes.lock().unwrap().insert(url.to_string(), doc);
            self
        }

        pub fn with_fetch(
            mut self,
            f: impl Fn(&MediaJob) -> Result<(), VideoError> + Send + Sync + 'static,
        ) -> Self {
            self.fetch = Box::new(f);
            self
        }

        pub fn fetched_urls(&self) -> Vec<String> {
            self.jobs
                .lock()
                .unwrap()
                .iter()
                .map(|j| j.url.clone())
                .collect()
        }
    }

    fn template_path(job: &MediaJob, ext: &str) -> PathBuf {
        PathBuf::from(
            job.output_template
                .to_string_lossy()
                .replace("%(ext)s", ext),
        )
    }

    /// Writes what yt-dlp would: the media, its description and a thumbnail.
    pub fn write_media(job: &MediaJob) -> Result<(), VideoError> {
        for (ext, body) in [("mp4", "v"), ("description", "about"), ("jpg", "j")] {
            std::fs::write(template_path(job, ext), body)
                .map_err(|e| VideoError::Failed(e.to_string()))?;
        }
        Ok(())
    }

    #[async_trait]
    impl MediaTool for FakeTool {
        async fn probe(&self, url: &str, flat: bool, _: &CancelFlag) -> Result<Value, VideoError> {
            self.probed.lock().unwrap().push((url.to_string(), flat));
            self.probes
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or_else(|| Err(VideoError::Failed(format!("no fixture for {url}"))))
        }

        async fn fetch(
            &self,
            job: &MediaJob,
            cancel: &CancelFlag,
            on_percent: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            if cancel.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            self.jobs.lock().unwrap().push(job.clone());
            self.percents.iter().for_each(|p| on_percent(*p));
            (self.fetch)(job)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeTool;
    use super::*;
    use lumen_core::CancelFlag;
    use lumen_download::VideoError;
    use serde_json::json;
    use std::sync::Arc;

    const V: &str = "dQw4w9WgXcQ";
    const PL: &str = "PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf";

    fn video_doc() -> serde_json::Value {
        json!({"id": V, "title": "Canción: ñ", "channel": "Canal", "duration": 212,
               "thumbnail": "https://i.ytimg.com/vi/x/hq.jpg"})
    }

    fn playlist_doc() -> serde_json::Value {
        json!({"id": PL, "title": "Mi lista: 1", "channel": "Canal", "entries": [
            {"id": "aaaaaaaaaaa", "title": "Uno", "duration": 61},
            {"id": "bbbbbbbbbbb", "title": "[Private video]"}
        ]})
    }

    fn yt(tool: FakeTool) -> (Youtube, Arc<FakeTool>) {
        let tool = Arc::new(tool);
        (Youtube::new(tool.clone(), None), tool)
    }

    #[tokio::test]
    async fn a_video_resolves_with_its_preview_and_the_singles_folder() {
        let url = format!("https://www.youtube.com/watch?v={V}");
        let (y, tool) = yt(FakeTool::new().with_probe(&url, Ok(video_doc())));
        let r = y
            .resolve(&format!("https://youtu.be/{V}?t=5"), &CancelFlag::new())
            .await
            .unwrap();
        assert_eq!(r.kind, ResolvedKind::Video);
        assert_eq!(
            (r.id.as_str(), r.title.as_str(), r.channel.as_str()),
            (V, "Canción: ñ", "Canal")
        );
        assert_eq!(r.duration_secs, Some(212));
        assert_eq!(r.folder, "Videos sueltos");
        assert!(r.items.is_none());
        assert_eq!(*tool.probed.lock().unwrap(), [(url, false)]);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["kind"], "video");
        assert_eq!(json["thumbnailUrl"], "https://i.ytimg.com/vi/x/hq.jpg");
        assert_eq!(json["durationSecs"], 212);
        assert!(json.get("items").is_none());
    }

    #[tokio::test]
    async fn a_playlist_resolves_flat_with_its_items_and_a_safe_folder() {
        let url = format!("https://www.youtube.com/playlist?list={PL}");
        let (y, tool) = yt(FakeTool::new().with_probe(&url, Ok(playlist_doc())));
        let r = y
            .resolve(
                &format!("https://www.youtube.com/watch?list={PL}"),
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert_eq!(r.kind, ResolvedKind::Playlist);
        assert_eq!((r.id.as_str(), r.title.as_str()), (PL, "Mi lista: 1"));
        assert_eq!(r.folder, "Mi lista_ 1");
        let items = r.items.unwrap();
        assert_eq!(items.len(), 2);
        assert!(items[0].available && !items[1].available);
        assert_eq!(*tool.probed.lock().unwrap(), [(url, true)]);
    }

    #[tokio::test]
    async fn bad_links_never_reach_yt_dlp() {
        let (y, tool) = yt(FakeTool::new());
        let c = CancelFlag::new();
        assert_eq!(
            y.resolve("hello world", &c).await.unwrap_err(),
            YoutubeError::InvalidUrl
        );
        assert_eq!(
            y.resolve("https://vimeo.com/12345", &c).await.unwrap_err(),
            YoutubeError::UnsupportedUrl
        );
        assert!(tool.probed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failures_are_classified_empty_playlists_and_tool_errors_are_reported() {
        let vurl = format!("https://www.youtube.com/watch?v={V}");
        let purl = format!("https://www.youtube.com/playlist?list={PL}");
        let (y, _) = yt(FakeTool::new()
            .with_probe(
                &vurl,
                Err(VideoError::Failed(
                    "[youtube] x: Video unavailable. This video is private".into(),
                )),
            )
            .with_probe(
                &purl,
                Ok(json!({"id": PL, "title": "Vacía", "entries": []})),
            ));
        let c = CancelFlag::new();
        assert_eq!(
            y.resolve(&vurl, &c).await.unwrap_err(),
            YoutubeError::Unavailable("private video".into())
        );
        assert_eq!(
            y.resolve(&purl, &c).await.unwrap_err(),
            YoutubeError::EmptyPlaylist
        );
        let (y, _) =
            yt(FakeTool::new().with_probe(&vurl, Err(VideoError::NotFound("/x/yt-dlp".into()))));
        assert!(matches!(
            y.resolve(&vurl, &c).await.unwrap_err(),
            YoutubeError::Tool(VideoError::NotFound(_))
        ));
        let (y, _) = yt(FakeTool::new().with_probe(&vurl, Ok(json!({"nope": 1}))));
        assert!(matches!(
            y.resolve(&vurl, &c).await.unwrap_err(),
            YoutubeError::Parse(_)
        ));
    }
}
