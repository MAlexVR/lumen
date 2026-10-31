//! Course download orchestration. Lessons are independent units processed by
//! a bounded-concurrency loop (`max_parallel_lessons`, 1 = sequential); every
//! failure becomes an [`Issue`] in the report, nothing is swallowed.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::{self, StreamExt};
use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Issue,
    IssueKind, Severity,
};
use lumen_download::files::finished_file_exists;
use lumen_download::layout::path_units;
use lumen_download::report::finalize_folder;
use lumen_download::{VideoError, VideoJob, VideoRunner};

use crate::client::{HotmartClient, BROWSER_UA};
use crate::error::HotmartError;
use crate::naming::clean_name;
use crate::parse::{
    extract_iframes, panda_referer, panda_to_m3u8, parse_media_assets, readings_html,
    AttachmentLink, AttachmentRef, HotmartCourse, Iframe, IframeKind, LessonData, MediaAsset,
    MediaKind, MediaSrc, Navigation,
};
use crate::planner::{course_dir, plan_course, PlannedLesson};

/// Platform id used in reports and by the app shell.
pub const PLATFORM_ID: &str = "hotmart";

/// Everything the downloader needs from Hotmart (implemented by [`HotmartClient`]).
#[async_trait]
pub trait HotmartSource: Send + Sync {
    async fn product_referer(&self, product_id: &str) -> Result<Option<String>, HotmartError>;
    async fn navigation(&self, course: &HotmartCourse) -> Result<Navigation, HotmartError>;
    async fn lesson(&self, course: &HotmartCourse, hash: &str) -> Result<LessonData, HotmartError>;
    async fn attachment_link(
        &self,
        course: &HotmartCourse,
        attachment_id: &str,
    ) -> Result<AttachmentLink, HotmartError>;
    async fn drm_file_url(&self, lambda_url: &str, token: &str) -> Result<String, HotmartError>;
    async fn player_page(&self, url: &str) -> Result<String, HotmartError>;
    async fn download_file(&self, url: &str, dest: &Path) -> Result<(), HotmartError>;
}

#[async_trait]
impl HotmartSource for HotmartClient {
    async fn product_referer(&self, product_id: &str) -> Result<Option<String>, HotmartError> {
        HotmartClient::product_referer(self, product_id).await
    }
    async fn navigation(&self, course: &HotmartCourse) -> Result<Navigation, HotmartError> {
        HotmartClient::navigation(self, course).await
    }
    async fn lesson(&self, course: &HotmartCourse, hash: &str) -> Result<LessonData, HotmartError> {
        HotmartClient::lesson(self, course, hash).await
    }
    async fn attachment_link(
        &self,
        course: &HotmartCourse,
        attachment_id: &str,
    ) -> Result<AttachmentLink, HotmartError> {
        HotmartClient::attachment_link(self, course, attachment_id).await
    }
    async fn drm_file_url(&self, lambda_url: &str, token: &str) -> Result<String, HotmartError> {
        HotmartClient::drm_file_url(self, lambda_url, token).await
    }
    async fn player_page(&self, url: &str) -> Result<String, HotmartError> {
        HotmartClient::player_page(self, url).await
    }
    async fn download_file(&self, url: &str, dest: &Path) -> Result<(), HotmartError> {
        HotmartClient::download_file(self, url, dest).await
    }
}

/// Item-level retries (the previous Go implementation tried lessons and attachments
/// three times with a growing pause).
#[derive(Debug, Clone)]
pub struct Retry {
    pub attempts: u32,
    /// Pause after attempt `n` is `n * unit`.
    pub unit: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            attempts: 3,
            unit: Duration::from_secs(2),
        }
    }
}

pub struct CourseDownloader<'a> {
    pub source: &'a dyn HotmartSource,
    pub videos: &'a dyn VideoRunner,
    pub sink: &'a dyn CourseProgressSink,
    pub cancel: &'a CancelFlag,
    pub options: DownloadOptions,
    pub retry: Retry,
}

/// Values shared by every lesson of one run.
struct RunContext {
    course: HotmartCourse,
    /// Referer for embedded players (Vimeo and unknown hosts).
    referer: String,
    total: u32,
    finished: AtomicU32,
    /// Set when Hotmart rejected the token mid-run: nothing else is attempted.
    expired: AtomicBool,
}

struct LessonOutcome {
    order: usize,
    skipped: bool,
    issues: Vec<Issue>,
}

const VIDEO_EXTS: [&str; 3] = ["mp4", "mkv", "webm"];
const NATIVE_REFERER: &str = "https://cf-embed.play.hotmart.com/";

/// Transport hiccups and API messages may pass on a retry; the rest will not.
fn transient(e: &HotmartError) -> bool {
    match e {
        HotmartError::Api(_) | HotmartError::Network(_) => true,
        HotmartError::Http { status, .. } => *status == 429 || *status >= 500,
        _ => false,
    }
}

/// True when a finished video for item `n` exists: `<n>. Lesson.<ext>` or
/// the YouTube form `<n>. Lesson [<id>].<ext>` (never `.part` or `.fNNN.` leftovers).
fn numbered_video_exists(dir: &Path, n: usize) -> bool {
    let plain = format!("{n}. Lesson.");
    let tagged = format!("{n}. Lesson [");
    let is_video =
        |ext: &str| !ext.contains('.') && VIDEO_EXTS.contains(&ext.to_ascii_lowercase().as_str());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(ext) = name.strip_prefix(&plain) {
            return is_video(ext);
        }
        name.starts_with(&tagged) && name.rsplit_once("].").is_some_and(|(_, ext)| is_video(ext))
    })
}

/// Removes `*.part` files left by interrupted downloads, recursively.
fn remove_part_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_part_files(&path);
        } else if path.extension().is_some_and(|e| e == "part") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn io_err(e: &std::io::Error) -> HotmartError {
    HotmartError::Io(e.to_string())
}

/// The best video among the assets of a native player page: an HLS playlist
/// or a video type, otherwise the first asset (the previous choice).
fn pick_video_asset(assets: &[MediaAsset]) -> Option<&MediaAsset> {
    assets
        .iter()
        .find(|a| {
            let ct = a.content_type.as_deref().unwrap_or("").to_ascii_lowercase();
            ct.contains("mpegurl") || ct.contains("video") || a.url.contains(".m3u8")
        })
        .or_else(|| assets.first())
}

fn pick_audio_asset(assets: &[MediaAsset]) -> Option<&MediaAsset> {
    assets.iter().find(|a| {
        a.content_type
            .as_deref()
            .is_some_and(|c| c.to_ascii_lowercase().contains("audio"))
    })
}

/// File extension of a URL path (without query), if any.
fn url_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_lowercase())
}

impl CourseDownloader<'_> {
    /// Downloads every lesson of `course` under `library_root`. Fatal problems
    /// (unreadable navigation, no modules, expired session before the first
    /// lesson) return `Err`; everything else lands in the report.
    pub async fn run(
        &self,
        course: &HotmartCourse,
        library_root: &Path,
    ) -> Result<DownloadReport, HotmartError> {
        let nav = self.source.navigation(course).await?;
        let referer = match self.source.product_referer(&course.id).await {
            Ok(Some(r)) => r,
            Ok(None) => format!("https://{}.club.hotmart.com/", course.slug),
            Err(e @ HotmartError::Unauthorized(_)) => return Err(e),
            Err(_) => format!("https://{}.club.hotmart.com/", course.slug),
        };
        let dir = course_dir(library_root, course);
        std::fs::create_dir_all(&dir).map_err(|e| io_err(&e))?;
        remove_part_files(&dir);

        let plan = plan_course(&dir, &nav);
        let mut report = DownloadReport::new(PLATFORM_ID, &course.name);
        if plan.is_empty() {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "the course has modules but no lessons",
            ));
        }
        let total = u32::try_from(plan.len()).unwrap_or(u32::MAX);
        report.total_lessons = total;
        let ctx = RunContext {
            course: course.clone(),
            referer,
            total,
            finished: AtomicU32::new(0),
            expired: AtomicBool::new(false),
        };
        let parallel = self.options.max_parallel_lessons.max(1);
        let lessons: Vec<BoxFuture<'_, LessonOutcome>> = plan
            .iter()
            .map(|lesson| self.process_lesson(&ctx, lesson).boxed())
            .collect();
        let mut outcomes: Vec<LessonOutcome> = stream::iter(lessons)
            .buffer_unordered(parallel)
            .collect()
            .await;
        outcomes.sort_by_key(|o| o.order);

        for outcome in outcomes.into_iter().filter(|o| !o.skipped) {
            if !outcome.issues.iter().any(|i| i.severity == Severity::Error) {
                report.completed_lessons += 1;
            }
            outcome.issues.into_iter().for_each(|i| report.push(i));
        }
        if ctx.expired.load(Ordering::SeqCst) {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "session expired during the download: log in again and retry",
            ));
        }
        report.cancelled = self.cancel.is_cancelled();
        finalize_folder(&dir, &mut report);
        self.sink.on_report(&report);
        Ok(report)
    }

    fn emit(&self, ctx: &RunContext, lesson: &PlannedLesson, percent: f32) {
        self.sink.on_progress(&DownloadProgress {
            group: ctx.course.name.clone(),
            course: lesson.module_title.clone(),
            lesson: lesson.title.clone(),
            percent,
            completed: ctx.finished.load(Ordering::SeqCst),
            total: ctx.total,
        });
    }

    /// Holds while the download is paused, then reports whether it should stop.
    async fn checkpoint(&self, ctx: &RunContext) -> bool {
        lumen_download::wait_if_paused(self.cancel).await;
        self.stopped(ctx)
    }

    fn stopped(&self, ctx: &RunContext) -> bool {
        self.cancel.is_cancelled() || ctx.expired.load(Ordering::SeqCst)
    }

    async fn process_lesson(&self, ctx: &RunContext, lesson: &PlannedLesson) -> LessonOutcome {
        if self.checkpoint(ctx).await {
            return LessonOutcome {
                order: lesson.order,
                skipped: true,
                issues: Vec::new(),
            };
        }
        self.emit(ctx, lesson, 0.0);
        let issues = self.lesson_issues(ctx, lesson).await;
        ctx.finished.fetch_add(1, Ordering::SeqCst);
        self.emit(ctx, lesson, 100.0);
        LessonOutcome {
            order: lesson.order,
            skipped: false,
            issues,
        }
    }

    /// Runs `op` up to `retry.attempts` times, pausing between attempts, but
    /// only for transient errors; an expired session stops the whole run.
    async fn attempt<T, F, Fut>(&self, ctx: &RunContext, mut op: F) -> Result<T, HotmartError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, HotmartError>>,
    {
        let mut n = 0;
        loop {
            n += 1;
            match op().await {
                Ok(v) => return Ok(v),
                Err(e @ HotmartError::Unauthorized(_)) => {
                    ctx.expired.store(true, Ordering::SeqCst);
                    return Err(e);
                }
                Err(e) if n >= self.retry.attempts || !transient(&e) => return Err(e),
                Err(_) => tokio::time::sleep(self.retry.unit * n).await,
            }
        }
    }

    async fn lesson_issues(&self, ctx: &RunContext, lesson: &PlannedLesson) -> Vec<Issue> {
        let label = lesson.label();
        let label = Some(label.as_str());
        let mut issues = Vec::new();
        if lesson.hash.is_empty() {
            issues.push(Issue::error(
                IssueKind::ModulePage,
                label,
                None,
                "the page has no hash in the course navigation and cannot be fetched",
            ));
            return issues;
        }
        let data = self
            .attempt(ctx, || self.source.lesson(&ctx.course, &lesson.hash))
            .await;
        let data = match data {
            Ok(d) => d,
            Err(e) => {
                issues.push(Issue::error(
                    IssueKind::ModulePage,
                    label,
                    None,
                    e.to_string(),
                ));
                return issues;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&lesson.dir) {
            issues.push(Issue::error(
                IssueKind::Course,
                label,
                None,
                format!("cannot create the lesson folder: {e}"),
            ));
            return issues;
        }

        let iframes = extract_iframes(&data.content);
        let has_content = !data.content.trim().is_empty();
        if data.attachments.is_empty()
            && data.medias.is_empty()
            && iframes.is_empty()
            && !has_content
            && data.readings.is_empty()
        {
            issues.push(Issue::warning(
                IssueKind::EmptyLesson,
                label,
                None,
                "no description, video or materials",
            ));
        }

        for att in &data.attachments {
            if self.checkpoint(ctx).await {
                return issues;
            }
            self.download_attachment(ctx, lesson, att, &mut issues)
                .await;
        }

        if data.has_player_media {
            for (i, media) in data.medias.iter().enumerate() {
                if self.checkpoint(ctx).await {
                    return issues;
                }
                let n = i + 1;
                match media.kind {
                    MediaKind::Video => {
                        self.native_video(ctx, lesson, n, media, &mut issues).await;
                    }
                    MediaKind::Audio => {
                        self.native_audio(ctx, lesson, n, media, &mut issues).await;
                    }
                    MediaKind::Other => issues.push(Issue::warning(
                        IssueKind::Resource,
                        label,
                        Some(&media.url),
                        format!("unsupported media type for \"{}\"", media.name),
                    )),
                }
            }
        }

        // Embedded players continue the numbering after the native media.
        let offset = if data.has_player_media {
            data.medias.len()
        } else {
            0
        };
        for (i, iframe) in iframes.iter().enumerate() {
            if self.checkpoint(ctx).await {
                return issues;
            }
            let n = offset + i + 1;
            if numbered_video_exists(&lesson.dir, n) {
                continue;
            }
            let job = self.iframe_job(ctx, lesson, n, iframe);
            self.run_video(ctx, lesson, n, job, &mut issues).await;
        }

        write_once(
            &lesson.dir.join("Description.html"),
            has_content.then_some(data.content.as_str()),
            label,
            &mut issues,
        );
        let readings = readings_html(&data.readings);
        write_once(
            &lesson.dir.join("Supplementary reading.html"),
            (!readings.is_empty()).then_some(readings.as_str()),
            label,
            &mut issues,
        );
        issues
    }

    fn iframe_job(
        &self,
        ctx: &RunContext,
        lesson: &PlannedLesson,
        n: usize,
        iframe: &Iframe,
    ) -> VideoJob {
        let template = |name: String| lesson.dir.join(name);
        match iframe.kind {
            IframeKind::Panda => VideoJob {
                url: panda_to_m3u8(&iframe.url),
                referer: Some(panda_referer(&iframe.url)),
                user_agent: Some(BROWSER_UA.into()),
                output_template: template(format!("{n}. Lesson.%(ext)s")),
            },
            IframeKind::YouTube => VideoJob {
                url: iframe.url.clone(),
                referer: None,
                user_agent: None,
                output_template: template(format!("{n}. Lesson [%(id)s].%(ext)s")),
            },
            IframeKind::Vimeo | IframeKind::Other => VideoJob {
                url: iframe.url.clone(),
                referer: Some(ctx.referer.clone()),
                user_agent: Some(BROWSER_UA.into()),
                output_template: template(format!("{n}. Lesson.%(ext)s")),
            },
        }
    }

    async fn run_video(
        &self,
        ctx: &RunContext,
        lesson: &PlannedLesson,
        n: usize,
        job: VideoJob,
        issues: &mut Vec<Issue>,
    ) {
        let label = lesson.label();
        let on_percent = |p: f32| self.emit(ctx, lesson, p);
        match self.videos.download(&job, self.cancel, &on_percent).await {
            Ok(()) if numbered_video_exists(&lesson.dir, n) => {}
            Ok(()) => issues.push(Issue::error(
                IssueKind::Video,
                Some(&label),
                Some(&job.url),
                "yt-dlp finished but no video file was found",
            )),
            Err(VideoError::Cancelled) => {}
            Err(e) => issues.push(Issue::error(
                IssueKind::Video,
                Some(&label),
                Some(&job.url),
                e.to_string(),
            )),
        }
    }

    /// Assets of a native player page, or the issue explaining why not.
    async fn player_assets(
        &self,
        media: &MediaSrc,
        label: &str,
        kind: IssueKind,
    ) -> Result<Vec<MediaAsset>, Issue> {
        let page = self
            .source
            .player_page(&media.url)
            .await
            .map_err(|e| Issue::error(kind, Some(label), Some(&media.url), e.to_string()))?;
        parse_media_assets(&page)
            .map_err(|e| Issue::error(kind, Some(label), Some(&media.url), e.to_string()))
    }

    async fn native_video(
        &self,
        ctx: &RunContext,
        lesson: &PlannedLesson,
        n: usize,
        media: &MediaSrc,
        issues: &mut Vec<Issue>,
    ) {
        if numbered_video_exists(&lesson.dir, n) {
            return;
        }
        let label = lesson.label();
        let assets = match self
            .player_assets(media, &label, IssueKind::VideoNotResolved)
            .await
        {
            Ok(a) => a,
            Err(issue) => return issues.push(issue),
        };
        let Some(asset) = pick_video_asset(&assets) else {
            return issues.push(Issue::error(
                IssueKind::VideoNotResolved,
                Some(&label),
                Some(&media.url),
                "the player page has no media assets",
            ));
        };
        let job = VideoJob {
            url: asset.url.clone(),
            referer: Some(NATIVE_REFERER.into()),
            user_agent: Some(BROWSER_UA.into()),
            output_template: lesson.dir.join(format!("{n}. Lesson.%(ext)s")),
        };
        self.run_video(ctx, lesson, n, job, issues).await;
    }

    async fn native_audio(
        &self,
        ctx: &RunContext,
        lesson: &PlannedLesson,
        n: usize,
        media: &MediaSrc,
        issues: &mut Vec<Issue>,
    ) {
        let label = lesson.label();
        let budget = name_budget(path_units(&lesson.dir), 6).min(120);
        let stem = sanitize_file_name(&format!("{n}. {}", clean_name(&media.name)), budget);
        let base = lesson.dir.join(&stem);
        if base.exists() || finished_file_exists(&base) {
            return;
        }
        let assets = match self.player_assets(media, &label, IssueKind::Resource).await {
            Ok(a) => a,
            Err(issue) => return issues.push(issue),
        };
        let Some(asset) = pick_audio_asset(&assets) else {
            return issues.push(Issue::error(
                IssueKind::Resource,
                Some(&label),
                Some(&media.url),
                "the player page has no audio asset",
            ));
        };
        // Keep the original name; add an extension only when the name has none.
        let dest = match (Path::new(&stem).extension(), url_extension(&asset.url)) {
            (None, Some(ext)) => lesson.dir.join(format!("{stem}.{ext}")),
            _ => base,
        };
        if let Err(e) = self
            .attempt(ctx, || self.source.download_file(&asset.url, &dest))
            .await
        {
            issues.push(Issue::error(
                IssueKind::Resource,
                Some(&label),
                Some(&asset.url),
                e.to_string(),
            ));
        }
    }

    async fn download_attachment(
        &self,
        ctx: &RunContext,
        lesson: &PlannedLesson,
        att: &AttachmentRef,
        issues: &mut Vec<Issue>,
    ) {
        let label = lesson.label();
        let materials = lesson.dir.join("Materials");
        let budget = name_budget(path_units(&materials), 5).min(150);
        let name = sanitize_file_name(&clean_name(&att.name), budget);
        let save = materials.join(&name);
        let drm = materials.join(format!("drm_{name}"));
        if save.exists() || drm.exists() {
            return;
        }
        let result = self
            .attempt(ctx, || async {
                match self.source.attachment_link(&ctx.course, &att.id).await? {
                    AttachmentLink::Direct { url } => self.source.download_file(&url, &save).await,
                    AttachmentLink::Drm { lambda_url, token } => {
                        let file = self.source.drm_file_url(&lambda_url, &token).await?;
                        self.source.download_file(&file, &drm).await
                    }
                }
            })
            .await;
        if let Err(e) = result {
            issues.push(Issue::error(
                IssueKind::Resource,
                Some(&label),
                None,
                format!("cannot download \"{name}\": {e}"),
            ));
        }
    }
}

/// Writes `body` to `path` unless it is `None` or the file already exists.
fn write_once(path: &Path, body: Option<&str>, label: Option<&str>, issues: &mut Vec<Issue>) {
    let Some(body) = body else {
        return;
    };
    if path.exists() {
        return;
    }
    if let Err(e) = std::fs::write(path, body) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        issues.push(Issue::error(
            IssueKind::Course,
            label,
            None,
            format!("cannot write {name}: {e}"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{AttachmentRef, MediaKind, MediaSrc, NavModule, NavPage, Reading};
    use lumen_core::{DownloadProgress, IssueKind, Severity};
    use lumen_download::{VideoError, VideoJob};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::Mutex;

    const PLAYER: &str = include_str!("../tests/fixtures/player.html");
    const PLAYER_AUDIO: &str = include_str!("../tests/fixtures/player_audio.html");
    const M3U8: &str = "https://cdn.example.test/video/master.m3u8?t=SECRET";

    type Lessons = Mutex<HashMap<String, VecDeque<Result<LessonData, HotmartError>>>>;

    #[derive(Default)]
    struct FakeSource {
        referer: Option<String>,
        nav: Option<Result<Navigation, HotmartError>>,
        lessons: Lessons,
        attachments: HashMap<String, Result<AttachmentLink, HotmartError>>,
        players: HashMap<String, Result<String, HotmartError>>,
        failing_downloads: HashSet<String>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeSource {
        fn lesson_data(&self, hash: &str, seq: Vec<Result<LessonData, HotmartError>>) {
            self.lessons.lock().unwrap().insert(hash.into(), seq.into());
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn count(&self, prefix: &str) -> usize {
            self.calls()
                .iter()
                .filter(|c| c.starts_with(prefix))
                .count()
        }
    }

    #[async_trait]
    impl HotmartSource for FakeSource {
        async fn product_referer(&self, _: &str) -> Result<Option<String>, HotmartError> {
            Ok(self.referer.clone())
        }
        async fn navigation(&self, _: &HotmartCourse) -> Result<Navigation, HotmartError> {
            self.nav
                .clone()
                .unwrap_or(Err(HotmartError::Shape("no nav".into())))
        }
        async fn lesson(&self, _: &HotmartCourse, hash: &str) -> Result<LessonData, HotmartError> {
            self.calls.lock().unwrap().push(format!("lesson:{hash}"));
            let mut all = self.lessons.lock().unwrap();
            let q = all
                .get_mut(hash)
                .ok_or(HotmartError::Shape("no lesson".into()))?;
            if q.len() > 1 {
                q.pop_front()
                    .unwrap_or(Err(HotmartError::Shape("empty".into())))
            } else {
                q.front()
                    .cloned()
                    .unwrap_or(Err(HotmartError::Shape("empty".into())))
            }
        }
        async fn attachment_link(
            &self,
            _: &HotmartCourse,
            id: &str,
        ) -> Result<AttachmentLink, HotmartError> {
            self.calls.lock().unwrap().push(format!("attachment:{id}"));
            self.attachments
                .get(id)
                .cloned()
                .unwrap_or(Err(HotmartError::Http {
                    status: 404,
                    url: "x".into(),
                }))
        }
        async fn drm_file_url(&self, lambda: &str, token: &str) -> Result<String, HotmartError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("drm:{lambda}:{token}"));
            Ok("https://files.example.test/secret.pdf".into())
        }
        async fn player_page(&self, url: &str) -> Result<String, HotmartError> {
            self.calls.lock().unwrap().push(format!("player:{url}"));
            self.players
                .get(url)
                .cloned()
                .unwrap_or(Err(HotmartError::Http {
                    status: 404,
                    url: url.into(),
                }))
        }
        async fn download_file(&self, url: &str, dest: &Path) -> Result<(), HotmartError> {
            self.calls.lock().unwrap().push(format!("dl:{url}"));
            if self.failing_downloads.iter().any(|f| url.contains(f)) {
                return Err(HotmartError::Network("boom".into()));
            }
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(dest, "file").map_err(|e| HotmartError::Io(e.to_string()))
        }
    }

    #[derive(Default)]
    struct FakeVideo {
        jobs: Mutex<Vec<VideoJob>>,
        failing: Vec<String>,
        cancel_on_first: Option<CancelFlag>,
    }

    #[async_trait]
    impl VideoRunner for FakeVideo {
        async fn download(
            &self,
            job: &VideoJob,
            _: &CancelFlag,
            on_percent: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            self.jobs.lock().unwrap().push(job.clone());
            if let Some(c) = &self.cancel_on_first {
                c.cancel();
            }
            on_percent(50.0);
            if self.failing.iter().any(|f| job.url.contains(f)) {
                return Err(VideoError::Failed("HTTP Error 403: Forbidden".into()));
            }
            let file = job
                .output_template
                .to_string_lossy()
                .replace("%(ext)s", "mp4")
                .replace("%(id)s", "YT");
            std::fs::write(file, "v").map_err(|e| VideoError::Failed(e.to_string()))
        }
    }

    #[derive(Default)]
    struct Collect {
        progress: Mutex<Vec<DownloadProgress>>,
        reports: Mutex<Vec<DownloadReport>>,
    }

    impl CourseProgressSink for Collect {
        fn on_progress(&self, p: &DownloadProgress) {
            self.progress.lock().unwrap().push(p.clone());
        }
        fn on_report(&self, r: &DownloadReport) {
            self.reports.lock().unwrap().push(r.clone());
        }
    }

    fn course() -> HotmartCourse {
        HotmartCourse {
            id: "1001".into(),
            name: "Horizonte".into(),
            slug: "horizonte-club".into(),
            seller: Some("Marta".into()),
            cover_url: None,
            purchase_date: None,
        }
    }

    fn nav(pages: &[(&str, &str)]) -> Navigation {
        Navigation {
            modules: vec![NavModule {
                name: "Mod".into(),
                pages: pages
                    .iter()
                    .map(|(hash, name)| NavPage {
                        hash: (*hash).into(),
                        name: (*name).into(),
                    })
                    .collect(),
            }],
        }
    }

    fn lesson_dir(root: &Path, n: usize, name: &str) -> PathBuf {
        root.join(format!("Hotmart/Horizonte - Marta/1. Mod/{n}. {name}"))
    }

    fn quick() -> Retry {
        Retry {
            attempts: 3,
            unit: Duration::ZERO,
        }
    }

    async fn run_with(
        source: &FakeSource,
        videos: &FakeVideo,
        root: &Path,
        parallel: usize,
        cancel: &CancelFlag,
    ) -> (Result<DownloadReport, HotmartError>, Collect) {
        let sink = Collect::default();
        let d = CourseDownloader {
            source,
            videos,
            sink: &sink,
            cancel,
            options: DownloadOptions {
                max_parallel_lessons: parallel,
            },
            retry: quick(),
        };
        let r = d.run(&course(), root).await;
        (r, sink)
    }

    async fn run(source: &FakeSource, videos: &FakeVideo, root: &Path) -> DownloadReport {
        run_with(source, videos, root, 1, &CancelFlag::new())
            .await
            .0
            .unwrap()
    }

    fn simple_lesson(content: &str) -> LessonData {
        LessonData {
            content: content.into(),
            ..LessonData::default()
        }
    }

    fn full_lesson() -> LessonData {
        LessonData {
            attachments: vec![
                AttachmentRef {
                    id: "a1".into(),
                    name: "Plan: 2021.pdf".into(),
                },
                AttachmentRef {
                    id: "a2".into(),
                    name: "Secret.pdf".into(),
                },
            ],
            has_player_media: true,
            medias: vec![
                MediaSrc {
                    kind: MediaKind::Video,
                    url: "https://play.test/v".into(),
                    name: "Intro".into(),
                },
                MediaSrc {
                    kind: MediaKind::Audio,
                    url: "https://play.test/a".into(),
                    name: "Podcast.mp3".into(),
                },
            ],
            content: "<p>Hi</p>\
                <iframe src=\"https://player.vimeo.com/video/1\"></iframe>\
                <iframe src=\"https://player-vz-x.tv.pandavideo.com.br/embed/?v=u1\"></iframe>\
                <iframe src=\"https://www.youtube.com/embed/YT\"></iframe>"
                .into(),
            readings: vec![Reading {
                url: "https://read.test/a".into(),
                name: "Read".into(),
            }],
        }
    }

    fn full_source() -> FakeSource {
        let mut s = FakeSource {
            referer: Some("https://area.example.test/".into()),
            nav: Some(Ok(nav(&[("h1", "Lesson")]))),
            ..FakeSource::default()
        };
        s.lesson_data("h1", vec![Ok(full_lesson())]);
        s.attachments.insert(
            "a1".into(),
            Ok(AttachmentLink::Direct {
                url: "https://files.example.test/plan.pdf?sig=S".into(),
            }),
        );
        s.attachments.insert(
            "a2".into(),
            Ok(AttachmentLink::Drm {
                lambda_url: "https://lambda.example.test/d".into(),
                token: "DT".into(),
            }),
        );
        s.players
            .insert("https://play.test/v".into(), Ok(PLAYER.into()));
        s.players
            .insert("https://play.test/a".into(), Ok(PLAYER_AUDIO.into()));
        s
    }

    fn sorted_files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let name = format!("{prefix}{}", e.file_name().to_string_lossy());
                if e.path().is_dir() {
                    walk(&e.path(), &format!("{name}/"), out);
                } else {
                    out.push(name);
                }
            }
        }
        walk(dir, "", &mut out);
        out.sort();
        out
    }

    #[tokio::test]
    async fn a_full_lesson_lands_in_the_established_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let (source, videos) = (full_source(), FakeVideo::default());
        let report = run(&source, &videos, tmp.path()).await;
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!((report.total_lessons, report.completed_lessons), (1, 1));
        assert_eq!(report.platform, "hotmart");
        assert_eq!(report.group, "Horizonte");

        let course_dir = tmp.path().join("Hotmart/Horizonte - Marta");
        assert!(course_dir.join(".complete").is_file());
        assert!(course_dir.join("download-report.json").is_file());
        let lesson = lesson_dir(tmp.path(), 1, "Lesson");
        assert_eq!(
            sorted_files(&lesson),
            [
                "1. Lesson.mp4",
                "2. Podcast.mp3",
                "3. Lesson.mp4",
                "4. Lesson.mp4",
                "5. Lesson [YT].mp4",
                "Description.html",
                "Materials/Plan 2021.pdf",
                "Materials/drm_Secret.pdf",
                "Supplementary reading.html",
            ]
        );
        assert!(std::fs::read_to_string(lesson.join("Description.html"))
            .unwrap()
            .starts_with("<p>Hi</p>"));
        assert!(
            std::fs::read_to_string(lesson.join("Supplementary reading.html"))
                .unwrap()
                .contains("<a href=\"https://read.test/a\">Read</a>")
        );

        let jobs = videos.jobs.lock().unwrap().clone();
        assert_eq!(jobs.len(), 4);
        assert_eq!(jobs[0].url, M3U8);
        assert_eq!(
            jobs[0].referer.as_deref(),
            Some("https://cf-embed.play.hotmart.com/")
        );
        assert_eq!(
            jobs[0].user_agent.as_deref(),
            Some(crate::client::BROWSER_UA)
        );
        assert!(jobs[0].output_template.ends_with("1. Lesson.%(ext)s"));
        assert_eq!(jobs[1].url, "https://player.vimeo.com/video/1");
        assert_eq!(
            jobs[1].referer.as_deref(),
            Some("https://area.example.test/")
        );
        assert!(jobs[1].output_template.ends_with("3. Lesson.%(ext)s"));
        assert_eq!(
            jobs[2].url,
            "https://b-vz-x.tv.pandavideo.com.br/u1/playlist.m3u8"
        );
        assert_eq!(
            jobs[2].referer.as_deref(),
            Some("https://player-vz-x.tv.pandavideo.com.br")
        );
        assert_eq!(jobs[3].url, "https://www.youtube.com/embed/YT");
        assert_eq!(jobs[3].referer, None);
        assert!(jobs[3]
            .output_template
            .ends_with("5. Lesson [%(id)s].%(ext)s"));

        let calls = source.calls();
        assert!(calls.contains(&"dl:https://files.example.test/plan.pdf?sig=S".to_string()));
        assert!(calls.contains(&"drm:https://lambda.example.test/d:DT".to_string()));
        assert!(calls.contains(&"dl:https://files.example.test/secret.pdf".to_string()));
        assert!(calls.contains(&"dl:https://cdn.example.test/audio/track.mp3".to_string()));
    }

    #[tokio::test]
    async fn a_second_run_downloads_nothing_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (source, videos) = (full_source(), FakeVideo::default());
        run(&source, &videos, tmp.path()).await;
        let (downloads, jobs) = (source.count("dl:"), videos.jobs.lock().unwrap().len());
        let report = run(&source, &videos, tmp.path()).await;
        assert!(report.complete);
        assert_eq!(source.count("dl:"), downloads);
        assert_eq!(videos.jobs.lock().unwrap().len(), jobs);
        assert_eq!(
            source.count("lesson:h1"),
            2,
            "the lesson is re-read so missing pieces are found"
        );
    }

    #[tokio::test]
    async fn numbered_videos_are_recognised_by_their_numbered_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = lesson_dir(tmp.path(), 1, "Lesson");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1. Lesson.mp4"), "old").unwrap();
        std::fs::write(dir.join("2. Lesson [abc].mp4"), "old").unwrap();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "Lesson")]))),
            ..FakeSource::default()
        };
        source.lesson_data(
            "h1",
            vec![Ok(LessonData {
                content: "<iframe src=\"https://player.vimeo.com/video/1\"></iframe>\
                          <iframe src=\"https://www.youtube.com/embed/abc\"></iframe>"
                    .into(),
                ..LessonData::default()
            })],
        );
        let videos = FakeVideo::default();
        let report = run(&source, &videos, tmp.path()).await;
        assert!(
            videos.jobs.lock().unwrap().is_empty(),
            "both videos already exist"
        );
        assert!(report.complete);
    }

    #[tokio::test]
    async fn lesson_api_errors_are_retried_then_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "L")]))),
            ..FakeSource::default()
        };
        source.lesson_data("h1", vec![Err(HotmartError::Api("locked".into()))]);
        let report = run(&source, &FakeVideo::default(), tmp.path()).await;
        assert_eq!(source.count("lesson:h1"), 3);
        assert!(!report.complete);
        let i = &report.issues[0];
        assert_eq!(
            (i.kind, i.severity),
            (IssueKind::ModulePage, Severity::Error)
        );
        assert!(i.message.contains("locked"));
        assert_eq!(i.lesson.as_deref(), Some("1. Mod / 1. L"));
        assert!(!tmp
            .path()
            .join("Hotmart/Horizonte - Marta/.complete")
            .exists());
        assert_eq!(report.completed_lessons, 0);
    }

    #[tokio::test]
    async fn a_lesson_that_recovers_on_retry_is_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "L")]))),
            ..FakeSource::default()
        };
        source.lesson_data(
            "h1",
            vec![
                Err(HotmartError::Api("busy".into())),
                Ok(simple_lesson("<p>x</p>")),
            ],
        );
        let report = run(&source, &FakeVideo::default(), tmp.path()).await;
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(source.count("lesson:h1"), 2);
    }

    #[tokio::test]
    async fn pages_without_a_hash_are_reported_not_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("", "Orphan")]))),
            ..FakeSource::default()
        };
        let report = run(&source, &FakeVideo::default(), tmp.path()).await;
        assert!(!report.complete);
        assert_eq!(report.issues[0].kind, IssueKind::ModulePage);
        assert!(report.issues[0].message.contains("no hash"));
        assert_eq!(source.count("lesson:"), 0);
    }

    #[tokio::test]
    async fn a_failing_attachment_is_retried_then_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = full_source();
        source.failing_downloads.insert("plan.pdf".into());
        let report = run(&source, &FakeVideo::default(), tmp.path()).await;
        assert_eq!(source.count("attachment:a1"), 3);
        assert!(!report.complete);
        let i = report
            .issues
            .iter()
            .find(|i| i.kind == IssueKind::Resource)
            .unwrap();
        assert_eq!(i.severity, Severity::Error);
        assert!(i.message.contains("Plan 2021.pdf"));
        let url = i.url.clone().unwrap_or_default();
        assert!(!url.contains("sig=S"), "signed URLs are redacted: {url}");
        assert!(lesson_dir(tmp.path(), 1, "Lesson")
            .join("Materials/drm_Secret.pdf")
            .is_file());
    }

    #[tokio::test]
    async fn video_failures_carry_the_ytdlp_message_and_a_redacted_url() {
        let tmp = tempfile::tempdir().unwrap();
        let source = full_source();
        let videos = FakeVideo {
            failing: vec!["master.m3u8".into(), "vimeo".into()],
            ..FakeVideo::default()
        };
        let report = run(&source, &videos, tmp.path()).await;
        assert!(!report.complete);
        let video_errors: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Video)
            .collect();
        assert_eq!(video_errors.len(), 2);
        assert!(video_errors.iter().all(|i| i.message.contains("403")));
        let native = video_errors[0];
        assert_eq!(
            native.url.as_deref(),
            Some("https://cdn.example.test/video/master.m3u8")
        );
        assert_eq!(native.host.as_deref(), Some("cdn.example.test"));
        let lesson = lesson_dir(tmp.path(), 1, "Lesson");
        assert!(
            lesson.join("4. Lesson.mp4").is_file(),
            "other videos still download"
        );
    }

    #[tokio::test]
    async fn unresolvable_native_players_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = full_source();
        source.players.remove("https://play.test/v");
        source.players.insert(
            "https://play.test/a".into(),
            Ok("<html>no data</html>".into()),
        );
        let report = run(&source, &FakeVideo::default(), tmp.path()).await;
        let kinds: Vec<_> = report.issues.iter().map(|i| (i.kind, i.severity)).collect();
        assert!(
            kinds.contains(&(IssueKind::VideoNotResolved, Severity::Error)),
            "{kinds:?}"
        );
        assert!(
            kinds.contains(&(IssueKind::Resource, Severity::Error)),
            "audio: {kinds:?}"
        );
        assert!(!report.complete);
    }

    #[tokio::test]
    async fn empty_lessons_and_unsupported_media_are_only_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "Empty"), ("h2", "Image")]))),
            ..FakeSource::default()
        };
        source.lesson_data("h1", vec![Ok(LessonData::default())]);
        source.lesson_data(
            "h2",
            vec![Ok(LessonData {
                has_player_media: true,
                medias: vec![MediaSrc {
                    kind: MediaKind::Other,
                    url: "https://x.test/i".into(),
                    name: "pic".into(),
                }],
                content: "<p>text</p>".into(),
                ..LessonData::default()
            })],
        );
        let report = run(&source, &FakeVideo::default(), tmp.path()).await;
        assert!(report.complete, "{:?}", report.issues);
        assert_eq!(report.completed_lessons, 2);
        let kinds: Vec<_> = report.issues.iter().map(|i| (i.kind, i.severity)).collect();
        assert_eq!(
            kinds,
            [
                (IssueKind::EmptyLesson, Severity::Warning),
                (IssueKind::Resource, Severity::Warning)
            ]
        );
    }

    #[tokio::test]
    async fn missing_referer_falls_back_to_the_club_address() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = full_source();
        source.referer = None;
        let videos = FakeVideo::default();
        run(&source, &videos, tmp.path()).await;
        let jobs = videos.jobs.lock().unwrap().clone();
        assert_eq!(
            jobs[1].referer.as_deref(),
            Some("https://horizonte-club.club.hotmart.com/")
        );
    }

    #[tokio::test]
    async fn a_paused_run_holds_before_the_next_lesson_until_resumed() {
        let tmp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        cancel.pause();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "A")]))),
            ..FakeSource::default()
        };
        source.lesson_data("h1", vec![Ok(simple_lesson("<p>a</p>"))]);
        let videos = FakeVideo::default();
        let run = run_with(&source, &videos, tmp.path(), 1, &cancel);
        tokio::pin!(run);
        let held = tokio::time::timeout(std::time::Duration::from_millis(300), &mut run).await;
        assert!(held.is_err(), "the run holds while paused");
        assert_eq!(source.count("lesson:h1"), 0);
        cancel.resume();
        let (report, _) = run.await;
        assert_eq!(report.unwrap().completed_lessons, 1);
        assert_eq!(source.count("lesson:h1"), 1);
    }

    #[tokio::test]
    async fn cancel_stops_the_run_and_never_marks_it_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "A"), ("h2", "B")]))),
            ..FakeSource::default()
        };
        let with_iframe =
            simple_lesson("<iframe src=\"https://player.vimeo.com/video/1\"></iframe>");
        source.lesson_data("h1", vec![Ok(with_iframe.clone())]);
        source.lesson_data("h2", vec![Ok(with_iframe)]);
        let videos = FakeVideo {
            cancel_on_first: Some(cancel.clone()),
            ..FakeVideo::default()
        };
        let (report, _) = run_with(&source, &videos, tmp.path(), 1, &cancel).await;
        let report = report.unwrap();
        assert!(report.cancelled && !report.complete);
        assert_eq!(source.count("lesson:h2"), 0);
        assert!(!tmp
            .path()
            .join("Hotmart/Horizonte - Marta/.complete")
            .exists());
    }

    #[tokio::test]
    async fn an_expired_session_stops_the_run_with_one_clear_issue() {
        let tmp = tempfile::tempdir().unwrap();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "A"), ("h2", "B"), ("h3", "C")]))),
            ..FakeSource::default()
        };
        source.lesson_data("h1", vec![Ok(simple_lesson("<p>a</p>"))]);
        source.lesson_data("h2", vec![Err(HotmartError::Unauthorized(401))]);
        source.lesson_data("h3", vec![Ok(simple_lesson("<p>c</p>"))]);
        let report = run(&source, &FakeVideo::default(), tmp.path()).await;
        assert_eq!(source.count("lesson:h2"), 1, "401 is not retried");
        assert_eq!(
            source.count("lesson:h3"),
            0,
            "later lessons are not attempted"
        );
        assert!(!report.complete);
        assert_eq!(report.completed_lessons, 1);
        let course_issue = report
            .issues
            .iter()
            .find(|i| i.kind == IssueKind::Course)
            .unwrap();
        assert!(course_issue.message.contains("session expired"));
    }

    #[tokio::test]
    async fn unreadable_navigation_is_fatal_and_creates_nothing() {
        for nav in [
            Err(HotmartError::Unauthorized(401)),
            Err(HotmartError::Shape("course has no modules".into())),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let source = FakeSource {
                nav: Some(nav.clone()),
                ..FakeSource::default()
            };
            let (r, _) = run_with(
                &source,
                &FakeVideo::default(),
                tmp.path(),
                1,
                &CancelFlag::new(),
            )
            .await;
            assert_eq!(r.unwrap_err(), nav.unwrap_err());
            assert!(!tmp.path().join("Hotmart").exists());
        }
    }

    #[tokio::test]
    async fn parallel_runs_process_every_lesson_and_keep_issue_order() {
        let tmp = tempfile::tempdir().unwrap();
        let pages: Vec<(String, String)> = (1..=6)
            .map(|i| (format!("h{i}"), format!("L{i}")))
            .collect();
        let refs: Vec<(&str, &str)> = pages
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let source = FakeSource {
            nav: Some(Ok(nav(&refs))),
            ..FakeSource::default()
        };
        for (h, _) in &pages {
            source.lesson_data(h, vec![Ok(LessonData::default())]);
        }
        let (r, sink) = run_with(
            &source,
            &FakeVideo::default(),
            tmp.path(),
            3,
            &CancelFlag::new(),
        )
        .await;
        let report = r.unwrap();
        assert_eq!(report.total_lessons, 6);
        let labels: Vec<_> = report
            .issues
            .iter()
            .map(|i| i.lesson.clone().unwrap())
            .collect();
        let expected: Vec<_> = (1..=6).map(|i| format!("1. Mod / {i}. L{i}")).collect();
        assert_eq!(labels, expected);
        assert_eq!(sink.reports.lock().unwrap().len(), 1);
        let p = sink.progress.lock().unwrap();
        assert!(p
            .iter()
            .all(|p| p.group == "Horizonte" && p.course == "Mod" && p.total == 6));
        assert!(p.iter().any(|p| p.percent >= 100.0 && p.completed == 6));
    }

    #[tokio::test]
    async fn leftover_part_files_are_removed_before_downloading() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = lesson_dir(tmp.path(), 1, "Lesson");
        std::fs::create_dir_all(dir.join("Materials")).unwrap();
        std::fs::write(dir.join("Materials/x.pdf.part"), "x").unwrap();
        std::fs::write(dir.join("keep.pdf"), "x").unwrap();
        let source = FakeSource {
            nav: Some(Ok(nav(&[("h1", "Lesson")]))),
            ..FakeSource::default()
        };
        source.lesson_data("h1", vec![Ok(simple_lesson("<p>x</p>"))]);
        run(&source, &FakeVideo::default(), tmp.path()).await;
        assert!(!dir.join("Materials/x.pdf.part").exists());
        assert!(dir.join("keep.pdf").exists());
    }
}
