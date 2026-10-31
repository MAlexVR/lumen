//! Workout download orchestration. Every workout is an independent unit; each
//! exercise video is skipped when it is already on disk, so running again only
//! fetches what is new (typically workouts scheduled since the last run). Every
//! failure becomes an [`Issue`] in the report, nothing is swallowed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::{self, StreamExt};
use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Issue,
    IssueKind, Severity,
};
use lumen_download::report::finalize_folder;
use lumen_download::{VideoError, VideoJob, VideoRunner};
use lumen_youtube::reason::classify;
use lumen_youtube::url::{parse_target, Target};
use serde_json::Value;

use crate::error::HarbizError;
use crate::parse::{Parsed, Workout};
use crate::planner::{
    course_dir, find_video, plan_workouts, video_exists, PlannedExercise, PlannedWorkout,
    COURSE_TITLE, DESCRIPTION_FILE, LEGACY_WORKOUT_FILE, WORKOUT_FILE,
};

/// Platform id used in reports and by the app shell.
pub const PLATFORM_ID: &str = "harbiz";

pub struct WorkoutDownloader<'a> {
    pub videos: &'a dyn VideoRunner,
    pub sink: &'a dyn CourseProgressSink,
    pub cancel: &'a CancelFlag,
    pub options: DownloadOptions,
    /// Pause after each video request so a long agenda is not a burst.
    pub pace: std::time::Duration,
}

/// YouTube asks to sign in after a burst of requests; asking again only
/// prolongs it, so the run stops after this many bot checks in a row.
const BOT_CHECK_LIMIT: u32 = 3;

/// What this run already learned about a YouTube video, so a video that many
/// workouts share is asked for once.
#[derive(Clone)]
enum Seen {
    Saved(PathBuf),
    Unavailable(&'static str),
}

struct RunContext {
    total: u32,
    finished: AtomicU32,
    seen: Mutex<HashMap<String, Seen>>,
    bot_checks: AtomicU32,
    blocked: AtomicBool,
    not_attempted: AtomicU32,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The report keeps the video id in the link (queries are stripped from every
/// other URL, and a YouTube watch link is useless without `v=`).
fn video_issue(
    severity: Severity,
    kind: IssueKind,
    label: &str,
    url: &str,
    id: Option<&str>,
    message: String,
) -> Issue {
    let mut issue = Issue::new(kind, severity, Some(label), Some(url), message);
    if let Some(id) = id {
        issue.url = Some(format!("https://youtu.be/{id}"));
    }
    issue
}

struct WorkoutOutcome {
    order: usize,
    skipped: bool,
    issues: Vec<Issue>,
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

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn logged_sets(result: &Value) -> Vec<String> {
    let Some(sets) = result.get("sets").and_then(Value::as_array) else {
        return Vec::new();
    };
    sets.iter()
        .filter_map(Value::as_object)
        .map(|set| {
            set.iter()
                .filter_map(|(key, value)| match value {
                    Value::String(text) if !text.is_empty() => Some(format!("{key}: {text}")),
                    Value::Number(number) => Some(format!("{key}: {number}")),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect()
}

/// The description the library shows for a class: the workout, how it went and
/// every exercise with its prescription, coach notes and logged sets.
fn workout_description(workout: &Workout, exercises: &[PlannedExercise]) -> String {
    let title = if workout.title.trim().is_empty() {
        "Workout"
    } else {
        workout.title.trim()
    };
    let mut html = format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>{0}</title>\n<h1>{0}</h1>\n<p>{1}</p>\n",
        escape_html(title),
        escape_html(&workout.date)
    );
    if !workout.description.is_empty() {
        html += &format!(
            "<p>{}</p>\n",
            escape_html(&workout.description).replace('\n', "<br>")
        );
    }
    let how_it_went = overall_facts(&workout.result_general);
    if !how_it_went.is_empty() {
        html += &format!("<p>{}</p>\n", escape_html(&how_it_went.join(" · ")));
    }
    html += "<ol>\n";
    for planned in exercises {
        let e = &planned.exercise;
        let name = if e.name.trim().is_empty() {
            "Exercise"
        } else {
            e.name.trim()
        };
        html += &format!("<li><strong>{}</strong>", escape_html(name));
        if let Some(section) = e.section.as_deref() {
            html += &format!(" ({})", escape_html(section));
        }
        let facts: Vec<String> = [("Sets", &e.sets), ("Target", &e.target), ("Rest", &e.rest)]
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(label, value)| format!("{label}: {value}"))
            .collect();
        if !facts.is_empty() {
            html += &format!("<br>{}", escape_html(&facts.join(" · ")));
        }
        if !e.notes.is_empty() {
            html += &format!("<br>{}", escape_html(&e.notes).replace('\n', "<br>"));
        }
        for (i, set) in logged_sets(&e.result).iter().enumerate() {
            html += &format!("<br>Set {}: {}", i + 1, escape_html(set));
        }
        html += "</li>\n";
    }
    html += "</ol>\n";
    html
}

/// `rpe`, `compliance`, ... from the overall result of the workout.
fn overall_facts(result: &Value) -> Vec<String> {
    let Some(object) = result.as_object() else {
        return Vec::new();
    };
    ["rpe", "compliance", "rate", "comments"]
        .into_iter()
        .filter_map(|key| match object.get(key)? {
            Value::String(text) if !text.is_empty() => Some(format!("{key}: {text}")),
            Value::Number(number) => Some(format!("{key}: {number}")),
            _ => None,
        })
        .collect()
}

/// Moves a workout folder written by an earlier version (straight under the
/// course) into its month and removes the files that version generated.
fn migrate_legacy(workout: &PlannedWorkout) -> Result<(), String> {
    if workout.legacy_dir.is_dir() && !workout.dir.exists() {
        std::fs::create_dir_all(&workout.month_dir).map_err(|e| e.to_string())?;
        std::fs::rename(&workout.legacy_dir, &workout.dir).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(workout.dir.join(LEGACY_WORKOUT_FILE));
    for planned in &workout.exercises {
        let _ = std::fs::remove_file(workout.dir.join(format!("{}.html", planned.stem)));
    }
    Ok(())
}

impl WorkoutDownloader<'_> {
    /// Downloads every workout of `parsed` under `library_root`. Fatal problems
    /// (an unwritable library) return `Err`; everything else lands in the report.
    pub async fn run(
        &self,
        parsed: &Parsed,
        library_root: &Path,
    ) -> Result<DownloadReport, HarbizError> {
        let dir = course_dir(library_root);
        std::fs::create_dir_all(&dir).map_err(|e| HarbizError::Io(e.to_string()))?;
        remove_part_files(&dir);

        let plan = plan_workouts(&dir, &parsed.workouts);
        let mut report = DownloadReport::new(PLATFORM_ID, COURSE_TITLE);
        if plan.is_empty() {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "the agenda has no workouts: the platform returned nothing to download",
            ));
        }
        if parsed.photo_checkins > 0 {
            report.push(Issue::warning(
                IssueKind::Course,
                None,
                None,
                format!(
                    "{} body-photo check-ins are on the agenda; photos are not downloaded",
                    parsed.photo_checkins
                ),
            ));
        }
        if parsed.unsupported_events > 0 {
            report.push(Issue::warning(
                IssueKind::Course,
                None,
                None,
                format!(
                    "{} agenda events of an unknown type were skipped",
                    parsed.unsupported_events
                ),
            ));
        }
        let total = u32::try_from(plan.len()).unwrap_or(u32::MAX);
        report.total_lessons = total;
        let ctx = RunContext {
            total,
            finished: AtomicU32::new(0),
            seen: Mutex::new(HashMap::new()),
            bot_checks: AtomicU32::new(0),
            blocked: AtomicBool::new(false),
            not_attempted: AtomicU32::new(0),
        };
        let parallel = self.options.max_parallel_lessons.max(1);
        let units: Vec<BoxFuture<'_, WorkoutOutcome>> = plan
            .iter()
            .map(|workout| self.process_workout(&ctx, workout).boxed())
            .collect();
        let mut outcomes: Vec<WorkoutOutcome> = stream::iter(units)
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
        if ctx.blocked.load(Ordering::SeqCst) {
            let left = ctx.not_attempted.load(Ordering::SeqCst);
            let skipped = match left {
                0 => String::new(),
                1 => ", so 1 video was not attempted".to_string(),
                n => format!(", so {n} videos were not attempted"),
            };
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                format!(
                    "YouTube asked to sign in (bot check) {BOT_CHECK_LIMIT} times in a row{skipped}: try again later and press Update, what is already downloaded is kept"
                ),
            ));
        }
        report.cancelled = self.cancel.is_cancelled();
        finalize_folder(&dir, &mut report);
        self.sink.on_report(&report);
        Ok(report)
    }

    fn emit(&self, ctx: &RunContext, workout: &PlannedWorkout, percent: f32) {
        self.sink.on_progress(&DownloadProgress {
            group: COURSE_TITLE.to_string(),
            course: COURSE_TITLE.to_string(),
            lesson: workout.label.clone(),
            percent,
            completed: ctx.finished.load(Ordering::SeqCst),
            total: ctx.total,
        });
    }

    async fn process_workout(&self, ctx: &RunContext, workout: &PlannedWorkout) -> WorkoutOutcome {
        lumen_download::wait_if_paused(self.cancel).await;
        if self.cancel.is_cancelled() {
            return WorkoutOutcome {
                order: workout.order,
                skipped: true,
                issues: Vec::new(),
            };
        }
        self.emit(ctx, workout, 0.0);
        let issues = self.workout_issues(ctx, workout).await;
        ctx.finished.fetch_add(1, Ordering::SeqCst);
        self.emit(ctx, workout, 100.0);
        WorkoutOutcome {
            order: workout.order,
            skipped: false,
            issues,
        }
    }

    async fn workout_issues(&self, ctx: &RunContext, workout: &PlannedWorkout) -> Vec<Issue> {
        let label = workout.label.as_str();
        let mut issues = Vec::new();
        if let Err(e) = migrate_legacy(workout) {
            issues.push(Issue::error(
                IssueKind::Course,
                Some(label),
                None,
                format!("cannot move the workout into its month folder: {e}"),
            ));
            return issues;
        }
        // A workout already in the library is completed in place; a new one
        // downloads into a hidden folder and is published when it is done.
        let work: &Path = if workout.dir.is_dir() {
            &workout.dir
        } else {
            &workout.staging_dir
        };
        if let Err(e) = std::fs::create_dir_all(work) {
            issues.push(Issue::error(
                IssueKind::Course,
                Some(label),
                None,
                format!("cannot create the workout folder: {e}"),
            ));
            return issues;
        }
        // Metadata is rewritten every run: a scheduled workout gains its results later.
        let metadata = serde_json::to_string_pretty(&workout.workout)
            .map_err(|e| e.to_string())
            .and_then(|json| {
                std::fs::write(work.join(WORKOUT_FILE), json).map_err(|e| e.to_string())
            });
        if let Err(e) = metadata {
            issues.push(Issue::error(
                IssueKind::Course,
                Some(label),
                None,
                format!("cannot write {WORKOUT_FILE}: {e}"),
            ));
        }
        let page = workout_description(&workout.workout, &workout.exercises);
        if let Err(e) = std::fs::write(work.join(DESCRIPTION_FILE), page) {
            issues.push(Issue::error(
                IssueKind::Course,
                Some(label),
                None,
                format!("cannot write {DESCRIPTION_FILE}: {e}"),
            ));
        }
        if workout.exercises.is_empty() {
            issues.push(Issue::warning(
                IssueKind::EmptyLesson,
                Some(label),
                None,
                "the workout has no exercises",
            ));
        }
        for planned in &workout.exercises {
            let Some(url) = planned.exercise.url.as_deref() else {
                issues.push(Issue::warning(
                    IssueKind::VideoNotResolved,
                    Some(label),
                    None,
                    format!("{} has no video link", planned.stem),
                ));
                continue;
            };
            if video_exists(work, planned.number) {
                continue;
            }
            lumen_download::wait_if_paused(self.cancel).await;
            if self.cancel.is_cancelled() {
                continue;
            }
            self.run_video(ctx, workout, work, planned, url, &mut issues)
                .await;
        }
        if work != workout.dir.as_path() {
            self.publish(ctx, workout, work, &mut issues);
        }
        issues
    }

    /// Moves a finished hidden folder into place so the library lists the class.
    /// A class where every attempt failed, or a cancelled one, stays hidden and
    /// the next run resumes from it.
    fn publish(
        &self,
        ctx: &RunContext,
        workout: &PlannedWorkout,
        work: &Path,
        issues: &mut Vec<Issue>,
    ) {
        let has_video = workout
            .exercises
            .iter()
            .any(|e| find_video(work, e.number).is_some());
        let failed = issues.iter().any(|i| i.severity == Severity::Error);
        if self.cancel.is_cancelled() || (failed && !has_video) {
            return;
        }
        match std::fs::rename(work, &workout.dir) {
            Ok(()) => {
                for seen in lock(&ctx.seen).values_mut() {
                    if let Seen::Saved(path) = seen {
                        if let Ok(rest) = path.strip_prefix(work) {
                            *path = workout.dir.join(rest);
                        }
                    }
                }
            }
            Err(e) => issues.push(Issue::error(
                IssueKind::Course,
                Some(workout.label.as_str()),
                None,
                format!("cannot publish the workout: {e}"),
            )),
        }
    }

    async fn run_video(
        &self,
        ctx: &RunContext,
        workout: &PlannedWorkout,
        dir: &Path,
        planned: &PlannedExercise,
        url: &str,
        issues: &mut Vec<Issue>,
    ) {
        let label = workout.label.as_str();
        let id = match parse_target(url) {
            Ok(Target::Video(id)) => Some(id),
            _ => None,
        };
        let known = id.as_ref().and_then(|id| lock(&ctx.seen).get(id).cloned());
        match known {
            Some(Seen::Saved(source)) => {
                let target = dir.join(format!(
                    "{}.{}",
                    planned.stem,
                    source.extension().and_then(|e| e.to_str()).unwrap_or("mp4")
                ));
                let copied = std::fs::hard_link(&source, &target)
                    .or_else(|_| std::fs::copy(&source, &target).map(|_| ()));
                if let Err(e) = copied {
                    issues.push(video_issue(
                        Severity::Error,
                        IssueKind::Video,
                        label,
                        url,
                        id.as_deref(),
                        format!("{}: cannot reuse the downloaded video: {e}", planned.stem),
                    ));
                }
                return;
            }
            Some(Seen::Unavailable(reason)) => {
                issues.push(video_issue(
                    Severity::Warning,
                    IssueKind::VideoNotResolved,
                    label,
                    url,
                    id.as_deref(),
                    format!("{}: {reason}", planned.stem),
                ));
                return;
            }
            None => {}
        }
        if ctx.blocked.load(Ordering::SeqCst) {
            ctx.not_attempted.fetch_add(1, Ordering::SeqCst);
            return;
        }
        let job = VideoJob {
            url: url.to_string(),
            referer: None,
            user_agent: None,
            // `%` is yt-dlp's template escape character.
            output_template: dir.join(format!("{}.%(ext)s", planned.stem.replace('%', "%%"))),
        };
        let on_percent = |p: f32| self.emit(ctx, workout, p);
        match self.videos.download(&job, self.cancel, &on_percent).await {
            Ok(()) => match find_video(dir, planned.number) {
                Some(path) => {
                    ctx.bot_checks.store(0, Ordering::SeqCst);
                    if let Some(id) = id {
                        lock(&ctx.seen).insert(id, Seen::Saved(path));
                    }
                }
                None => issues.push(video_issue(
                    Severity::Error,
                    IssueKind::Video,
                    label,
                    url,
                    id.as_deref(),
                    "yt-dlp finished but no video file was found".to_string(),
                )),
            },
            Err(VideoError::Cancelled) => {}
            Err(e) => self.video_failed(ctx, label, planned, url, id, &e.to_string(), issues),
        }
        if !self.pace.is_zero() {
            tokio::time::sleep(self.pace).await;
        }
    }

    /// A video that no longer exists is a warning (nothing can fix it from
    /// here); a bot check is an error that stops the run after a few in a row;
    /// anything else is an error with yt-dlp's own words.
    #[allow(clippy::too_many_arguments)]
    fn video_failed(
        &self,
        ctx: &RunContext,
        label: &str,
        planned: &PlannedExercise,
        url: &str,
        id: Option<String>,
        message: &str,
        issues: &mut Vec<Issue>,
    ) {
        match classify(message) {
            Some(reason) if reason.permanent => {
                if let Some(id) = &id {
                    lock(&ctx.seen).insert(id.clone(), Seen::Unavailable(reason.text));
                }
                issues.push(video_issue(
                    Severity::Warning,
                    IssueKind::VideoNotResolved,
                    label,
                    url,
                    id.as_deref(),
                    format!("{}: {}", planned.stem, reason.text),
                ));
            }
            Some(reason) => {
                issues.push(video_issue(
                    Severity::Error,
                    IssueKind::Video,
                    label,
                    url,
                    id.as_deref(),
                    format!("{}: {}", planned.stem, reason.text),
                ));
                if ctx.bot_checks.fetch_add(1, Ordering::SeqCst) + 1 >= BOT_CHECK_LIMIT {
                    ctx.blocked.store(true, Ordering::SeqCst);
                }
            }
            None => issues.push(video_issue(
                Severity::Error,
                IssueKind::Video,
                label,
                url,
                id.as_deref(),
                message.to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Exercise, Workout};
    use crate::planner::{course_dir, WORKOUT_FILE};
    use async_trait::async_trait;
    use lumen_core::{DownloadProgress, IssueKind, Severity};
    use lumen_download::{VideoError, VideoJob};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct FakeVideos {
        jobs: Mutex<Vec<VideoJob>>,
        fail_urls: Vec<String>,
        /// Fails with exactly this yt-dlp message.
        errors: Vec<(String, String)>,
        write_nothing: bool,
        /// Records, at every call, whether this folder is already visible.
        watch_dir: Option<PathBuf>,
        visible: Mutex<Vec<bool>>,
        cancel_on_first: Option<CancelFlag>,
    }

    impl FakeVideos {
        fn urls(&self) -> Vec<String> {
            self.jobs
                .lock()
                .unwrap()
                .iter()
                .map(|j| j.url.clone())
                .collect()
        }
    }

    #[async_trait]
    impl VideoRunner for FakeVideos {
        async fn download(
            &self,
            job: &VideoJob,
            _cancel: &CancelFlag,
            on_percent: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            self.jobs.lock().unwrap().push(job.clone());
            if let Some(dir) = &self.watch_dir {
                self.visible.lock().unwrap().push(dir.exists());
            }
            if let Some(cancel) = &self.cancel_on_first {
                cancel.cancel();
            }
            if let Some((_, message)) = self.errors.iter().find(|(url, _)| *url == job.url) {
                return Err(VideoError::Failed(message.clone()));
            }
            if self.fail_urls.contains(&job.url) {
                return Err(VideoError::Failed("boom".to_string()));
            }
            on_percent(100.0);
            if !self.write_nothing {
                // yt-dlp turns `%%` into a literal percent sign and fills `%(ext)s`.
                let path = job.output_template.to_string_lossy().replace("%%", "%");
                std::fs::write(path.replace("%(ext)s", "mp4"), "video").unwrap();
            }
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

    fn exercise(name: &str, url: Option<&str>) -> Exercise {
        Exercise {
            name: name.to_string(),
            url: url.map(str::to_string),
            section: Some("Mobility".to_string()),
            sets: "2".to_string(),
            target: "10".to_string(),
            rest: "30".to_string(),
            notes: "Keep <it> slow & steady".to_string(),
            result: json!({"sets": [{"reps": "10", "weight": "2.5"}]}),
        }
    }

    fn workout(id: &str, date: &str, title: &str, exercises: Vec<Exercise>) -> Workout {
        Workout {
            id: id.to_string(),
            title: title.to_string(),
            date: date.to_string(),
            completed: true,
            description: "Warm up first".to_string(),
            exercises,
            result_general: Value::Null,
        }
    }

    fn parsed(workouts: Vec<Workout>) -> Parsed {
        Parsed {
            workouts,
            ..Parsed::default()
        }
    }

    struct Run {
        report: DownloadReport,
        videos: FakeVideos,
        sink: Sink,
    }

    async fn run_with(videos: FakeVideos, parsed: &Parsed, root: &Path, cancel: CancelFlag) -> Run {
        run_paced(videos, parsed, root, cancel, Duration::ZERO).await
    }

    async fn run_paced(
        videos: FakeVideos,
        parsed: &Parsed,
        root: &Path,
        cancel: CancelFlag,
        pace: Duration,
    ) -> Run {
        let sink = Sink::default();
        let report = WorkoutDownloader {
            videos: &videos,
            sink: &sink,
            cancel: &cancel,
            options: DownloadOptions::default(),
            pace,
        }
        .run(parsed, root)
        .await
        .unwrap();
        Run {
            report,
            videos,
            sink,
        }
    }

    async fn run(parsed: &Parsed, root: &Path) -> Run {
        run_with(FakeVideos::default(), parsed, root, CancelFlag::new()).await
    }

    fn two_workouts() -> Parsed {
        parsed(vec![
            workout(
                "a",
                "2026-06-02",
                "Legs",
                vec![
                    exercise("Squat", Some("https://youtu.be/sq?si=token")),
                    exercise("Lunge", Some("https://youtu.be/lu")),
                ],
            ),
            workout(
                "b",
                "2026-06-04",
                "Core",
                vec![exercise("Plank", Some("https://youtu.be/pl"))],
            ),
        ])
    }

    #[tokio::test]
    async fn downloads_videos_notes_and_metadata_and_marks_the_course_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run(&two_workouts(), tmp.path()).await;
        let course = course_dir(tmp.path());
        let legs = course.join("2026-06/2026-06-02 - Legs");
        assert!(legs.join("01. Squat.mp4").is_file());
        assert!(legs.join("02. Lunge.mp4").is_file());
        assert!(course
            .join("2026-06/2026-06-04 - Core/01. Plank.mp4")
            .is_file());
        let meta: Value =
            serde_json::from_str(&std::fs::read_to_string(legs.join(WORKOUT_FILE)).unwrap())
                .unwrap();
        assert_eq!(meta["id"], "a");
        assert_eq!(meta["exercises"][0]["name"], "Squat");
        assert!(out.report.complete);
        assert_eq!(out.report.platform, "harbiz");
        assert_eq!(out.report.total_lessons, 2);
        assert_eq!(out.report.completed_lessons, 2);
        assert!(out.report.issues.is_empty());
        assert!(course.join(".complete").exists());
        assert_eq!(out.sink.reports.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_class_description_is_escaped_html_with_exercises_sets_and_results() {
        let tmp = tempfile::tempdir().unwrap();
        run(&two_workouts(), tmp.path()).await;
        let legs = course_dir(tmp.path()).join("2026-06/2026-06-02 - Legs");
        let page = std::fs::read_to_string(legs.join("Description.html")).unwrap();
        assert!(page.contains("Legs") && page.contains("2026-06-02"));
        assert!(page.contains("Warm up first"));
        assert!(page.contains("Squat") && page.contains("Lunge"));
        assert!(page.contains("Keep &lt;it&gt; slow &amp; steady"));
        assert!(!page.contains("<it>"));
        assert!(page.contains("reps: 10"));
        assert!(page.contains("weight: 2.5"));
        assert!(
            !legs.join("01. Squat.html").exists(),
            "exercises have no page of their own"
        );
    }

    #[tokio::test]
    async fn the_metadata_file_is_hidden_from_the_library() {
        assert!(WORKOUT_FILE.starts_with('.'));
    }

    #[tokio::test]
    async fn the_old_flat_folders_are_moved_into_their_month_and_cleaned() {
        let tmp = tempfile::tempdir().unwrap();
        let course = course_dir(tmp.path());
        let old = course.join("2026-06-02 - Legs");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("01. Squat.mp4"), "old video").unwrap();
        std::fs::write(old.join("01. Squat.html"), "old note").unwrap();
        std::fs::write(old.join("workout.json"), "{}").unwrap();
        let out = run(&two_workouts(), tmp.path()).await;
        let moved = course.join("2026-06/2026-06-02 - Legs");
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(moved.join("01. Squat.mp4")).unwrap(),
            "old video"
        );
        assert!(!out
            .videos
            .urls()
            .contains(&"https://youtu.be/sq?si=token".to_string()));
        assert!(!moved.join("01. Squat.html").exists());
        assert!(!moved.join("workout.json").exists());
        assert!(moved.join(WORKOUT_FILE).is_file());
        assert!(out.report.complete);
    }

    #[tokio::test]
    async fn a_second_run_downloads_nothing_but_refreshes_the_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let first = two_workouts();
        run(&first, tmp.path()).await;
        let mut again = first.clone();
        again.workouts[0].description = "Updated".to_string();
        let out = run(&again, tmp.path()).await;
        assert!(out.videos.urls().is_empty());
        assert!(out.report.complete);
        let meta = std::fs::read_to_string(
            course_dir(tmp.path())
                .join("2026-06/2026-06-02 - Legs")
                .join(WORKOUT_FILE),
        )
        .unwrap();
        assert!(meta.contains("Updated"));
    }

    #[tokio::test]
    async fn a_newly_scheduled_workout_is_the_only_thing_downloaded() {
        let tmp = tempfile::tempdir().unwrap();
        let mut agenda = two_workouts();
        run(&agenda, tmp.path()).await;
        agenda.workouts.push(workout(
            "c",
            "2026-10-31",
            "Kicks",
            vec![exercise("Kick", Some("https://youtu.be/ki"))],
        ));
        let out = run(&agenda, tmp.path()).await;
        assert_eq!(out.videos.urls(), ["https://youtu.be/ki"]);
        assert_eq!(out.report.total_lessons, 3);
        assert!(course_dir(tmp.path())
            .join("2026-10/2026-10-31 - Kicks/01. Kick.mp4")
            .is_file());
    }

    #[tokio::test]
    async fn a_failed_video_is_a_redacted_error_and_the_rest_still_downloads() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            fail_urls: vec!["https://youtu.be/sq?si=token".to_string()],
            ..FakeVideos::default()
        };
        let out = run_with(videos, &two_workouts(), tmp.path(), CancelFlag::new()).await;
        let errors: Vec<_> = out
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, IssueKind::Video);
        assert_eq!(errors[0].url.as_deref(), Some("https://youtu.be/sq"));
        assert_eq!(errors[0].message, "boom");
        assert!(errors[0]
            .lesson
            .as_deref()
            .unwrap()
            .contains("2026-06-02 - Legs"));
        assert!(!out.report.complete);
        assert_eq!(out.report.completed_lessons, 1);
        let course = course_dir(tmp.path());
        assert!(course
            .join("2026-06/2026-06-04 - Core/01. Plank.mp4")
            .is_file());
        assert!(!course.join(".complete").exists());
    }

    const A: &str = "aaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbb";
    const C: &str = "ccccccccccc";
    const D: &str = "ddddddddddd";
    const E: &str = "eeeeeeeeeee";
    const NOT_AVAILABLE: &str = "[youtube] x: This video is not available";
    const BOT: &str = "[youtube] x: Sign in to confirm you\u{2019}re not a bot. Use --cookies-from-browser or --cookies for the authentication.";

    fn url_of(id: &str) -> String {
        format!("https://youtu.be/{id}")
    }

    fn class(id: &str, date: &str, videos: &[&str]) -> Workout {
        let exercises = videos
            .iter()
            .enumerate()
            .map(|(i, v)| exercise(&format!("Move {i}"), Some(&url_of(v))))
            .collect();
        workout(id, date, "t", exercises)
    }

    fn failing(ids: &[&str], message: &str) -> Vec<(String, String)> {
        ids.iter()
            .map(|id| (url_of(id), message.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn an_unavailable_video_is_a_warning_with_the_reason_and_the_full_link() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            errors: failing(&[A], NOT_AVAILABLE),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B])]);
        let out = run_with(videos, &agenda, tmp.path(), CancelFlag::new()).await;
        assert!(
            out.report.complete,
            "a video that no longer exists cannot block the course"
        );
        let notes: Vec<_> = out
            .report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::VideoNotResolved)
            .collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].severity, Severity::Warning);
        assert!(notes[0].message.contains("01. Move 0"));
        assert!(notes[0].message.contains("removed or unavailable video"));
        assert_eq!(notes[0].url.as_deref(), Some(url_of(A).as_str()));
    }

    #[tokio::test]
    async fn an_unavailable_video_is_asked_for_once_however_many_workouts_use_it() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            errors: failing(&[A], NOT_AVAILABLE),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![
            class("w1", "2026-06-02", &[A]),
            class("w2", "2026-06-04", &[A]),
        ]);
        let out = run_with(videos, &agenda, tmp.path(), CancelFlag::new()).await;
        assert_eq!(out.videos.urls(), [url_of(A)]);
        let warnings = out
            .report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::VideoNotResolved)
            .count();
        assert_eq!(warnings, 2, "every place it appears is still reported");
    }

    #[tokio::test]
    async fn a_video_used_by_several_workouts_is_downloaded_once_and_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let agenda = parsed(vec![
            class("w1", "2026-06-02", &[A]),
            class("w2", "2026-06-04", &[A]),
        ]);
        let out = run(&agenda, tmp.path()).await;
        assert_eq!(out.videos.urls(), [url_of(A)]);
        let course = course_dir(tmp.path());
        for dir in ["2026-06/2026-06-02 - t", "2026-06/2026-06-04 - t"] {
            let copy = std::fs::read_to_string(course.join(dir).join("01. Move 0.mp4")).unwrap();
            assert_eq!(copy, "video");
        }
        assert!(out.report.complete && out.report.issues.is_empty());
    }

    #[tokio::test]
    async fn three_bot_checks_in_a_row_stop_the_run_with_one_clear_error() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            errors: failing(&[A, B, C, D, E], BOT),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B, C, D, E])]);
        let out = run_with(videos, &agenda, tmp.path(), CancelFlag::new()).await;
        assert_eq!(
            out.videos.urls().len(),
            3,
            "it stops asking YouTube after the third"
        );
        assert!(!out.report.complete);
        let video_errors: Vec<_> = out
            .report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Video && i.severity == Severity::Error)
            .collect();
        assert_eq!(video_errors.len(), 3);
        assert!(video_errors[0].message.contains("bot check"));
        let course_errors: Vec<_> = out
            .report
            .issues
            .iter()
            .filter(|i| i.kind == IssueKind::Course && i.severity == Severity::Error)
            .collect();
        assert_eq!(course_errors.len(), 1);
        assert!(course_errors[0]
            .message
            .contains("2 videos were not attempted"));
        assert!(course_errors[0].message.contains("try again later"));
    }

    #[tokio::test]
    async fn a_success_resets_the_bot_check_count() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            errors: failing(&[A, B, D, E], BOT),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B, C, D, E])]);
        let out = run_with(videos, &agenda, tmp.path(), CancelFlag::new()).await;
        assert_eq!(out.videos.urls().len(), 5);
        assert!(out
            .report
            .issues
            .iter()
            .all(|i| i.kind != IssueKind::Course));
    }

    #[tokio::test]
    async fn downloads_are_paced_to_stay_polite_with_youtube() {
        let tmp = tempfile::tempdir().unwrap();
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B])]);
        let started = std::time::Instant::now();
        run_paced(
            FakeVideos::default(),
            &agenda,
            tmp.path(),
            CancelFlag::new(),
            Duration::from_millis(80),
        )
        .await;
        assert!(started.elapsed() >= Duration::from_millis(80));
    }

    fn final_dir(tmp: &Path, date: &str) -> PathBuf {
        course_dir(tmp).join(&date[..7]).join(format!("{date} - t"))
    }

    fn staging_dir(tmp: &Path, date: &str) -> PathBuf {
        course_dir(tmp)
            .join(&date[..7])
            .join(format!(".{date} - t.downloading"))
    }

    #[tokio::test]
    async fn a_class_appears_in_the_library_only_when_it_is_finished() {
        let tmp = tempfile::tempdir().unwrap();
        let done = final_dir(tmp.path(), "2026-06-02");
        let videos = FakeVideos {
            watch_dir: Some(done.clone()),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B])]);
        let out = run_with(videos, &agenda, tmp.path(), CancelFlag::new()).await;
        assert_eq!(*out.videos.visible.lock().unwrap(), [false, false]);
        assert!(done.join("01. Move 0.mp4").is_file() && done.join("02. Move 1.mp4").is_file());
        assert!(done.join("Description.html").is_file());
        assert!(!staging_dir(tmp.path(), "2026-06-02").exists());
    }

    #[tokio::test]
    async fn an_interrupted_class_resumes_from_its_hidden_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let hidden = staging_dir(tmp.path(), "2026-06-02");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("01. Move 0.mp4"), "kept").unwrap();
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B])]);
        let out = run(&agenda, tmp.path()).await;
        assert_eq!(out.videos.urls(), [url_of(B)]);
        let done = final_dir(tmp.path(), "2026-06-02");
        assert_eq!(
            std::fs::read_to_string(done.join("01. Move 0.mp4")).unwrap(),
            "kept"
        );
        assert!(done.join("02. Move 1.mp4").is_file());
        assert!(!hidden.exists());
    }

    #[tokio::test]
    async fn a_class_where_nothing_could_be_downloaded_stays_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            errors: failing(&[A], BOT),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A])]);
        let out = run_with(videos, &agenda, tmp.path(), CancelFlag::new()).await;
        assert!(!final_dir(tmp.path(), "2026-06-02").exists());
        assert!(staging_dir(tmp.path(), "2026-06-02").is_dir());
        assert_eq!(out.report.error_count(), 1);
    }

    #[tokio::test]
    async fn a_class_whose_videos_no_longer_exist_is_still_published_with_its_description() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            errors: failing(&[A], NOT_AVAILABLE),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A])]);
        let out = run_with(videos, &agenda, tmp.path(), CancelFlag::new()).await;
        assert!(final_dir(tmp.path(), "2026-06-02")
            .join("Description.html")
            .is_file());
        assert!(out.report.complete);
    }

    #[tokio::test]
    async fn a_cancelled_class_is_not_published_half_done() {
        let tmp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        let videos = FakeVideos {
            cancel_on_first: Some(cancel.clone()),
            ..FakeVideos::default()
        };
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B])]);
        run_with(videos, &agenda, tmp.path(), cancel).await;
        assert!(!final_dir(tmp.path(), "2026-06-02").exists());
        assert!(staging_dir(tmp.path(), "2026-06-02")
            .join("01. Move 0.mp4")
            .is_file());
    }

    #[tokio::test]
    async fn a_class_already_in_the_library_is_completed_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let done = final_dir(tmp.path(), "2026-06-02");
        std::fs::create_dir_all(&done).unwrap();
        std::fs::write(done.join("01. Move 0.mp4"), "kept").unwrap();
        let agenda = parsed(vec![class("w1", "2026-06-02", &[A, B])]);
        let out = run(&agenda, tmp.path()).await;
        assert_eq!(out.videos.urls(), [url_of(B)]);
        assert!(done.join("02. Move 1.mp4").is_file());
        assert!(!staging_dir(tmp.path(), "2026-06-02").exists());
    }

    #[tokio::test]
    async fn a_video_shared_with_a_published_class_can_still_be_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let agenda = parsed(vec![
            class("w1", "2026-06-02", &[A]),
            class("w2", "2026-07-04", &[A]),
        ]);
        let out = run(&agenda, tmp.path()).await;
        assert_eq!(out.videos.urls(), [url_of(A)]);
        assert!(final_dir(tmp.path(), "2026-07-04")
            .join("01. Move 0.mp4")
            .is_file());
        assert!(out.report.issues.is_empty());
    }

    #[tokio::test]
    async fn a_run_that_produced_no_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let videos = FakeVideos {
            write_nothing: true,
            ..FakeVideos::default()
        };
        let out = run_with(videos, &two_workouts(), tmp.path(), CancelFlag::new()).await;
        assert_eq!(out.report.error_count(), 3);
        assert!(!out.report.complete);
    }

    #[tokio::test]
    async fn an_exercise_without_a_video_link_is_a_warning_not_a_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let agenda = parsed(vec![workout(
            "a",
            "2026-06-02",
            "t",
            vec![exercise("Stretch", None)],
        )]);
        let out = run(&agenda, tmp.path()).await;
        assert!(out.report.complete);
        assert_eq!(out.report.issues.len(), 1);
        assert_eq!(out.report.issues[0].kind, IssueKind::VideoNotResolved);
        assert_eq!(out.report.issues[0].severity, Severity::Warning);
        assert!(out.videos.urls().is_empty());
    }

    #[tokio::test]
    async fn a_workout_without_exercises_is_a_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run(
            &parsed(vec![workout("a", "2026-06-02", "t", vec![])]),
            tmp.path(),
        )
        .await;
        assert!(out.report.complete);
        assert_eq!(out.report.issues[0].kind, IssueKind::EmptyLesson);
        assert_eq!(out.report.issues[0].severity, Severity::Warning);
    }

    #[tokio::test]
    async fn skipped_photo_checkins_and_unknown_events_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let mut agenda = two_workouts();
        agenda.photo_checkins = 5;
        agenda.unsupported_events = 2;
        let out = run(&agenda, tmp.path()).await;
        let warnings: Vec<_> = out
            .report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .map(|i| i.message.as_str())
            .collect();
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains('5') && warnings[0].contains("photo"));
        assert!(warnings[1].contains('2'));
        assert!(out.report.complete);
    }

    #[tokio::test]
    async fn an_empty_agenda_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run(&Parsed::default(), tmp.path()).await;
        assert!(!out.report.complete);
        assert_eq!(out.report.issues[0].kind, IssueKind::Course);
        assert_eq!(out.report.issues[0].severity, Severity::Error);
    }

    #[tokio::test]
    async fn a_percent_sign_in_a_name_is_escaped_for_the_yt_dlp_template() {
        let tmp = tempfile::tempdir().unwrap();
        let agenda = parsed(vec![workout(
            "a",
            "2026-06-02",
            "t",
            vec![exercise("50% effort", Some("https://youtu.be/x"))],
        )]);
        let out = run(&agenda, tmp.path()).await;
        let template = out.videos.jobs.lock().unwrap()[0].output_template.clone();
        assert!(template
            .to_string_lossy()
            .ends_with("01. 50%% effort.%(ext)s"));
        assert!(out.report.complete);
    }

    #[tokio::test]
    async fn a_paused_run_holds_before_the_next_workout_until_resumed() {
        let tmp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        cancel.pause();
        let agenda = two_workouts();
        let run = run_with(FakeVideos::default(), &agenda, tmp.path(), cancel.clone());
        tokio::pin!(run);
        let held = tokio::time::timeout(std::time::Duration::from_millis(300), &mut run).await;
        assert!(held.is_err(), "the run holds while paused");
        assert!(!course_dir(tmp.path())
            .join("2026-06/2026-06-02 - Legs/01. Squat.mp4")
            .exists());
        cancel.resume();
        let out = run.await;
        assert!(out.report.complete);
        assert_eq!(out.videos.urls().len(), 3);
    }

    #[tokio::test]
    async fn a_cancelled_run_starts_no_download_and_leaves_the_marker_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        cancel.cancel();
        let out = run_with(FakeVideos::default(), &two_workouts(), tmp.path(), cancel).await;
        assert!(out.videos.urls().is_empty());
        assert!(out.report.cancelled);
        assert!(!out.report.complete);
    }

    #[tokio::test]
    async fn leftover_part_files_are_removed_and_progress_names_the_course() {
        let tmp = tempfile::tempdir().unwrap();
        let legs = course_dir(tmp.path()).join("2026-06/2026-06-02 - Legs");
        std::fs::create_dir_all(&legs).unwrap();
        std::fs::write(legs.join("01. Squat.mp4.part"), "x").unwrap();
        let out = run(&two_workouts(), tmp.path()).await;
        assert!(!legs.join("01. Squat.mp4.part").exists());
        let progress = out.sink.progress.lock().unwrap();
        assert!(progress
            .iter()
            .all(|p| p.group == "Harbiz Training" && p.total == 2));
        assert!(progress
            .iter()
            .any(|p| (p.percent - 100.0).abs() < f32::EPSILON));
    }
}
