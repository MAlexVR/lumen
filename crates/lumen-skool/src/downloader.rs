//! Group download orchestration. Lessons are independent units processed by a
//! bounded-concurrency loop (`max_parallel_lessons`, 1 = sequential); every
//! failure becomes an [`Issue`] in the report, nothing is swallowed.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::{self, StreamExt};
use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Issue,
    IssueKind, PlatformGroup, Severity,
};

use crate::client::{FetchedFile, SkoolClient, WEB_BASE};
use crate::error::SkoolError;
use crate::links::{clean_video_url, is_downloadable_link, needs_skool_referer, url_file_stem};
use crate::parse::{parse_resources, CourseListing, CourseTree, ModuleDetail, SkipReason};
use crate::planner::{group_folder_name, plan_course, PlannedLesson};
use crate::richtext::{description_links, description_to_html, is_empty_description};
use lumen_download::files::{finished_file_exists, video_exists, write_link_file, UniqueNames};
use lumen_download::layout::path_units;
use lumen_download::report::finalize_folder;
use lumen_download::{VideoError, VideoJob, VideoRunner};

/// Platform id used in reports and by the app shell.
pub const PLATFORM_ID: &str = "skool";

/// Everything the downloader needs from Skool (implemented by [`SkoolClient`]).
#[async_trait]
pub trait SkoolSource: Send + Sync {
    async fn classroom(&self, slug: &str) -> Result<CourseListing, SkoolError>;
    async fn course(&self, slug: &str, course: &str) -> Result<CourseTree, SkoolError>;
    async fn module(
        &self,
        slug: &str,
        course: &str,
        module_id: &str,
    ) -> Result<ModuleDetail, SkoolError>;
    async fn fetch_file(&self, url: &str, dest_base: &Path) -> Result<FetchedFile, SkoolError>;
}

#[async_trait]
impl SkoolSource for SkoolClient {
    async fn classroom(&self, slug: &str) -> Result<CourseListing, SkoolError> {
        SkoolClient::classroom(self, slug).await
    }
    async fn course(&self, slug: &str, course: &str) -> Result<CourseTree, SkoolError> {
        SkoolClient::course(self, slug, course).await
    }
    async fn module(
        &self,
        slug: &str,
        course: &str,
        module_id: &str,
    ) -> Result<ModuleDetail, SkoolError> {
        SkoolClient::module(self, slug, course, module_id).await
    }
    async fn fetch_file(&self, url: &str, dest_base: &Path) -> Result<FetchedFile, SkoolError> {
        SkoolClient::fetch_file(self, url, dest_base).await
    }
}

pub struct GroupDownloader<'a> {
    pub source: &'a dyn SkoolSource,
    pub videos: &'a dyn VideoRunner,
    pub sink: &'a dyn CourseProgressSink,
    pub cancel: &'a CancelFlag,
    pub options: DownloadOptions,
}

/// Values shared by every lesson of one run.
struct RunContext {
    slug: String,
    group_name: String,
    total: u32,
    finished: AtomicU32,
}

/// How a lesson's video is obtained.
enum VideoSource {
    None,
    External(String),
    Native(String),
    Unresolved,
}

struct LessonOutcome {
    order: usize,
    skipped: bool,
    issues: Vec<Issue>,
}

fn non_empty_resources(raw: &str) -> bool {
    !matches!(raw.trim(), "" | "[]" | "false")
}

impl GroupDownloader<'_> {
    /// Downloads every accessible course of `group` under `library_root`.
    /// Fatal problems (no courses, unreadable group) return `Err`.
    pub async fn run(
        &self,
        group: &PlatformGroup,
        library_root: &Path,
    ) -> Result<DownloadReport, SkoolError> {
        let group_dir = library_root
            .join("Skool")
            .join(group_folder_name(&group.name));
        std::fs::create_dir_all(&group_dir).map_err(|e| {
            SkoolError::Io(format!("cannot create \"{}\": {e}", group_dir.display()))
        })?;

        let listing = self.source.classroom(&group.slug).await?;
        if listing.courses.is_empty() {
            return Err(SkoolError::Shape(
                "no accessible courses found in group".into(),
            ));
        }
        let mut report = DownloadReport::new(PLATFORM_ID, &group.name);
        for skipped in &listing.skipped {
            if skipped.reason == SkipReason::NoAccess {
                report.push(Issue::warning(
                    IssueKind::Course,
                    None,
                    None,
                    format!("course \"{}\" skipped: no access", skipped.title),
                ));
            }
        }

        let mut plan: Vec<PlannedLesson> = Vec::new();
        for (i, course) in listing.courses.iter().enumerate() {
            if self.cancel.is_cancelled() {
                break;
            }
            match self.source.course(&group.slug, &course.short_name).await {
                Ok(tree) => {
                    let lessons = plan_course(&group_dir, i + 1, course, &tree, plan.len());
                    plan.extend(lessons);
                }
                Err(e) => report.push(Issue::error(
                    IssueKind::Course,
                    None,
                    None,
                    format!("cannot read course \"{}\": {e}", course.title),
                )),
            }
        }

        let total = u32::try_from(plan.len()).unwrap_or(u32::MAX);
        report.total_lessons = total;
        let ctx = RunContext {
            slug: group.slug.clone(),
            group_name: group.name.clone(),
            total,
            finished: AtomicU32::new(0),
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
        report.cancelled = self.cancel.is_cancelled();
        finalize_folder(&group_dir, &mut report);
        self.sink.on_report(&report);
        Ok(report)
    }

    fn emit(&self, ctx: &RunContext, lesson: &PlannedLesson, percent: f32) {
        self.sink.on_progress(&DownloadProgress {
            group: ctx.group_name.clone(),
            course: lesson.course_title.clone(),
            lesson: lesson.title.clone(),
            percent,
            completed: ctx.finished.load(Ordering::SeqCst),
            total: ctx.total,
        });
    }

    async fn process_lesson(&self, ctx: &RunContext, lesson: &PlannedLesson) -> LessonOutcome {
        lumen_download::wait_if_paused(self.cancel).await;
        if self.cancel.is_cancelled() {
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

    async fn lesson_issues(&self, ctx: &RunContext, lesson: &PlannedLesson) -> Vec<Issue> {
        let label = lesson.label();
        let label = Some(label.as_str());
        let mut issues = Vec::new();
        if let Err(e) = std::fs::create_dir_all(&lesson.dir) {
            issues.push(Issue::error(
                IssueKind::Course,
                label,
                None,
                format!("cannot create the lesson folder: {e}"),
            ));
            return issues;
        }

        let entry = &lesson.module;
        let course = &lesson.course_short_name;
        let detail = match self.source.module(&ctx.slug, course, &entry.id).await {
            Ok(d) => Some(d),
            Err(e) => {
                issues.push(Issue::error(
                    IssueKind::ModulePage,
                    label,
                    None,
                    e.to_string(),
                ));
                None
            }
        };
        let detail_ref = detail.as_ref();

        let desc = match detail_ref {
            Some(d) if !is_empty_description(&d.desc) => d.desc.as_str(),
            _ => entry.desc.as_str(),
        };
        let resources_raw = match detail_ref {
            Some(d) if non_empty_resources(&d.resources) => d.resources.as_str(),
            _ => entry.resources.as_str(),
        };
        let link = match detail_ref {
            Some(d) if !d.video_link.is_empty() => d.video_link.as_str(),
            _ => entry.video_link.as_str(),
        };
        let source = if !link.is_empty() {
            VideoSource::External(link.to_string())
        } else if let Some(native) = detail_ref.and_then(|d| d.native.as_ref()) {
            VideoSource::Native(native.stream_url())
        } else if entry.is_native() || detail_ref.is_some_and(|d| !d.video_id.is_empty()) {
            VideoSource::Unresolved
        } else {
            VideoSource::None
        };

        let has_desc = !is_empty_description(desc);
        if !has_desc && !non_empty_resources(resources_raw) && matches!(source, VideoSource::None) {
            issues.push(Issue::warning(
                IssueKind::EmptyLesson,
                label,
                None,
                "no description, video or resources",
            ));
        }

        if has_desc {
            let path = lesson.dir.join("description.html");
            if !path.exists() {
                if let Err(e) = std::fs::write(&path, description_to_html(desc)) {
                    issues.push(Issue::error(
                        IssueKind::Course,
                        label,
                        None,
                        format!("cannot write description.html: {e}"),
                    ));
                }
            }
        }

        match source {
            VideoSource::None => {}
            VideoSource::Unresolved => issues.push(Issue::error(
                IssueKind::VideoNotResolved,
                label,
                None,
                "native video has no playback token in the module page",
            )),
            VideoSource::External(url) | VideoSource::Native(url) => {
                if !video_exists(&lesson.dir) {
                    let native = detail_ref.is_some_and(|d| d.native.is_some()) && link.is_empty();
                    self.download_video(ctx, lesson, url, native, &mut issues)
                        .await;
                }
            }
        }

        lumen_download::wait_if_paused(self.cancel).await;
        if !self.cancel.is_cancelled() {
            self.download_resources(lesson, resources_raw, desc, &mut issues)
                .await;
        }
        issues
    }

    async fn download_video(
        &self,
        ctx: &RunContext,
        lesson: &PlannedLesson,
        mut url: String,
        native: bool,
        issues: &mut Vec<Issue>,
    ) {
        let label = lesson.label();
        let template = lesson.dir.join(format!("{}.%(ext)s", lesson.file_stem));
        let mut refreshed = false;
        loop {
            let job = VideoJob {
                url: clean_video_url(&url),
                referer: needs_skool_referer(&url).then(|| format!("{WEB_BASE}/")),
                user_agent: None,
                output_template: template.clone(),
            };
            let on_percent = |p: f32| self.emit(ctx, lesson, p);
            match self.videos.download(&job, self.cancel, &on_percent).await {
                Ok(()) => {
                    if !video_exists(&lesson.dir) {
                        issues.push(Issue::error(
                            IssueKind::Video,
                            Some(&label),
                            Some(&job.url),
                            "yt-dlp finished but no video file was found",
                        ));
                    }
                    return;
                }
                Err(VideoError::Cancelled) => return,
                Err(e) => {
                    if native && !refreshed {
                        refreshed = true;
                        let fresh = self
                            .source
                            .module(&ctx.slug, &lesson.course_short_name, &lesson.module.id)
                            .await;
                        if let Some(stream) = fresh.ok().and_then(|d| d.native) {
                            url = stream.stream_url();
                            continue;
                        }
                    }
                    issues.push(Issue::error(
                        IssueKind::Video,
                        Some(&label),
                        Some(&job.url),
                        e.to_string(),
                    ));
                    return;
                }
            }
        }
    }

    async fn download_resources(
        &self,
        lesson: &PlannedLesson,
        resources_raw: &str,
        desc: &str,
        issues: &mut Vec<Issue>,
    ) {
        let label = lesson.label();
        let label = Some(label.as_str());
        let budget = name_budget(path_units(&lesson.dir), 12).min(120);
        let mut seen: HashSet<String> = HashSet::new();
        let mut names = UniqueNames::default();

        match parse_resources(resources_raw) {
            Err(e) => issues.push(Issue::error(
                IssueKind::Resource,
                label,
                None,
                format!("unreadable resources list: {e}"),
            )),
            Ok(list) => {
                for res in list {
                    lumen_download::wait_if_paused(self.cancel).await;
                    if self.cancel.is_cancelled() {
                        return;
                    }
                    seen.insert(res.link.clone());
                    let title = match res.title.trim() {
                        "" => match url_file_stem(&res.link) {
                            s if s.is_empty() => "Resource".to_string(),
                            s => s,
                        },
                        t => t.to_string(),
                    };
                    let name = names.claim(&sanitize_file_name(&title, budget));
                    let base = lesson.dir.join(&name);
                    if is_downloadable_link(&res.link) {
                        if finished_file_exists(&base) {
                            continue;
                        }
                        match self.source.fetch_file(&res.link, &base).await {
                            Ok(FetchedFile::Saved(_)) => continue,
                            Ok(FetchedFile::NotAFile) => {}
                            Err(e) => issues.push(Issue::error(
                                IssueKind::Resource,
                                label,
                                Some(&res.link),
                                e.to_string(),
                            )),
                        }
                    }
                    if let Err(e) = write_link_file(&lesson.dir, &name, &title, &res.link) {
                        issues.push(Issue::error(
                            IssueKind::Resource,
                            label,
                            Some(&res.link),
                            format!("cannot write the link file: {e}"),
                        ));
                    }
                }
            }
        }

        let mut attachments = UniqueNames::default();
        for link in description_links(desc) {
            lumen_download::wait_if_paused(self.cancel).await;
            if self.cancel.is_cancelled() {
                return;
            }
            if seen.contains(&link.link) || !is_downloadable_link(&link.link) {
                continue;
            }
            seen.insert(link.link.clone());
            let title = match link.title.trim() {
                "" => match url_file_stem(&link.link) {
                    s if s.is_empty() => "Attachment".to_string(),
                    s => s,
                },
                t => t.to_string(),
            };
            let name = attachments.claim(&sanitize_file_name(&title, budget));
            let base = lesson.dir.join("Attachments").join(name);
            if finished_file_exists(&base) {
                continue;
            }
            if let Err(e) = self.source.fetch_file(&link.link, &base).await {
                issues.push(Issue::error(
                    IssueKind::Resource,
                    label,
                    Some(&link.link),
                    e.to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{CourseInfo, ModuleEntry, NativeVideo, SkipReason, SkippedCourse};
    use lumen_core::{DownloadProgress, IssueKind, Severity};
    use lumen_download::{VideoError, VideoJob};
    use std::collections::{HashMap, VecDeque};
    use std::path::PathBuf;
    use std::sync::Mutex;

    type FileRule = Result<Option<&'static str>, SkoolError>;

    #[derive(Default)]
    struct FakeSource {
        listing: CourseListing,
        trees: HashMap<String, Result<CourseTree, SkoolError>>,
        modules: Mutex<HashMap<String, VecDeque<Result<ModuleDetail, SkoolError>>>>,
        files: HashMap<String, FileRule>,
        module_calls: Mutex<Vec<String>>,
        file_calls: Mutex<Vec<String>>,
    }

    impl FakeSource {
        fn set_modules(&self, id: &str, seq: Vec<Result<ModuleDetail, SkoolError>>) {
            self.modules.lock().unwrap().insert(id.into(), seq.into());
        }
    }

    #[async_trait]
    impl SkoolSource for FakeSource {
        async fn classroom(&self, _: &str) -> Result<CourseListing, SkoolError> {
            Ok(self.listing.clone())
        }
        async fn course(&self, _: &str, course: &str) -> Result<CourseTree, SkoolError> {
            self.trees
                .get(course)
                .cloned()
                .unwrap_or_else(|| Err(SkoolError::Shape("no tree".into())))
        }
        async fn module(&self, _: &str, _: &str, id: &str) -> Result<ModuleDetail, SkoolError> {
            self.module_calls.lock().unwrap().push(id.into());
            let mut all = self.modules.lock().unwrap();
            match all.get_mut(id) {
                Some(q) if q.len() > 1 => q.pop_front().unwrap_or(Err(SkoolError::SessionBlocked)),
                Some(q) => q
                    .front()
                    .cloned()
                    .unwrap_or(Err(SkoolError::SessionBlocked)),
                None => Ok(detail("", "", "")),
            }
        }
        async fn fetch_file(&self, url: &str, base: &Path) -> Result<FetchedFile, SkoolError> {
            self.file_calls.lock().unwrap().push(url.into());
            match self.files.get(url) {
                Some(Ok(Some(ext))) => {
                    if let Some(dir) = base.parent() {
                        std::fs::create_dir_all(dir).unwrap();
                    }
                    let mut p = base.as_os_str().to_owned();
                    p.push(ext);
                    std::fs::write(&p, "data").unwrap();
                    Ok(FetchedFile::Saved(PathBuf::from(p)))
                }
                Some(Ok(None)) | None => Ok(FetchedFile::NotAFile),
                Some(Err(e)) => Err(e.clone()),
            }
        }
    }

    #[derive(Default)]
    struct FakeVideo {
        results: Mutex<VecDeque<Result<(), VideoError>>>,
        jobs: Mutex<Vec<VideoJob>>,
        skip_file: bool,
        cancel_on_call: Option<CancelFlag>,
    }

    #[async_trait]
    impl VideoRunner for FakeVideo {
        async fn download(
            &self,
            job: &VideoJob,
            cancel: &CancelFlag,
            on_percent: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            self.jobs.lock().unwrap().push(job.clone());
            if let Some(c) = &self.cancel_on_call {
                c.cancel();
            }
            if cancel.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            on_percent(50.0);
            let next = self.results.lock().unwrap().pop_front().unwrap_or(Ok(()));
            if next.is_ok() && !self.skip_file {
                let name = job
                    .output_template
                    .to_string_lossy()
                    .replace("%(ext)s", "mp4");
                std::fs::write(name, "video").unwrap();
            }
            next
        }
    }

    #[derive(Default)]
    struct Recorder {
        progress: Mutex<Vec<DownloadProgress>>,
        reports: Mutex<Vec<DownloadReport>>,
    }

    impl CourseProgressSink for Recorder {
        fn on_progress(&self, p: &DownloadProgress) {
            self.progress.lock().unwrap().push(p.clone());
        }
        fn on_report(&self, r: &DownloadReport) {
            self.reports.lock().unwrap().push(r.clone());
        }
    }

    fn entry(id: &str, section: &str, title: &str) -> ModuleEntry {
        ModuleEntry {
            id: id.into(),
            section: section.into(),
            title: title.into(),
            video_link: String::new(),
            video_id: String::new(),
            desc: String::new(),
            resources: String::new(),
            has_access: true,
        }
    }

    fn detail(desc: &str, resources: &str, link: &str) -> ModuleDetail {
        ModuleDetail {
            desc: desc.into(),
            resources: resources.into(),
            video_link: link.into(),
            video_id: String::new(),
            native: None,
        }
    }

    fn native(token: &str) -> Option<NativeVideo> {
        Some(NativeVideo {
            playback_id: "PB".into(),
            token: token.into(),
            expire: None,
        })
    }

    const DESC: &str = r#"[v2][{"type":"paragraph","content":[{"type":"text","text":"Read "},{"type":"text","text":"sheet","marks":[{"type":"link","attrs":{"href":"https://files.test/sheet.xlsx?sig=1"}}]}]}]"#;
    const RESOURCES: &str = r#"[{"title":"Guide","link":"https://files.test/guide.pdf"},{"title":"Drive","link":"https://drive.google.com/file/d/1/view"}]"#;

    /// Course "intro" with m1 (native, rich), m2 (youtube) and m3 (vimeo).
    fn source() -> FakeSource {
        let mut m1 = entry("m1", "Start", "Welcome");
        m1.video_id = "v1".into();
        let mut m2 = entry("m2", "Start", "External");
        m2.video_link = "https://www.youtube.com/watch?v=abc".into();
        let mut m3 = entry("m3", "Advanced", "Vimeo");
        m3.video_link = "https://vimeo.com/123?share=copy".into();
        let mut src = FakeSource::default();
        src.listing.courses.push(CourseInfo {
            id: "c1".into(),
            short_name: "intro".into(),
            title: "Intro".into(),
            num_modules: 3,
        });
        src.trees.insert(
            "intro".into(),
            Ok(CourseTree {
                title: "Intro".into(),
                modules: vec![m1, m2, m3],
            }),
        );
        let mut d1 = detail(DESC, RESOURCES, "");
        d1.native = native("TOK1");
        src.set_modules("m1", vec![Ok(d1)]);
        src.files
            .insert("https://files.test/guide.pdf".into(), Ok(Some(".pdf")));
        src.files.insert(
            "https://files.test/sheet.xlsx?sig=1".into(),
            Ok(Some(".xlsx")),
        );
        src
    }

    fn group() -> PlatformGroup {
        PlatformGroup {
            slug: "growth".into(),
            name: "Growth".into(),
            cover_url: None,
            num_courses: 1,
        }
    }

    struct Run {
        report: Result<DownloadReport, SkoolError>,
        sink: Recorder,
        root: tempfile::TempDir,
    }

    impl Run {
        fn dir(&self) -> PathBuf {
            self.root.path().join("Skool/Growth")
        }
        fn lesson(&self, rel: &str) -> PathBuf {
            self.dir().join("01 - Intro").join(rel)
        }
        fn report(&self) -> &DownloadReport {
            self.report.as_ref().unwrap()
        }
    }

    async fn run_in(
        root: tempfile::TempDir,
        src: &FakeSource,
        vid: &FakeVideo,
        cancel: &CancelFlag,
        parallel: usize,
    ) -> Run {
        let sink = Recorder::default();
        let report = GroupDownloader {
            source: src,
            videos: vid,
            sink: &sink,
            cancel,
            options: DownloadOptions {
                max_parallel_lessons: parallel,
            },
        }
        .run(&group(), root.path())
        .await;
        Run { report, sink, root }
    }

    async fn run(src: &FakeSource, vid: &FakeVideo) -> Run {
        run_in(
            tempfile::tempdir().unwrap(),
            src,
            vid,
            &CancelFlag::new(),
            1,
        )
        .await
    }

    fn issues_of(r: &Run, kind: IssueKind) -> Vec<lumen_core::Issue> {
        r.report()
            .issues
            .iter()
            .filter(|i| i.kind == kind)
            .cloned()
            .collect()
    }

    #[tokio::test]
    async fn happy_path_writes_the_expected_layout_and_marks_complete() {
        let (src, vid) = (source(), FakeVideo::default());
        let r = run(&src, &vid).await;
        let rep = r.report();
        assert!(rep.complete, "{:?}", rep.issues);
        assert_eq!((rep.total_lessons, rep.completed_lessons), (3, 3));

        let l1 = r.lesson("01 - Start/01 - Welcome");
        assert!(l1.join("Welcome.mp4").is_file());
        let html = std::fs::read_to_string(l1.join("description.html")).unwrap();
        assert!(html.contains("<p>Read "));
        assert_eq!(std::fs::read(l1.join("Guide.pdf")).unwrap(), b"data");
        assert_eq!(
            std::fs::read_to_string(l1.join("Drive.txt")).unwrap(),
            "Drive\nhttps://drive.google.com/file/d/1/view\n"
        );
        assert!(l1.join("Attachments/sheet.xlsx").is_file());
        assert!(r.lesson("01 - Start/02 - External/External.mp4").is_file());
        assert!(r.lesson("02 - Advanced/01 - Vimeo/Vimeo.mp4").is_file());

        assert!(r.dir().join(".complete").is_file());
        let on_disk: DownloadReport = serde_json::from_str(
            &std::fs::read_to_string(r.dir().join("download-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(&on_disk, rep);
        assert_eq!(r.sink.reports.lock().unwrap().len(), 1);
        let last = r.sink.progress.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.completed, last.total), (3, 3));
        assert_eq!(last.group, "Growth");
        assert_eq!(last.course, "Intro");
    }

    #[tokio::test]
    async fn video_sources_get_the_right_url_and_referer() {
        let (src, vid) = (source(), FakeVideo::default());
        let _ = run(&src, &vid).await;
        let jobs = vid.jobs.lock().unwrap().clone();
        let by = |needle: &str| {
            jobs.iter()
                .find(|j| j.url.contains(needle))
                .unwrap()
                .clone()
        };
        let mux = by("stream.mux.com");
        assert_eq!(mux.url, "https://stream.mux.com/PB.m3u8?token=TOK1");
        assert_eq!(mux.referer.as_deref(), Some("https://www.skool.com/"));
        let yt = by("youtube.com");
        assert_eq!(yt.url, "https://www.youtube.com/watch?v=abc");
        assert_eq!(yt.referer, None);
        let vimeo = by("vimeo.com");
        assert_eq!(vimeo.url, "https://vimeo.com/123");
        assert_eq!(vimeo.referer.as_deref(), Some("https://www.skool.com/"));
    }

    #[tokio::test]
    async fn an_uncreatable_group_folder_error_names_the_folder() {
        let root = tempfile::tempdir().unwrap();
        // A file where the "Skool" folder must go makes create_dir_all fail.
        std::fs::write(root.path().join("Skool"), "not a folder").unwrap();
        let (src, vid) = (source(), FakeVideo::default());
        let r = GroupDownloader {
            source: &src,
            videos: &vid,
            sink: &Recorder::default(),
            cancel: &CancelFlag::new(),
            options: DownloadOptions {
                max_parallel_lessons: 1,
            },
        }
        .run(&group(), root.path())
        .await;
        let message = r.unwrap_err().to_string();
        assert!(message.contains("Skool"), "{message}");
        assert!(message.contains("Growth"), "{message}");
    }

    #[tokio::test]
    async fn a_failed_video_is_reported_redacted_and_blocks_complete() {
        let (src, vid) = (source(), FakeVideo::default());
        // m1 native: first attempt and the token-refresh retry both fail.
        vid.results.lock().unwrap().extend([
            Err(VideoError::Failed("HTTP Error 403".into())),
            Err(VideoError::Failed("HTTP Error 403".into())),
        ]);
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Skool/Growth")).unwrap();
        std::fs::write(root.path().join("Skool/Growth/.complete"), "done").unwrap();
        let r = run_in(root, &src, &vid, &CancelFlag::new(), 1).await;
        let rep = r.report();
        assert!(!rep.complete);
        let v = issues_of(&r, IssueKind::Video);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].severity, Severity::Error);
        assert_eq!(v[0].url.as_deref(), Some("https://stream.mux.com/PB.m3u8"));
        assert_eq!(v[0].host.as_deref(), Some("stream.mux.com"));
        assert_eq!(v[0].message, "HTTP Error 403");
        assert_eq!(v[0].lesson.as_deref(), Some("Start / 01 - Welcome"));
        assert!(!r.dir().join(".complete").exists(), "stale marker removed");
        let json = std::fs::read_to_string(r.dir().join("download-report.json")).unwrap();
        assert!(json.contains("HTTP Error 403") && !json.contains("TOK1"));
        assert_eq!(rep.completed_lessons, 2);
    }

    #[tokio::test]
    async fn native_video_is_retried_once_with_a_fresh_token() {
        let (src, vid) = (source(), FakeVideo::default());
        let mut fresh = detail(DESC, RESOURCES, "");
        fresh.native = native("TOK2");
        let mut first = detail(DESC, RESOURCES, "");
        first.native = native("TOK1");
        src.set_modules("m1", vec![Ok(first), Ok(fresh)]);
        vid.results
            .lock()
            .unwrap()
            .push_back(Err(VideoError::Failed("expired".into())));
        let r = run(&src, &vid).await;
        assert!(r.report().complete, "{:?}", r.report().issues);
        let jobs = vid.jobs.lock().unwrap().clone();
        assert!(jobs[0].url.ends_with("token=TOK1"));
        assert!(jobs[1].url.ends_with("token=TOK2"));
        assert_eq!(
            src.module_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|m| *m == "m1")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn module_page_failure_is_reported_and_listing_data_is_used() {
        let (src, vid) = (source(), FakeVideo::default());
        src.set_modules("m1", vec![Err(SkoolError::Unauthorized(403))]);
        src.set_modules(
            "m2",
            vec![Err(SkoolError::Http {
                status: 500,
                url: "https://www.skool.com/g".into(),
            })],
        );
        let r = run(&src, &vid).await;
        let pages = issues_of(&r, IssueKind::ModulePage);
        assert_eq!(pages.len(), 2);
        assert!(pages[0].message.contains("403"));
        // m1 is native and cannot be resolved without its page.
        assert_eq!(issues_of(&r, IssueKind::VideoNotResolved).len(), 1);
        // m2 still downloads its external link from the listing.
        assert!(r.lesson("01 - Start/02 - External/External.mp4").is_file());
        assert!(!r.report().complete);
    }

    #[tokio::test]
    async fn native_lesson_without_stream_is_video_not_resolved() {
        let (src, vid) = (source(), FakeVideo::default());
        src.set_modules("m1", vec![Ok(detail(DESC, RESOURCES, ""))]);
        let r = run(&src, &vid).await;
        let v = issues_of(&r, IssueKind::VideoNotResolved);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].severity, Severity::Error);
        assert!(!r.report().complete);
        assert!(!r.lesson("01 - Start/01 - Welcome/Welcome.mp4").exists());
    }

    #[tokio::test]
    async fn resource_failures_are_reported_but_the_link_is_kept() {
        let (mut src, vid) = (source(), FakeVideo::default());
        src.files.insert(
            "https://files.test/guide.pdf".into(),
            Err(SkoolError::Http {
                status: 404,
                url: "https://files.test/guide.pdf".into(),
            }),
        );
        src.files
            .insert("https://files.test/sheet.xlsx?sig=1".into(), Ok(None));
        let r = run(&src, &vid).await;
        let res = issues_of(&r, IssueKind::Resource);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].url.as_deref(), Some("https://files.test/guide.pdf"));
        assert_eq!(res[0].host.as_deref(), Some("files.test"));
        let l1 = r.lesson("01 - Start/01 - Welcome");
        assert!(l1.join("Guide.txt").is_file(), "link kept after failure");
        assert!(
            !l1.join("Attachments").exists(),
            "html page is not an attachment"
        );
        assert!(!r.report().complete);
    }

    #[tokio::test]
    async fn an_empty_lesson_is_a_warning_and_does_not_block_complete() {
        let (mut src, vid) = (source(), FakeVideo::default());
        src.trees.insert(
            "intro".into(),
            Ok(CourseTree {
                title: "Intro".into(),
                modules: vec![entry("e1", "Start", "Nothing here")],
            }),
        );
        let r = run(&src, &vid).await;
        let w = issues_of(&r, IssueKind::EmptyLesson);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].severity, Severity::Warning);
        assert!(r.report().complete);
    }

    #[tokio::test]
    async fn rerun_skips_what_is_already_on_disk() {
        let (src, vid) = (source(), FakeVideo::default());
        let first = run(&src, &vid).await;
        let desc = first.lesson("01 - Start/01 - Welcome/description.html");
        std::fs::write(&desc, "edited").unwrap();
        let root = first.root;
        let vid2 = FakeVideo::default();
        let src2 = source();
        let second = run_in(root, &src2, &vid2, &CancelFlag::new(), 1).await;
        assert!(second.report().complete);
        assert!(
            vid2.jobs.lock().unwrap().is_empty(),
            "videos exist: no download"
        );
        assert!(
            src2.file_calls.lock().unwrap().is_empty(),
            "files exist: no fetch"
        );
        assert_eq!(std::fs::read_to_string(desc).unwrap(), "edited");
    }

    #[tokio::test]
    async fn video_runner_success_without_a_file_is_an_error() {
        let (src, mut vid) = (source(), FakeVideo::default());
        vid.skip_file = true;
        let r = run(&src, &vid).await;
        assert_eq!(issues_of(&r, IssueKind::Video).len(), 3);
        assert!(!r.report().complete);
    }

    #[tokio::test]
    async fn a_paused_run_holds_before_the_next_lesson_until_resumed() {
        let src = source();
        let vid = FakeVideo::default();
        let cancel = CancelFlag::new();
        cancel.pause();
        let run = run_in(tempfile::tempdir().unwrap(), &src, &vid, &cancel, 1);
        tokio::pin!(run);
        let held = tokio::time::timeout(std::time::Duration::from_millis(300), &mut run).await;
        assert!(held.is_err(), "the run holds while paused");
        assert!(vid.jobs.lock().unwrap().is_empty());
        cancel.resume();
        let r = run.await;
        assert!(!r.report().cancelled);
        assert!(!vid.jobs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancel_stops_the_run_and_never_marks_complete() {
        let src = source();
        let cancel = CancelFlag::new();
        let vid = FakeVideo {
            cancel_on_call: Some(cancel.clone()),
            ..FakeVideo::default()
        };
        let r = run_in(tempfile::tempdir().unwrap(), &src, &vid, &cancel, 1).await;
        let rep = r.report();
        assert!(rep.cancelled && !rep.complete);
        assert!(!r.dir().join(".complete").exists());
        assert_eq!(vid.jobs.lock().unwrap().len(), 1);
        assert!(
            rep.issues.iter().all(|i| i.kind != IssueKind::Video),
            "cancel is not a failure"
        );
        assert!(r.dir().join("download-report.json").is_file());
    }

    #[tokio::test]
    async fn parallel_mode_processes_every_lesson_and_orders_issues() {
        let (src, vid) = (source(), FakeVideo::default());
        for id in ["m1", "m2", "m3"] {
            src.set_modules(id, vec![Err(SkoolError::SessionBlocked)]);
        }
        let r = run_in(
            tempfile::tempdir().unwrap(),
            &src,
            &vid,
            &CancelFlag::new(),
            3,
        )
        .await;
        let rep = r.report();
        assert_eq!(rep.total_lessons, 3);
        let lessons: Vec<_> = issues_of(&r, IssueKind::ModulePage)
            .iter()
            .filter_map(|i| i.lesson.clone())
            .collect();
        assert_eq!(
            lessons,
            [
                "Start / 01 - Welcome",
                "Start / 02 - External",
                "Advanced / 01 - Vimeo"
            ]
        );
        let last = r
            .sink
            .progress
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.completed)
            .max();
        assert_eq!(last, Some(3));
    }

    #[tokio::test]
    async fn skipped_courses_and_course_failures_are_reported() {
        let (mut src, vid) = (source(), FakeVideo::default());
        src.listing.skipped.push(SkippedCourse {
            title: "Secret".into(),
            reason: SkipReason::Private,
        });
        src.listing.skipped.push(SkippedCourse {
            title: "Locked".into(),
            reason: SkipReason::NoAccess,
        });
        src.listing.courses.push(CourseInfo {
            id: "c2".into(),
            short_name: "broken".into(),
            title: "Broken".into(),
            num_modules: 1,
        });
        let r = run(&src, &vid).await;
        let courses = issues_of(&r, IssueKind::Course);
        assert_eq!(courses.len(), 2, "{courses:?}");
        assert!(courses
            .iter()
            .any(|i| i.severity == Severity::Warning && i.message.contains("Locked")));
        assert!(courses
            .iter()
            .any(|i| i.severity == Severity::Error && i.message.contains("Broken")));
        assert!(!courses.iter().any(|i| i.message.contains("Secret")));
        assert!(
            r.lesson("01 - Start/01 - Welcome/Welcome.mp4").is_file(),
            "good course still downloads"
        );
        assert!(!r.report().complete);
    }

    #[tokio::test]
    async fn a_group_without_accessible_courses_is_an_error() {
        let (mut src, vid) = (source(), FakeVideo::default());
        src.listing.courses.clear();
        let r = run(&src, &vid).await;
        assert!(matches!(r.report, Err(SkoolError::Shape(_))));
    }
}
