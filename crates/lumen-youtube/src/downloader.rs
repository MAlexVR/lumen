//! Downloads a video or the selected items of a playlist, one yt-dlp run per
//! video so a failure never stops the rest. Every skipped or failed item
//! becomes an issue in the report.

use std::path::Path;

use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadProgress, DownloadReport, Issue, IssueKind,
};
use lumen_download::layout::NumberStyle;
use lumen_download::report::finalize_folder;
use lumen_download::{MediaJob, VideoError};
use serde::Deserialize;

use crate::client::Youtube;
use crate::error::YoutubeError;
use crate::parse::{parse_playlist, parse_video};
use crate::planner::{
    file_stem, format_args_for, group_dir, lesson_folder, next_number, Quality, SINGLES_FOLDER,
};
use crate::reason::classify;
use crate::url::Target;

pub const PLATFORM_ID: &str = "youtube";
/// Hidden file in a lesson folder holding the video id, so a repeated
/// download of the same video is recognised and skipped.
const ID_MARKER: &str = ".youtube-id";
const DESCRIPTION_FILE: &str = "description.txt";
const THUMBNAIL_STEM: &str = "thumbnail";
const IMAGE_EXTS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
const MEDIA_EXTS: [&str; 8] = ["mp4", "mkv", "webm", "m4a", "mp3", "opus", "ogg", "aac"];

/// What the UI asks for: a link, optionally the ids to keep, and a preset.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub url: String,
    #[serde(default)]
    pub items: Option<Vec<String>>,
    pub quality: Quality,
}

struct Entry {
    id: String,
    title: String,
    number: usize,
    /// Why it cannot be downloaded, when known before trying.
    unavailable: Option<String>,
    /// Folder of a previous download of the same video (single videos).
    existing: Option<String>,
}

impl Entry {
    fn label(&self) -> String {
        format!("{}{}", NumberStyle::Padded.label(self.number), self.title)
    }

    fn real_id(&self) -> Option<&str> {
        (!self.id.starts_with("unavailable-")).then_some(self.id.as_str())
    }
}

struct Plan {
    group_title: String,
    entries: Vec<Entry>,
    /// Requested ids that are not in the playlist.
    unknown: Vec<String>,
}

fn watch_url(id: &str) -> String {
    Target::Video(id.to_string()).canonical_url()
}

fn read_marker(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(ID_MARKER))
        .ok()
        .map(|s| s.trim().to_string())
}

fn has_media(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|d| {
        d.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.rsplit_once('.')
                .is_some_and(|(_, ext)| MEDIA_EXTS.contains(&ext))
        })
    })
}

/// Existing folder of `group_dir` that already holds video `id`.
fn find_downloaded(group_dir: &Path, id: &str) -> Option<String> {
    std::fs::read_dir(group_dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .find(|e| read_marker(&e.path()).as_deref() == Some(id) && has_media(&e.path()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
}

/// Gives yt-dlp's side files their stable names inside `dir`.
fn tidy_extras(dir: &Path, stem: &str) {
    let _ = std::fs::rename(
        dir.join(format!("{stem}.description")),
        dir.join(DESCRIPTION_FILE),
    );
    for ext in IMAGE_EXTS {
        let from = dir.join(format!("{stem}.{ext}"));
        if from.is_file() {
            let _ = std::fs::rename(from, dir.join(format!("{THUMBNAIL_STEM}.{ext}")));
        }
    }
}

/// Removes a lesson folder a failed or cancelled run left empty.
fn drop_if_empty(dir: &Path) {
    let _ = std::fs::remove_dir(dir);
}

/// Report URLs lose their query, which is where the video id lives: keep the
/// id in the message so the item stays identifiable.
fn with_id(entry: &Entry, message: String) -> String {
    match entry.real_id() {
        Some(id) => format!("{message} [{id}]"),
        None => message,
    }
}

fn failure_issue(entry: &Entry, message: &str) -> Issue {
    let label = entry.label();
    let url = entry.real_id().map(watch_url);
    match classify(message) {
        Some(r) if r.permanent => Issue::warning(
            IssueKind::VideoNotResolved,
            Some(&label),
            url.as_deref(),
            with_id(entry, format!("{}: {message}", r.text)),
        ),
        Some(r) => Issue::error(
            IssueKind::VideoNotResolved,
            Some(&label),
            url.as_deref(),
            with_id(entry, format!("{}: {message}", r.text)),
        ),
        None => Issue::error(
            IssueKind::Video,
            Some(&label),
            url.as_deref(),
            with_id(entry, message.to_string()),
        ),
    }
}

impl Youtube {
    async fn plan(
        &self,
        target: &Target,
        request: &Request,
        root: &Path,
        cancel: &CancelFlag,
    ) -> Result<Plan, YoutubeError> {
        let doc = self.probe(target, cancel).await?;
        match target {
            Target::Video(_) => {
                let v = parse_video(&doc)?;
                let dir = group_dir(root, SINGLES_FOLDER);
                let existing = find_downloaded(&dir, &v.id);
                let number = existing
                    .as_deref()
                    .and_then(|f| f.split(" - ").next())
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| next_number(&dir));
                Ok(Plan {
                    group_title: SINGLES_FOLDER.to_string(),
                    entries: vec![Entry {
                        id: v.id,
                        title: v.title,
                        number,
                        unavailable: None,
                        existing,
                    }],
                    unknown: Vec::new(),
                })
            }
            Target::Playlist(_) => {
                let p = parse_playlist(&doc)?;
                if p.items.is_empty() {
                    return Err(YoutubeError::EmptyPlaylist);
                }
                let unknown = request
                    .items
                    .iter()
                    .flatten()
                    .filter(|id| !p.items.iter().any(|i| &i.id == *id))
                    .cloned()
                    .collect();
                let entries = p
                    .items
                    .into_iter()
                    .enumerate()
                    .filter(|(_, i)| request.items.as_ref().is_none_or(|ids| ids.contains(&i.id)))
                    .map(|(pos, i)| Entry {
                        id: i.id,
                        title: i.title,
                        number: pos + 1,
                        unavailable: i.reason,
                        existing: None,
                    })
                    .collect();
                Ok(Plan {
                    group_title: p.title,
                    entries,
                    unknown,
                })
            }
        }
    }

    /// Downloads what `request` asks for under `library_root`. Item failures
    /// go in the report; only a bad link, a missing yt-dlp or an unreadable
    /// listing are fatal.
    pub async fn download(
        &self,
        request: &Request,
        library_root: &Path,
        sink: &dyn CourseProgressSink,
        cancel: &CancelFlag,
    ) -> Result<DownloadReport, YoutubeError> {
        let target = crate::url::parse_target(&request.url)?;
        let canonical = target.canonical_url();
        let plan = self.plan(&target, request, library_root, cancel).await?;
        let mut report = DownloadReport::new(PLATFORM_ID, &canonical);
        report.total_lessons = plan.entries.len() as u32;
        for id in &plan.unknown {
            report.push(Issue::error(
                IssueKind::VideoNotResolved,
                None,
                None,
                format!("video {id} is not in the playlist"),
            ));
        }
        for e in plan.entries.iter().filter(|e| e.unavailable.is_some()) {
            report.push(Issue::warning(
                IssueKind::VideoNotResolved,
                Some(&e.label()),
                e.real_id().map(watch_url).as_deref(),
                with_id(e, e.unavailable.clone().unwrap_or_default()),
            ));
        }
        let downloads: Vec<&Entry> = plan
            .entries
            .iter()
            .filter(|e| e.unavailable.is_none())
            .collect();
        if downloads.is_empty() {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "no video could be downloaded",
            ));
            report.finish();
            sink.on_report(&report);
            return Ok(report);
        }
        if request.quality.needs_merge() && self.ffmpeg.is_none() {
            report.push(Issue::warning(
                IssueKind::Course,
                None,
                None,
                "ffmpeg was not found: single-file formats are used, which may be lower quality",
            ));
        }
        let group_dir = group_dir(library_root, &plan.group_title);
        std::fs::create_dir_all(&group_dir).map_err(|e| YoutubeError::Io(e.to_string()))?;
        let total = downloads.len() as u32;
        let mut completed = 0u32;
        for entry in downloads {
            if cancel.is_cancelled() {
                report.cancelled = true;
                break;
            }
            let folder = entry
                .existing
                .clone()
                .unwrap_or_else(|| lesson_folder(&group_dir, entry.number, &entry.title));
            let lesson_dir = group_dir.join(&folder);
            if read_marker(&lesson_dir).as_deref() == Some(entry.id.as_str())
                && has_media(&lesson_dir)
            {
                completed += 1;
                continue;
            }
            if let Err(e) = std::fs::create_dir_all(&lesson_dir) {
                report.push(Issue::error(
                    IssueKind::Video,
                    Some(&entry.label()),
                    None,
                    format!("cannot create the folder: {e}"),
                ));
                continue;
            }
            let stem = file_stem(&lesson_dir, &entry.title);
            let job = MediaJob {
                url: watch_url(&entry.id),
                output_template: lesson_dir.join(format!("{stem}.%(ext)s")),
                args: self.job_args(request.quality),
            };
            let on_percent = |percent: f32| {
                sink.on_progress(&DownloadProgress {
                    group: canonical.clone(),
                    course: plan.group_title.clone(),
                    lesson: folder.clone(),
                    percent,
                    completed,
                    total,
                });
            };
            match self.tool.fetch(&job, cancel, &on_percent).await {
                Ok(()) if has_media(&lesson_dir) => {
                    tidy_extras(&lesson_dir, &stem);
                    match std::fs::write(lesson_dir.join(ID_MARKER), &entry.id) {
                        Ok(()) => completed += 1,
                        Err(e) => report.push(Issue::error(
                            IssueKind::Video,
                            Some(&entry.label()),
                            None,
                            format!("cannot record the download: {e}"),
                        )),
                    }
                }
                Ok(()) => {
                    drop_if_empty(&lesson_dir);
                    report.push(Issue::error(
                        IssueKind::Video,
                        Some(&entry.label()),
                        entry.real_id().map(watch_url).as_deref(),
                        "yt-dlp finished without producing a media file",
                    ));
                }
                Err(VideoError::Cancelled) => {
                    drop_if_empty(&lesson_dir);
                    report.cancelled = true;
                    break;
                }
                Err(e @ (VideoError::NotFound(_) | VideoError::Spawn(_))) => {
                    drop_if_empty(&lesson_dir);
                    return Err(e.into());
                }
                Err(VideoError::Failed(message)) => {
                    drop_if_empty(&lesson_dir);
                    report.push(failure_issue(entry, &message));
                }
            }
        }
        report.completed_lessons = completed;
        finalize_folder(&group_dir, &mut report);
        sink.on_report(&report);
        Ok(report)
    }

    fn job_args(&self, quality: Quality) -> Vec<String> {
        let mut args = format_args_for(quality, self.ffmpeg.as_deref());
        args.push("--write-description".into());
        args.push("--write-thumbnail".into());
        if self.ffmpeg.is_some() {
            args.push("--convert-thumbnails".into());
            args.push("jpg".into());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::fake::FakeTool;
    use lumen_core::{CourseProgressSink, DownloadProgress, DownloadReport, IssueKind, Severity};
    use lumen_download::VideoError;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    const V: &str = "dQw4w9WgXcQ";
    const W: &str = "abcdefghijk";
    const PL: &str = "PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf";

    fn watch(id: &str) -> String {
        format!("https://www.youtube.com/watch?v={id}")
    }

    fn list_url() -> String {
        format!("https://www.youtube.com/playlist?list={PL}")
    }

    fn video(id: &str, title: &str) -> serde_json::Value {
        json!({"id": id, "title": title, "channel": "Canal"})
    }

    fn playlist(entries: serde_json::Value) -> serde_json::Value {
        json!({"id": PL, "title": "Mi lista: ñ", "channel": "Canal", "entries": entries})
    }

    #[derive(Default)]
    struct Rec {
        progress: Mutex<Vec<DownloadProgress>>,
        reports: Mutex<Vec<DownloadReport>>,
    }

    impl CourseProgressSink for Rec {
        fn on_progress(&self, p: &DownloadProgress) {
            self.progress.lock().unwrap().push(p.clone());
        }
        fn on_report(&self, r: &DownloadReport) {
            self.reports.lock().unwrap().push(r.clone());
        }
    }

    fn req(url: &str, items: Option<&[&str]>, quality: Quality) -> Request {
        Request {
            url: url.into(),
            items: items.map(|i| i.iter().map(|s| (*s).to_string()).collect()),
            quality,
        }
    }

    async fn run(
        tool: FakeTool,
        ffmpeg: Option<PathBuf>,
        root: &Path,
        r: &Request,
    ) -> (Result<DownloadReport, YoutubeError>, Arc<FakeTool>, Rec) {
        let tool = Arc::new(tool);
        let rec = Rec::default();
        let y = Youtube::new(tool.clone(), ffmpeg);
        let out = y.download(r, root, &rec, &CancelFlag::new()).await;
        (out, tool, rec)
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    #[tokio::test]
    async fn a_single_video_lands_in_videos_sueltos_with_its_extras() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = FakeTool::new().with_probe(&watch(V), Ok(video(V, "Canción: ñ")));
        let (out, tool, rec) = run(
            tool,
            None,
            tmp.path(),
            &req(&format!("https://youtu.be/{V}"), None, Quality::P720),
        )
        .await;
        let report = out.unwrap();
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(
            (report.platform.as_str(), report.group.as_str()),
            ("youtube", watch(V).as_str())
        );
        assert_eq!((report.total_lessons, report.completed_lessons), (1, 1));
        let group = tmp.path().join("YouTube/Videos sueltos");
        assert!(group.join(".complete").is_file() && group.join("download-report.json").is_file());
        let lesson = group.join("01 - Canción_ ñ");
        assert_eq!(
            names(&lesson),
            [
                ".youtube-id",
                "Canción_ ñ.mp4",
                "description.txt",
                "thumbnail.jpg"
            ]
        );
        assert_eq!(
            std::fs::read_to_string(lesson.join(".youtube-id")).unwrap(),
            V
        );
        assert_eq!(tool.fetched_urls(), [watch(V)]);
        assert_eq!(rec.reports.lock().unwrap().len(), 1);
        let last = rec.progress.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.percent, last.completed, last.total), (100.0, 0, 1));
        assert_eq!(last.course, "Videos sueltos");
        assert_eq!(last.lesson, "01 - Canción_ ñ");
    }

    #[tokio::test]
    async fn the_next_single_video_gets_the_next_number_and_repeats_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = || {
            FakeTool::new()
                .with_probe(&watch(V), Ok(video(V, "Uno")))
                .with_probe(&watch(W), Ok(video(W, "Dos")))
        };
        run(mk(), None, tmp.path(), &req(&watch(V), None, Quality::P720))
            .await
            .0
            .unwrap();
        let (out, tool, _) =
            run(mk(), None, tmp.path(), &req(&watch(W), None, Quality::P720)).await;
        assert!(out.unwrap().complete);
        let group = tmp.path().join("YouTube/Videos sueltos");
        assert_eq!(
            names(&group)
                .into_iter()
                .filter(|n| n.contains(" - "))
                .collect::<Vec<_>>(),
            ["01 - Uno", "02 - Dos"]
        );
        assert_eq!(tool.fetched_urls(), [watch(W)]);
        let (out, tool, _) =
            run(mk(), None, tmp.path(), &req(&watch(V), None, Quality::P720)).await;
        let report = out.unwrap();
        assert!(tool.fetched_urls().is_empty(), "already downloaded");
        assert_eq!((report.total_lessons, report.completed_lessons), (1, 1));
        assert_eq!(
            names(&group).iter().filter(|n| n.contains(" - ")).count(),
            2
        );
    }

    #[tokio::test]
    async fn playlists_number_by_position_and_report_unavailable_videos() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = playlist(json!([
            {"id": V, "title": "Uno"},
            {"id": W, "title": "[Private video]"},
            {"id": "zzzzzzzzzzz", "title": "Tres"},
            null
        ]));
        let tool = FakeTool::new().with_probe(&list_url(), Ok(doc));
        let (out, tool, _) = run(
            tool,
            None,
            tmp.path(),
            &req(&list_url(), None, Quality::Best1080),
        )
        .await;
        let report = out.unwrap();
        let group = tmp.path().join("YouTube/Mi lista_ ñ");
        let lessons: Vec<_> = names(&group)
            .into_iter()
            .filter(|n| n.contains(" - "))
            .collect();
        assert_eq!(lessons, ["01 - Uno", "03 - Tres"]);
        assert_eq!(tool.fetched_urls(), [watch(V), watch("zzzzzzzzzzz")]);
        assert_eq!((report.total_lessons, report.completed_lessons), (4, 2));
        let unresolved: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::VideoNotResolved)
            .collect();
        assert_eq!(unresolved.len(), 2, "{:?}", report.issues);
        assert_eq!(unresolved[0].severity, Severity::Warning);
        assert_eq!(
            unresolved[0].lesson.as_deref(),
            Some("02 - [Private video]")
        );
        assert!(unresolved[0].message.contains("private video"));
        assert_eq!(
            unresolved[0].url.as_deref(),
            Some("https://www.youtube.com/watch")
        );
        assert!(
            unresolved[0].message.contains(W),
            "the id survives URL redaction"
        );
        assert_eq!(
            unresolved[1].lesson.as_deref(),
            Some("04 - Unavailable video")
        );
        assert_eq!(unresolved[1].url, None, "placeholders have no real id");
        assert!(report.complete && group.join(".complete").is_file());
    }

    #[tokio::test]
    async fn a_selection_downloads_only_those_items_and_flags_unknown_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = playlist(json!([
            {"id": V, "title": "Uno"},
            {"id": W, "title": "Dos"}
        ]));
        let tool = FakeTool::new().with_probe(&list_url(), Ok(doc));
        let r = req(&list_url(), Some(&[W, "nnnnnnnnnnn"]), Quality::P720);
        let (out, tool, _) = run(tool, None, tmp.path(), &r).await;
        let report = out.unwrap();
        assert_eq!(tool.fetched_urls(), [watch(W)]);
        let group = tmp.path().join("YouTube/Mi lista_ ñ");
        assert!(group.join("02 - Dos").is_dir() && !group.join("01 - Uno").exists());
        assert_eq!(report.total_lessons, 1);
        let bad = report
            .issues
            .iter()
            .find(|i| i.message.contains("nnnnnnnnnnn"))
            .unwrap();
        assert_eq!(
            (bad.kind, bad.severity),
            (IssueKind::VideoNotResolved, Severity::Error)
        );
        assert!(!report.complete);
    }

    #[tokio::test]
    async fn failed_items_are_reported_and_the_playlist_continues() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = playlist(json!([
            {"id": V, "title": "Uno"},
            {"id": W, "title": "Dos"},
            {"id": "zzzzzzzzzzz", "title": "Tres"}
        ]));
        let tool = FakeTool::new()
            .with_probe(&list_url(), Ok(doc))
            .with_fetch(|job| {
                if job.url.contains(V) {
                    return Err(VideoError::Failed(
                        "[youtube] x: Video unavailable. This video is private".into(),
                    ));
                }
                if job.url.contains(W) {
                    return Err(VideoError::Failed(
                        "HTTP Error 503: Service Unavailable".into(),
                    ));
                }
                crate::client::fake::write_media(job)
            });
        let (out, tool, _) = run(
            tool,
            None,
            tmp.path(),
            &req(&list_url(), None, Quality::P720),
        )
        .await;
        let report = out.unwrap();
        assert_eq!(tool.fetched_urls().len(), 3);
        assert_eq!((report.total_lessons, report.completed_lessons), (3, 1));
        let item_issues: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.lesson.is_some())
            .collect();
        let private = item_issues[0];
        assert_eq!(
            (private.kind, private.severity),
            (IssueKind::VideoNotResolved, Severity::Warning)
        );
        assert!(private.message.starts_with("private video:"));
        let transient = item_issues[1];
        assert_eq!(
            (transient.kind, transient.severity),
            (IssueKind::Video, Severity::Error)
        );
        assert!(transient.message.contains("503"));
        let group = tmp.path().join("YouTube/Mi lista_ ñ");
        assert!(!report.complete && !group.join(".complete").exists());
        assert!(!group.join("01 - Uno").exists() && !group.join("02 - Dos").exists());
        assert!(group.join("03 - Tres").is_dir());
    }

    #[tokio::test]
    async fn cancel_stops_the_queue_and_keeps_the_previous_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let group = tmp.path().join("YouTube/Mi lista_ ñ");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join(".complete"), "done").unwrap();
        let doc = playlist(json!([
            {"id": V, "title": "Uno"},
            {"id": W, "title": "Dos"},
            {"id": "zzzzzzzzzzz", "title": "Tres"}
        ]));
        let cancel = CancelFlag::new();
        let c2 = cancel.clone();
        let tool = Arc::new(FakeTool::new().with_probe(&list_url(), Ok(doc)).with_fetch(
            move |job| {
                c2.cancel();
                crate::client::fake::write_media(job)
            },
        ));
        let rec = Rec::default();
        let y = Youtube::new(tool.clone(), None);
        let report = y
            .download(
                &req(&list_url(), None, Quality::P720),
                tmp.path(),
                &rec,
                &cancel,
            )
            .await
            .unwrap();
        assert!(report.cancelled && !report.complete);
        assert_eq!(tool.fetched_urls().len(), 1);
        assert!(
            group.join(".complete").is_file(),
            "cancel keeps the old state"
        );
    }

    #[tokio::test]
    async fn quality_presets_and_the_ffmpeg_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = || FakeTool::new().with_probe(&watch(V), Ok(video(V, "T")));
        let (out, tool, _) = run(
            mk(),
            None,
            tmp.path(),
            &req(&watch(V), None, Quality::Best1080),
        )
        .await;
        let report = out.unwrap();
        let warn = report
            .issues
            .iter()
            .find(|i| i.message.contains("ffmpeg"))
            .unwrap();
        assert_eq!(
            (warn.kind, warn.severity),
            (IssueKind::Course, Severity::Warning)
        );
        assert!(report.complete);
        let args = tool.jobs.lock().unwrap()[0].args.join(" ");
        assert!(args.contains("-f b[height<=1080]/b") && !args.contains("ffmpeg-location"));
        assert!(args.contains("--write-description") && args.contains("--write-thumbnail"));
        assert!(!args.contains("--convert-thumbnails"));

        let tmp2 = tempfile::tempdir().unwrap();
        let ff = Some(PathBuf::from("/opt/ffmpeg"));
        let (out, tool, _) = run(
            mk(),
            ff.clone(),
            tmp2.path(),
            &req(&watch(V), None, Quality::Best1080),
        )
        .await;
        assert!(out.unwrap().issues.is_empty());
        let args = tool.jobs.lock().unwrap()[0].args.join(" ");
        assert!(
            args.contains("--merge-output-format mp4")
                && args.contains("--ffmpeg-location /opt/ffmpeg")
        );
        assert!(args.contains("--convert-thumbnails jpg"));

        let tmp3 = tempfile::tempdir().unwrap();
        let (out, tool, _) = run(
            mk(),
            None,
            tmp3.path(),
            &req(&watch(V), None, Quality::Audio),
        )
        .await;
        assert!(out.unwrap().issues.is_empty(), "audio needs no ffmpeg");
        assert!(tool.jobs.lock().unwrap()[0]
            .args
            .join(" ")
            .contains("-f ba[ext=m4a]/ba/b"));
    }

    #[tokio::test]
    async fn nothing_downloadable_is_an_error_without_creating_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let doc = playlist(json!([{"id": V, "title": "[Deleted video]"}]));
        let tool = FakeTool::new().with_probe(&list_url(), Ok(doc));
        let (out, tool, _) = run(
            tool,
            None,
            tmp.path(),
            &req(&list_url(), None, Quality::P720),
        )
        .await;
        let report = out.unwrap();
        assert!(tool.fetched_urls().is_empty());
        assert!(!report.complete);
        assert!(report.issues.iter().any(|i| i.severity == Severity::Error));
        assert!(!tmp.path().join("YouTube").exists());
    }

    #[tokio::test]
    async fn invalid_links_and_a_missing_binary_are_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let (out, tool, _) = run(
            FakeTool::new(),
            None,
            tmp.path(),
            &req("not a link", None, Quality::P720),
        )
        .await;
        assert_eq!(out.unwrap_err(), YoutubeError::InvalidUrl);
        assert!(tool.probed.lock().unwrap().is_empty());
        let tool = FakeTool::new()
            .with_probe(&watch(V), Ok(video(V, "T")))
            .with_fetch(|_| Err(VideoError::NotFound("/x/yt-dlp".into())));
        let (out, _, _) = run(tool, None, tmp.path(), &req(&watch(V), None, Quality::P720)).await;
        assert!(matches!(
            out.unwrap_err(),
            YoutubeError::Tool(VideoError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn long_titles_still_fit_the_path_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let long = "Título larguísimo ".repeat(30);
        let tool = FakeTool::new().with_probe(&watch(V), Ok(video(V, &long)));
        let (out, _, _) = run(tool, None, tmp.path(), &req(&watch(V), None, Quality::P720)).await;
        assert!(out.unwrap().complete);
        let group = tmp.path().join("YouTube/Videos sueltos");
        let lesson = names(&group)
            .into_iter()
            .find(|n| n.contains(" - "))
            .unwrap();
        assert!(lesson.encode_utf16().count() <= 80);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn end_to_end_through_a_fake_yt_dlp_script() {
        use lumen_download::YtDlp;
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("fake-yt-dlp");
        let flat = playlist(json!([
            {"id": V, "title": "Uno"},
            {"id": W, "title": "[Private video]"}
        ]));
        std::fs::write(
            &bin,
            format!(
                r#"#!/bin/sh
case "$*" in
  *-J*) echo '{flat}' ;;
  *)
    while [ $# -gt 0 ]; do if [ "$1" = "-o" ]; then out="$2"; fi; shift; done
    echo '[download]  50.0% of 1MiB'
    f=$(printf '%s' "$out" | sed 's/%(ext)s/mp4/'); echo v > "$f" ;;
esac
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let lib = tmp.path().join("lib");
        let y = Youtube::new(Arc::new(YtDlp::new(&bin)), None);
        let rec = Rec::default();
        let report = y
            .download(
                &req(&list_url(), None, Quality::P720),
                &lib,
                &rec,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert_eq!((report.total_lessons, report.completed_lessons), (2, 1));
        assert!(lib.join("YouTube/Mi lista_ ñ/01 - Uno/Uno.mp4").is_file());
        assert!(rec
            .progress
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.percent == 50.0));
    }
}
