//! Course download orchestration. Lessons are independent units processed by
//! a bounded-concurrency loop (`max_parallel_lessons`, 1 = sequential); every
//! failure becomes an [`Issue`] in the report, nothing is swallowed.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::{self, StreamExt};
use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Issue,
    IssueKind, Severity,
};
use lumen_download::files::UniqueNames;
use lumen_download::layout::path_units;
use lumen_download::report::finalize_folder;
use lumen_download::{VideoError, VideoJob, VideoRunner};

use crate::client::{GoldenlifeApi, BROWSER_UA};
use crate::error::GoldenlifeError;
use crate::parse::{CourseInfo, Embed, EmbedKind, LessonPage};
use crate::planner::{course_dir, plan_course, PlannedLesson};

/// Platform id used in reports and by the app shell.
pub const PLATFORM_ID: &str = "goldenlife";

/// Item-level retries with a growing pause.
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
    pub api: &'a dyn GoldenlifeApi,
    pub videos: &'a dyn VideoRunner,
    pub sink: &'a dyn CourseProgressSink,
    pub cancel: &'a CancelFlag,
    pub options: DownloadOptions,
    pub retry: Retry,
}

/// Values shared by every lesson of one run.
struct RunContext {
    course: CourseInfo,
    total: u32,
    finished: AtomicU32,
    /// Set when the academy rejected the session mid-run: nothing else is attempted.
    expired: AtomicBool,
}

struct LessonOutcome {
    order: usize,
    skipped: bool,
    issues: Vec<Issue>,
}

const VIDEO_EXTS: [&str; 3] = ["mp4", "mkv", "webm"];

/// Transport hiccups and rate limits may pass on a retry; the rest will not.
fn transient(e: &GoldenlifeError) -> bool {
    match e {
        GoldenlifeError::Network(_) => true,
        GoldenlifeError::HttpStatus(status) => *status == 429 || *status >= 500,
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

/// `name` made unique inside the lesson, keeping its extension last.
fn unique_file_name(names: &mut UniqueNames, name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 5 => {
            format!("{}.{ext}", names.claim(stem))
        }
        _ => names.claim(name),
    }
}

impl CourseDownloader<'_> {
    /// Downloads every lesson of `course` under `library_root`. Fatal problems
    /// (unreadable outline, expired session before the first lesson) return
    /// `Err`; everything else lands in the report.
    pub async fn run(
        &self,
        course: &CourseInfo,
        library_root: &Path,
    ) -> Result<DownloadReport, GoldenlifeError> {
        let outline = self
            .attempt(&AtomicBool::new(false), || self.api.outline(&course.id))
            .await?;
        let dir = course_dir(library_root, &course.title);
        std::fs::create_dir_all(&dir).map_err(|e| GoldenlifeError::Io(e.to_string()))?;
        remove_part_files(&dir);

        let plan = plan_course(&dir, &outline);
        let mut report = DownloadReport::new(PLATFORM_ID, &course.title);
        if plan.is_empty() {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                if outline.sections.is_empty() {
                    "course has no modules: the platform returned no content for it"
                } else {
                    "the course has modules but no lessons"
                },
            ));
        }
        let total = u32::try_from(plan.len()).unwrap_or(u32::MAX);
        report.total_lessons = total;
        let ctx = RunContext {
            course: course.clone(),
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
            group: ctx.course.title.clone(),
            course: lesson.section_title.clone(),
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
    /// only for transient errors; a rejected session stops the whole run.
    async fn attempt<T, F, Fut>(
        &self,
        expired: &AtomicBool,
        mut op: F,
    ) -> Result<T, GoldenlifeError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, GoldenlifeError>>,
    {
        let mut n = 0;
        loop {
            n += 1;
            match op().await {
                Ok(v) => return Ok(v),
                Err(e @ GoldenlifeError::Unauthorized) => {
                    expired.store(true, Ordering::SeqCst);
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
        let page = match self
            .attempt(&ctx.expired, || self.api.lesson(&lesson.lesson_id))
            .await
        {
            Ok(page) => page,
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

        let videos: Vec<&Embed> = page.main_video.iter().chain(&page.embeds).collect();
        let has_content = !page.content_html.trim().is_empty();
        if page.video_missing {
            issues.push(Issue::warning(
                IssueKind::VideoNotResolved,
                label,
                None,
                "the platform shows no video for this lesson (\"URL no identificada\")",
            ));
        } else if videos.is_empty() && !has_content && page.attachments.is_empty() {
            issues.push(Issue::warning(
                IssueKind::EmptyLesson,
                label,
                None,
                "no description, video or materials",
            ));
        }

        self.download_attachments(ctx, lesson, &page, &mut issues)
            .await;

        for (i, embed) in videos.iter().enumerate() {
            if self.checkpoint(ctx).await {
                return issues;
            }
            let n = i + 1;
            if numbered_video_exists(&lesson.dir, n) {
                continue;
            }
            self.run_video(
                ctx,
                lesson,
                n,
                self.video_job(lesson, n, embed),
                &mut issues,
            )
            .await;
        }

        let description = format!("<meta charset=\"utf-8\">\n{}", page.content_html);
        write_once(
            &lesson.dir.join("Description.html"),
            has_content.then_some(description.as_str()),
            label,
            &mut issues,
        );
        issues
    }

    fn video_job(&self, lesson: &PlannedLesson, n: usize, embed: &Embed) -> VideoJob {
        let template = |name: String| lesson.dir.join(name);
        match embed.kind {
            EmbedKind::YouTube => VideoJob {
                url: embed.url.clone(),
                referer: None,
                user_agent: None,
                output_template: template(format!("{n}. Lesson [%(id)s].%(ext)s")),
            },
            EmbedKind::Panda | EmbedKind::Other => VideoJob {
                url: embed.url.clone(),
                referer: Some(self.api.referer()),
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

    async fn download_attachments(
        &self,
        ctx: &RunContext,
        lesson: &PlannedLesson,
        page: &LessonPage,
        issues: &mut Vec<Issue>,
    ) {
        let label = lesson.label();
        let materials = lesson.dir.join("Materials");
        let budget = name_budget(path_units(&materials), 5).min(150);
        let mut names = UniqueNames::default();
        for attachment in &page.attachments {
            if self.checkpoint(ctx).await {
                return;
            }
            let name = unique_file_name(&mut names, &sanitize_file_name(&attachment.name, budget));
            let save = materials.join(&name);
            if save.exists() {
                continue;
            }
            let result = self
                .attempt(&ctx.expired, || {
                    self.api.download_attachment(&attachment.path, &save)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Attachment, LessonRef, Outline, Section};
    use async_trait::async_trait;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::Mutex;

    const PANDA: &str = "https://b-vz-lib.tv.pandavideo.com.br/vid-1/playlist.m3u8";
    const YOUTUBE: &str = "https://www.youtube.com/watch?v=abc123";

    fn course() -> CourseInfo {
        CourseInfo {
            id: "574182".into(),
            title: "Meta Ads Basico".into(),
            shelf: Some("ADS".into()),
            image_url: None,
        }
    }

    fn outline(lessons: &[(&str, &str)]) -> Outline {
        Outline {
            product_id: Some("574182".into()),
            sections: vec![Section {
                id: "s1".into(),
                title: "Fundamentos".into(),
                lessons: lessons
                    .iter()
                    .map(|(id, title)| LessonRef {
                        id: (*id).into(),
                        title: (*title).into(),
                    })
                    .collect(),
            }],
        }
    }

    fn video_page(url: &str) -> LessonPage {
        LessonPage {
            title: "x".into(),
            main_video: Some(Embed {
                kind: EmbedKind::Panda,
                url: url.into(),
            }),
            content_html: "<p>Hola</p>".into(),
            ..LessonPage::default()
        }
    }

    #[derive(Default)]
    struct FakeApi {
        outline: Mutex<Option<Result<Outline, GoldenlifeError>>>,
        lessons: Mutex<HashMap<String, Vec<Result<LessonPage, GoldenlifeError>>>>,
        failing_attachments: HashSet<String>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeApi {
        fn with(outline: Outline) -> Self {
            let api = Self::default();
            *api.outline.lock().unwrap() = Some(Ok(outline));
            api
        }
        fn page(self, id: &str, pages: Vec<Result<LessonPage, GoldenlifeError>>) -> Self {
            self.lessons.lock().unwrap().insert(id.into(), pages);
            self
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl GoldenlifeApi for FakeApi {
        async fn validate(&self) -> Result<(), GoldenlifeError> {
            Ok(())
        }
        async fn courses(&self) -> Result<Vec<CourseInfo>, GoldenlifeError> {
            Ok(vec![course()])
        }
        async fn outline(&self, id: &str) -> Result<Outline, GoldenlifeError> {
            self.calls.lock().unwrap().push(format!("outline:{id}"));
            self.outline
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(Err(GoldenlifeError::InvalidResponse("no outline")))
        }
        async fn lesson(&self, id: &str) -> Result<LessonPage, GoldenlifeError> {
            self.calls.lock().unwrap().push(format!("lesson:{id}"));
            let mut all = self.lessons.lock().unwrap();
            let queue = all
                .get_mut(id)
                .ok_or(GoldenlifeError::InvalidResponse("no lesson"))?;
            if queue.len() > 1 {
                queue.remove(0)
            } else {
                queue[0].clone()
            }
        }
        async fn download_attachment(
            &self,
            path: &str,
            dest: &Path,
        ) -> Result<(), GoldenlifeError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("attachment:{path}"));
            if self.failing_attachments.contains(path) {
                return Err(GoldenlifeError::Network("boom".into()));
            }
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(dest, "file").map_err(|e| GoldenlifeError::Io(e.to_string()))
        }
        fn referer(&self) -> String {
            "https://academy.example.test/".into()
        }
    }

    #[derive(Default)]
    struct FakeVideos {
        jobs: Mutex<Vec<VideoJob>>,
        failing: Vec<String>,
        cancel_on_first: Option<CancelFlag>,
    }

    #[async_trait]
    impl VideoRunner for FakeVideos {
        async fn download(
            &self,
            job: &VideoJob,
            _: &CancelFlag,
            on_percent: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            self.jobs.lock().unwrap().push(job.clone());
            if let Some(flag) = &self.cancel_on_first {
                flag.cancel();
            }
            on_percent(50.0);
            if self.failing.iter().any(|f| job.url.contains(f)) {
                return Err(VideoError::Failed("HTTP Error 403: Forbidden".into()));
            }
            let name = job
                .output_template
                .to_string_lossy()
                .replace("%(ext)s", "mp4")
                .replace("%(id)s", "abc123");
            std::fs::write(PathBuf::from(name), "video").unwrap();
            Ok(())
        }
    }

    #[derive(Default)]
    struct Sink {
        progress: Mutex<Vec<DownloadProgress>>,
        reports: Mutex<Vec<DownloadReport>>,
    }

    impl CourseProgressSink for Sink {
        fn on_progress(&self, progress: &DownloadProgress) {
            self.progress.lock().unwrap().push(progress.clone());
        }
        fn on_report(&self, report: &DownloadReport) {
            self.reports.lock().unwrap().push(report.clone());
        }
    }

    struct Run {
        report: Result<DownloadReport, GoldenlifeError>,
        api: FakeApi,
        videos: FakeVideos,
        sink: Sink,
        root: tempfile::TempDir,
    }

    async fn run_with(
        api: FakeApi,
        videos: FakeVideos,
        parallel: usize,
        cancel: CancelFlag,
    ) -> Run {
        let root = tempfile::tempdir().unwrap();
        let sink = Sink::default();
        let report = CourseDownloader {
            api: &api,
            videos: &videos,
            sink: &sink,
            cancel: &cancel,
            options: DownloadOptions {
                max_parallel_lessons: parallel,
            },
            retry: Retry {
                attempts: 3,
                unit: Duration::ZERO,
            },
        }
        .run(&course(), root.path())
        .await;
        Run {
            report,
            api,
            videos,
            sink,
            root,
        }
    }

    async fn run(api: FakeApi, videos: FakeVideos) -> Run {
        run_with(api, videos, 1, CancelFlag::new()).await
    }

    impl Run {
        fn lesson_dir(&self, section: &str, lesson: &str) -> PathBuf {
            self.root
                .path()
                .join("GoldenLife/Meta Ads Basico")
                .join(section)
                .join(lesson)
        }
        fn report(&self) -> &DownloadReport {
            self.report.as_ref().unwrap()
        }
        fn course_dir(&self) -> PathBuf {
            self.root.path().join("GoldenLife/Meta Ads Basico")
        }
    }

    #[tokio::test]
    async fn a_lesson_is_saved_with_its_video_description_and_materials() {
        let mut page = video_page(PANDA);
        page.attachments = vec![Attachment {
            name: "Guia: 1/2.pdf".into(),
            path: "/area/arquivo/anexo?a=1&signature=s".into(),
        }];
        let r = run(
            FakeApi::with(outline(&[("10", "Bienvenida")])).page("10", vec![Ok(page)]),
            FakeVideos::default(),
        )
        .await;

        let report = r.report();
        assert_eq!((report.total_lessons, report.completed_lessons), (1, 1));
        assert!(report.complete && report.issues.is_empty(), "{report:?}");
        let dir = r.lesson_dir("1. Fundamentos", "1. Bienvenida");
        assert!(dir.join("1. Lesson.mp4").is_file());
        let description = std::fs::read_to_string(dir.join("Description.html")).unwrap();
        assert!(description.contains("<p>Hola</p>") && description.contains("charset"));
        assert_eq!(
            std::fs::read_dir(dir.join("Materials")).unwrap().count(),
            1,
            "the attachment name is made safe for Windows"
        );
        assert!(r.course_dir().join(".complete").exists());
        assert_eq!(r.sink.reports.lock().unwrap().len(), 1);
        let progress = r.sink.progress.lock().unwrap();
        assert_eq!(progress.first().unwrap().group, "Meta Ads Basico");
        assert_eq!(progress.last().unwrap().percent, 100.0);

        let jobs = r.videos.jobs.lock().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].url, PANDA);
        assert_eq!(
            jobs[0].referer.as_deref(),
            Some("https://academy.example.test/")
        );
        assert_eq!(jobs[0].user_agent.as_deref(), Some(BROWSER_UA));
        assert_eq!(jobs[0].output_template, dir.join("1. Lesson.%(ext)s"));
    }

    #[tokio::test]
    async fn embedded_youtube_videos_follow_the_main_video() {
        let mut page = video_page(PANDA);
        page.embeds = vec![Embed {
            kind: EmbedKind::YouTube,
            url: YOUTUBE.into(),
        }];
        let r = run(
            FakeApi::with(outline(&[("10", "Bienvenida")])).page("10", vec![Ok(page)]),
            FakeVideos::default(),
        )
        .await;
        assert!(r.report().complete, "{:?}", r.report());
        let jobs = r.videos.jobs.lock().unwrap();
        let dir = r.lesson_dir("1. Fundamentos", "1. Bienvenida");
        assert_eq!(jobs[1].url, YOUTUBE);
        assert_eq!(jobs[1].referer, None);
        assert_eq!(
            jobs[1].output_template,
            dir.join("2. Lesson [%(id)s].%(ext)s")
        );
    }

    #[tokio::test]
    async fn a_failed_video_is_an_error_and_blocks_the_complete_marker() {
        let r = run(
            FakeApi::with(outline(&[("10", "A"), ("11", "B")]))
                .page("10", vec![Ok(video_page(PANDA))])
                .page(
                    "11",
                    vec![Ok(video_page(
                        "https://b-vz-lib.tv.pandavideo.com.br/bad/playlist.m3u8",
                    ))],
                ),
            FakeVideos {
                failing: vec!["/bad/".into()],
                ..FakeVideos::default()
            },
        )
        .await;
        let report = r.report();
        assert_eq!((report.total_lessons, report.completed_lessons), (2, 1));
        assert!(!report.complete);
        let issue = &report.issues[0];
        assert_eq!(issue.kind, IssueKind::Video);
        assert_eq!(issue.severity, Severity::Error);
        assert_eq!(issue.lesson.as_deref(), Some("1. Fundamentos / 2. B"));
        assert!(issue.message.contains("403"));
        assert!(!r.course_dir().join(".complete").exists());
    }

    #[tokio::test]
    async fn a_lesson_the_platform_shows_without_a_video_is_a_warning() {
        let page = LessonPage {
            title: "x".into(),
            video_missing: true,
            content_html: "<p>texto</p>".into(),
            ..LessonPage::default()
        };
        let r = run(
            FakeApi::with(outline(&[("10", "Lectura")])).page("10", vec![Ok(page)]),
            FakeVideos::default(),
        )
        .await;
        let report = r.report();
        assert_eq!(report.issues.len(), 1, "{report:?}");
        assert_eq!(report.issues[0].kind, IssueKind::VideoNotResolved);
        assert_eq!(report.issues[0].severity, Severity::Warning);
        assert!(
            report.complete,
            "a platform-side gap does not block completion"
        );
        assert!(r.videos.jobs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_lesson_with_nothing_in_it_is_reported_as_empty() {
        let r = run(
            FakeApi::with(outline(&[("10", "Vacia")])).page("10", vec![Ok(LessonPage::default())]),
            FakeVideos::default(),
        )
        .await;
        let issue = &r.report().issues[0];
        assert_eq!(
            (issue.kind, issue.severity),
            (IssueKind::EmptyLesson, Severity::Warning)
        );
    }

    #[tokio::test]
    async fn a_failed_attachment_is_an_error_but_the_video_is_still_saved() {
        let mut page = video_page(PANDA);
        page.attachments = vec![Attachment {
            name: "a.pdf".into(),
            path: "/area/arquivo/anexo?a=bad".into(),
        }];
        let mut api = FakeApi::with(outline(&[("10", "Con adjunto")])).page("10", vec![Ok(page)]);
        api.failing_attachments
            .insert("/area/arquivo/anexo?a=bad".into());
        let r = run(api, FakeVideos::default()).await;
        let report = r.report();
        assert!(!report.complete);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, IssueKind::Resource);
        assert!(report.issues[0].message.contains("a.pdf"));
        assert!(r
            .lesson_dir("1. Fundamentos", "1. Con adjunto")
            .join("1. Lesson.mp4")
            .is_file());
    }

    #[tokio::test]
    async fn a_course_without_content_says_it_has_no_modules() {
        let r = run(
            FakeApi::with(Outline {
                product_id: Some("574182".into()),
                sections: Vec::new(),
            }),
            FakeVideos::default(),
        )
        .await;
        let report = r.report();
        assert!(!report.complete);
        assert_eq!(report.issues[0].kind, IssueKind::Course);
        assert!(report.issues[0].message.contains("course has no modules"));
    }

    #[tokio::test]
    async fn transient_failures_are_retried_and_the_rest_are_reported() {
        let r = run(
            FakeApi::with(outline(&[("10", "Reintento"), ("11", "Roto")]))
                .page(
                    "10",
                    vec![
                        Err(GoldenlifeError::Network("reset".into())),
                        Ok(video_page(PANDA)),
                    ],
                )
                .page(
                    "11",
                    vec![Err(GoldenlifeError::InvalidResponse("bad page"))],
                ),
            FakeVideos::default(),
        )
        .await;
        let report = r.report();
        assert_eq!(report.completed_lessons, 1);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, IssueKind::ModulePage);
        assert_eq!(
            r.api.calls().iter().filter(|c| *c == "lesson:10").count(),
            2,
            "one retry"
        );
        assert_eq!(
            r.api.calls().iter().filter(|c| *c == "lesson:11").count(),
            1,
            "permanent errors are not retried"
        );
    }

    #[tokio::test]
    async fn an_expired_session_stops_the_run_and_says_so() {
        let r = run(
            FakeApi::with(outline(&[("10", "A"), ("11", "B")]))
                .page("10", vec![Err(GoldenlifeError::Unauthorized)])
                .page("11", vec![Ok(video_page(PANDA))]),
            FakeVideos::default(),
        )
        .await;
        let report = r.report();
        assert!(!report.complete);
        assert!(report
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::Course && i.message.contains("session expired")));
        assert!(!r.api.calls().contains(&"lesson:11".to_string()));
    }

    #[tokio::test]
    async fn finished_files_are_not_downloaded_again() {
        let api = FakeApi::with(outline(&[("10", "Bienvenida")])).page("10", {
            let mut page = video_page(PANDA);
            page.attachments = vec![Attachment {
                name: "a.pdf".into(),
                path: "/area/arquivo/anexo?a=1".into(),
            }];
            vec![Ok(page)]
        });
        let root = tempfile::tempdir().unwrap();
        let dir = root
            .path()
            .join("GoldenLife/Meta Ads Basico/1. Fundamentos/1. Bienvenida");
        std::fs::create_dir_all(dir.join("Materials")).unwrap();
        std::fs::write(dir.join("1. Lesson.mp4"), "old").unwrap();
        std::fs::write(dir.join("Materials/a.pdf"), "old").unwrap();
        std::fs::write(dir.join("Lesson.mp4.part"), "partial").unwrap();
        let videos = FakeVideos::default();
        let sink = Sink::default();
        let cancel = CancelFlag::new();
        let report = CourseDownloader {
            api: &api,
            videos: &videos,
            sink: &sink,
            cancel: &cancel,
            options: DownloadOptions::default(),
            retry: Retry::default(),
        }
        .run(&course(), root.path())
        .await
        .unwrap();
        assert!(report.complete, "{report:?}");
        assert!(videos.jobs.lock().unwrap().is_empty());
        assert!(!api.calls().iter().any(|c| c.starts_with("attachment:")));
        assert_eq!(
            std::fs::read_to_string(dir.join("1. Lesson.mp4")).unwrap(),
            "old"
        );
        assert!(
            !dir.join("Lesson.mp4.part").exists(),
            "interrupted leftovers are removed"
        );
    }

    #[tokio::test]
    async fn a_paused_run_holds_before_the_next_lesson_until_resumed() {
        let cancel = CancelFlag::new();
        cancel.pause();
        let run = run_with(
            FakeApi::with(outline(&[("10", "A")])).page("10", vec![Ok(video_page(PANDA))]),
            FakeVideos::default(),
            1,
            cancel.clone(),
        );
        tokio::pin!(run);
        let held = tokio::time::timeout(std::time::Duration::from_millis(300), &mut run).await;
        assert!(held.is_err(), "the run holds while paused");
        cancel.resume();
        let r = run.await;
        assert_eq!(r.report().completed_lessons, 1);
        assert_eq!(r.videos.jobs.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cancelling_stops_the_run_without_the_complete_marker() {
        let cancel = CancelFlag::new();
        let r = run_with(
            FakeApi::with(outline(&[("10", "A"), ("11", "B")]))
                .page("10", vec![Ok(video_page(PANDA))])
                .page("11", vec![Ok(video_page(PANDA))]),
            FakeVideos {
                cancel_on_first: Some(cancel.clone()),
                ..FakeVideos::default()
            },
            1,
            cancel,
        )
        .await;
        let report = r.report();
        assert!(report.cancelled && !report.complete);
        assert_eq!(r.videos.jobs.lock().unwrap().len(), 1);
        assert!(!r.course_dir().join(".complete").exists());
    }

    #[tokio::test]
    async fn lessons_can_run_in_parallel() {
        let ids = ["10", "11", "12", "13"];
        let mut api = FakeApi::with(outline(
            &ids.iter().map(|id| (*id, "L")).collect::<Vec<_>>(),
        ));
        for id in ids {
            api = api.page(id, vec![Ok(video_page(PANDA))]);
        }
        let r = run_with(api, FakeVideos::default(), 3, CancelFlag::new()).await;
        let report = r.report();
        assert_eq!((report.total_lessons, report.completed_lessons), (4, 4));
        assert!(report.complete);
    }
}
