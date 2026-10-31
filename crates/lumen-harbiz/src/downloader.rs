//! Workout download orchestration. Every workout is an independent unit; each
//! exercise video is skipped when it is already on disk, so running again only
//! fetches what is new (typically workouts scheduled since the last run). Every
//! failure becomes an [`Issue`] in the report, nothing is swallowed.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::{self, StreamExt};
use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Issue,
    IssueKind, Severity,
};
use lumen_download::report::finalize_folder;
use lumen_download::{VideoError, VideoJob, VideoRunner};
use serde_json::Value;

use crate::error::HarbizError;
use crate::parse::Parsed;
use crate::planner::{
    course_dir, plan_workouts, video_exists, PlannedExercise, PlannedWorkout, COURSE_TITLE,
    WORKOUT_FILE,
};

/// Platform id used in reports and by the app shell.
pub const PLATFORM_ID: &str = "harbiz";

pub struct WorkoutDownloader<'a> {
    pub videos: &'a dyn VideoRunner,
    pub sink: &'a dyn CourseProgressSink,
    pub cancel: &'a CancelFlag,
    pub options: DownloadOptions,
}

struct RunContext {
    total: u32,
    finished: AtomicU32,
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

/// The note shown next to an exercise video in the library.
fn exercise_note(planned: &PlannedExercise) -> String {
    let e = &planned.exercise;
    let name = if e.name.trim().is_empty() {
        "Exercise"
    } else {
        e.name.trim()
    };
    let mut html = format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>{0}</title>\n<h1>{0}</h1>\n",
        escape_html(name)
    );
    if let Some(section) = e.section.as_deref() {
        html += &format!("<p>{}</p>\n", escape_html(section));
    }
    let facts: Vec<String> = [("Sets", &e.sets), ("Target", &e.target), ("Rest", &e.rest)]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| format!("<li>{label}: {}</li>", escape_html(value)))
        .collect();
    if !facts.is_empty() {
        html += &format!("<ul>{}</ul>\n", facts.join(""));
    }
    if !e.notes.is_empty() {
        html += &format!("<p>{}</p>\n", escape_html(&e.notes).replace('\n', "<br>"));
    }
    let sets = logged_sets(&e.result);
    if !sets.is_empty() {
        let items: String = sets
            .iter()
            .map(|set| format!("<li>{}</li>", escape_html(set)))
            .collect();
        html += &format!("<h2>Result</h2>\n<ol>{items}</ol>\n");
    }
    html
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
        if let Err(e) = std::fs::create_dir_all(&workout.dir) {
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
                std::fs::write(workout.dir.join(WORKOUT_FILE), json).map_err(|e| e.to_string())
            });
        if let Err(e) = metadata {
            issues.push(Issue::error(
                IssueKind::Course,
                Some(label),
                None,
                format!("cannot write {WORKOUT_FILE}: {e}"),
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
            let note = workout.dir.join(format!("{}.html", planned.stem));
            if let Err(e) = std::fs::write(&note, exercise_note(planned)) {
                issues.push(Issue::error(
                    IssueKind::Course,
                    Some(label),
                    None,
                    format!("cannot write the note of {}: {e}", planned.stem),
                ));
            }
            let Some(url) = planned.exercise.url.as_deref() else {
                issues.push(Issue::warning(
                    IssueKind::VideoNotResolved,
                    Some(label),
                    None,
                    format!("{} has no video link", planned.stem),
                ));
                continue;
            };
            if video_exists(&workout.dir, planned.number) || self.cancel.is_cancelled() {
                continue;
            }
            self.run_video(ctx, workout, planned, url, &mut issues)
                .await;
        }
        issues
    }

    async fn run_video(
        &self,
        ctx: &RunContext,
        workout: &PlannedWorkout,
        planned: &PlannedExercise,
        url: &str,
        issues: &mut Vec<Issue>,
    ) {
        let job = VideoJob {
            url: url.to_string(),
            referer: None,
            user_agent: None,
            // `%` is yt-dlp's template escape character.
            output_template: workout
                .dir
                .join(format!("{}.%(ext)s", planned.stem.replace('%', "%%"))),
        };
        let label = workout.label.as_str();
        let on_percent = |p: f32| self.emit(ctx, workout, p);
        match self.videos.download(&job, self.cancel, &on_percent).await {
            Ok(()) if video_exists(&workout.dir, planned.number) => {}
            Ok(()) => issues.push(Issue::error(
                IssueKind::Video,
                Some(label),
                Some(url),
                "yt-dlp finished but no video file was found",
            )),
            Err(VideoError::Cancelled) => {}
            Err(e) => issues.push(Issue::error(
                IssueKind::Video,
                Some(label),
                Some(url),
                e.to_string(),
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

    #[derive(Default)]
    struct FakeVideos {
        jobs: Mutex<Vec<VideoJob>>,
        fail_urls: Vec<String>,
        write_nothing: bool,
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
        let sink = Sink::default();
        let report = WorkoutDownloader {
            videos: &videos,
            sink: &sink,
            cancel: &cancel,
            options: DownloadOptions::default(),
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
        let legs = course.join("2026-06-02 - Legs");
        assert!(legs.join("01. Squat.mp4").is_file());
        assert!(legs.join("02. Lunge.mp4").is_file());
        assert!(course.join("2026-06-04 - Core/01. Plank.mp4").is_file());
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
    async fn the_exercise_note_is_escaped_html_with_sets_and_results() {
        let tmp = tempfile::tempdir().unwrap();
        run(&two_workouts(), tmp.path()).await;
        let note = std::fs::read_to_string(
            course_dir(tmp.path()).join("2026-06-02 - Legs/01. Squat.html"),
        )
        .unwrap();
        assert!(note.contains("Squat"));
        assert!(note.contains("Keep &lt;it&gt; slow &amp; steady"));
        assert!(!note.contains("<it>"));
        assert!(note.contains("reps: 10"));
        assert!(note.contains("weight: 2.5"));
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
                .join("2026-06-02 - Legs")
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
            .join("2026-10-31 - Kicks/01. Kick.mp4")
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
        assert!(course.join("2026-06-04 - Core/01. Plank.mp4").is_file());
        assert!(!course.join(".complete").exists());
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
        let legs = course_dir(tmp.path()).join("2026-06-02 - Legs");
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
