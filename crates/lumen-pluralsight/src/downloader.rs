//! Pluralsight clip delivery and report orchestration.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Issue,
    IssueKind, PlatformError,
};
use lumen_download::report::{finalize_folder, set_complete, REPORT_FILE};
use lumen_download::{VideoError, VideoJob, VideoRunner};

use crate::client::PluralsightApi;
use crate::parse::CourseInfo;
use crate::planner::{course_dir, plan_clips, PlannedClip};
use crate::PluralsightError;

const PLATFORM_ID: &str = "pluralsight";

#[derive(Debug, Clone)]
pub struct Retry {
    pub attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            attempts: 3,
            base_delay: Duration::from_secs(3),
            max_delay: Duration::from_secs(6),
        }
    }
}

pub struct PluralsightDownloader {
    pub api: Arc<dyn PluralsightApi>,
    pub videos: Arc<dyn VideoRunner>,
    pub retry: Retry,
}

impl PluralsightDownloader {
    pub async fn download(
        &self,
        course: &CourseInfo,
        root: &Path,
        _options: &DownloadOptions,
        sink: &dyn CourseProgressSink,
        cancel: &CancelFlag,
    ) -> Result<DownloadReport, PlatformError> {
        let dir = course_dir(root, course)?;
        prepare_directory(root, &dir).await?;
        set_complete_async(dir.clone(), false).await?;

        let group = course.id.clone().unwrap_or_else(|| course.title.clone());
        let mut report = DownloadReport::new(PLATFORM_ID, &group);
        if let Some(issue) = &course.issue {
            report.push(Issue::error(IssueKind::Course, None, None, issue));
        }
        if cancel.is_cancelled() {
            report.cancelled = true;
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "download cancelled",
            ));
            return self.finish(dir, report, sink).await;
        }
        let Some(course_id) = course.id.as_deref().filter(|id| valid_identifier(id)) else {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "course record is missing a valid identifier",
            ));
            return self.finish(dir, report, sink).await;
        };

        let toc_result = tokio::select! {
            biased;
            _ = wait_for_cancel(cancel) => None,
            result = self.api.toc(course_id) => Some(result),
        };
        let modules = match toc_result {
            None => {
                report.cancelled = true;
                report.push(Issue::error(
                    IssueKind::Course,
                    None,
                    None,
                    "download cancelled",
                ));
                return self.finish(dir, report, sink).await;
            }
            Some(Ok(modules)) => modules,
            Some(Err(error)) => {
                report.push(Issue::error(
                    IssueKind::Course,
                    None,
                    None,
                    safe_api_error(&error),
                ));
                return self.finish(dir, report, sink).await;
            }
        };
        let planned = plan_clips(&dir, &modules)?;
        let total = planned.len().saturating_add(
            modules
                .iter()
                .filter(|module| module.issue.is_some() || module.clips.is_empty())
                .count(),
        );
        report.total_lessons = u32::try_from(total).unwrap_or(u32::MAX);
        if modules.is_empty() {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "course has no modules",
            ));
        }
        for module in &modules {
            if let Some(issue) = &module.issue {
                report.push(Issue::error(
                    IssueKind::ModulePage,
                    Some(&module.title),
                    None,
                    issue,
                ));
                report.completed_lessons = report.completed_lessons.saturating_add(1);
            } else if module.clips.is_empty() {
                report.push(Issue::error(
                    IssueKind::ModulePage,
                    Some(&module.title),
                    None,
                    "module contains no clips",
                ));
                report.completed_lessons = report.completed_lessons.saturating_add(1);
            }
        }

        for item in planned {
            if cancel.is_cancelled() {
                report.cancelled = true;
                report.push(Issue::error(
                    IssueKind::Course,
                    Some(&item.label),
                    None,
                    "download cancelled",
                ));
                break;
            }
            if item.clip.issue.is_some() {
                report.push(Issue::error(
                    IssueKind::VideoNotResolved,
                    Some(&item.label),
                    None,
                    item.clip.issue.as_deref().unwrap_or("invalid clip record"),
                ));
                report.completed_lessons = report.completed_lessons.saturating_add(1);
                self.progress(sink, course, &item, 100.0, &report);
                continue;
            }
            match self
                .download_clip(course, &item, &dir, sink, cancel, &mut report)
                .await
            {
                Ok(()) => report.completed_lessons = report.completed_lessons.saturating_add(1),
                Err(ClipFailure::Cancelled) => {
                    report.cancelled = true;
                    report.push(Issue::error(
                        IssueKind::Course,
                        Some(&item.label),
                        None,
                        "download cancelled",
                    ));
                    report.completed_lessons = report.completed_lessons.saturating_add(1);
                    self.progress(sink, course, &item, 100.0, &report);
                    break;
                }
                Err(ClipFailure::Issue(issue)) => {
                    report.push(issue);
                    report.completed_lessons = report.completed_lessons.saturating_add(1);
                    self.progress(sink, course, &item, 100.0, &report);
                }
            }
        }
        // Processing is intentionally sequential for deterministic report and
        // cancellation handling; requested parallelism is conservatively capped at one.
        self.finish(dir, report, sink).await
    }

    async fn download_clip(
        &self,
        course: &CourseInfo,
        item: &PlannedClip,
        course_dir: &Path,
        sink: &dyn CourseProgressSink,
        cancel: &CancelFlag,
        report: &mut DownloadReport,
    ) -> Result<(), ClipFailure> {
        let (Some(clip_id), Some(version)) =
            (item.clip.id.as_deref(), item.clip.version.as_deref())
        else {
            return Err(ClipFailure::Issue(Issue::error(
                IssueKind::VideoNotResolved,
                Some(&item.label),
                None,
                "clip record is missing an identifier or version",
            )));
        };
        prepare_directory(course_dir, item.output.parent().unwrap_or(course_dir))
            .await
            .map_err(|_| {
                ClipFailure::Issue(Issue::error(
                    IssueKind::Video,
                    Some(&item.label),
                    None,
                    "cannot create a confined clip directory",
                ))
            })?;
        if runner_temporary_symlink(&item.output).await.unwrap_or(true) {
            return Err(ClipFailure::Issue(Issue::error(
                IssueKind::Video,
                Some(&item.label),
                None,
                "video runner temporary output is a symbolic link",
            )));
        }
        match tokio::fs::symlink_metadata(&item.output).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ClipFailure::Issue(Issue::error(
                    IssueKind::Video,
                    Some(&item.label),
                    None,
                    "clip output is a symbolic link",
                )));
            }
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => {
                self.progress(sink, course, item, 100.0, report);
                return Ok(());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(ClipFailure::Issue(Issue::error(
                    IssueKind::Video,
                    Some(&item.label),
                    None,
                    "cannot inspect clip output",
                )))
            }
        }

        self.progress(sink, course, item, 0.0, report);
        let delivery_result = tokio::select! {
            biased;
            _ = wait_for_cancel(cancel) => None,
            result = self.api.clip_delivery(clip_id, version) => Some(result),
        };
        let delivery = match delivery_result {
            None => return Err(ClipFailure::Cancelled),
            Some(Ok(delivery)) => delivery,
            Some(Err(error)) => {
                let message = if error == PluralsightError::ProtectedContent {
                    "clip is protected or encrypted; downloading is not supported"
                } else {
                    safe_api_error(&error)
                };
                return Err(ClipFailure::Issue(Issue::error(
                    IssueKind::VideoNotResolved,
                    Some(&item.label),
                    None,
                    message,
                )));
            }
        };
        if cancel.is_cancelled() {
            return Err(ClipFailure::Cancelled);
        }
        let template = item.output.with_extension("%(ext)s");
        let job = VideoJob {
            url: delivery.stream_url.clone(),
            referer: None,
            user_agent: None,
            output_template: template,
        };
        let attempts = self.retry.attempts.max(1);
        for attempt in 1..=attempts {
            if cancel.is_cancelled() {
                return Err(ClipFailure::Cancelled);
            }
            let callback = |percent: f32| {
                sink.on_progress(&DownloadProgress {
                    group: course.id.clone().unwrap_or_else(|| course.title.clone()),
                    course: course.title.clone(),
                    lesson: item.label.clone(),
                    percent: percent.clamp(0.0, 100.0),
                    completed: report.completed_lessons,
                    total: report.total_lessons,
                });
            };
            match self.videos.download(&job, cancel, &callback).await {
                Ok(()) => {
                    let metadata =
                        tokio::fs::symlink_metadata(&item.output)
                            .await
                            .map_err(|_| {
                                ClipFailure::Issue(Issue::error(
                                    IssueKind::Video,
                                    Some(&item.label),
                                    Some(&delivery.stream_url),
                                    "video runner did not produce the expected MP4 file",
                                ))
                            })?;
                    if !metadata.is_file()
                        || metadata.file_type().is_symlink()
                        || metadata.len() == 0
                    {
                        return Err(ClipFailure::Issue(Issue::error(
                            IssueKind::Video,
                            Some(&item.label),
                            Some(&delivery.stream_url),
                            "video runner produced an empty or unsafe output file",
                        )));
                    }
                    self.progress(sink, course, item, 100.0, report);
                    return Ok(());
                }
                Err(VideoError::Cancelled) => return Err(ClipFailure::Cancelled),
                Err(error) if video_retryable(&error) && attempt < attempts => {
                    report.push(Issue::warning(
                        IssueKind::Video,
                        Some(&item.label),
                        Some(&delivery.stream_url),
                        format!("video attempt {attempt}/{attempts} failed; retrying"),
                    ));
                    sleep_or_cancel(retry_delay(&self.retry, attempt), cancel).await?;
                }
                Err(_) => {
                    return Err(ClipFailure::Issue(Issue::error(
                        IssueKind::Video,
                        Some(&item.label),
                        Some(&delivery.stream_url),
                        format!("video download failed on attempt {attempt}/{attempts}"),
                    )))
                }
            }
        }
        Ok(())
    }

    fn progress(
        &self,
        sink: &dyn CourseProgressSink,
        course: &CourseInfo,
        item: &PlannedClip,
        percent: f32,
        report: &DownloadReport,
    ) {
        sink.on_progress(&DownloadProgress {
            group: course.id.clone().unwrap_or_else(|| course.title.clone()),
            course: course.title.clone(),
            lesson: item.label.clone(),
            percent,
            completed: report.completed_lessons,
            total: report.total_lessons,
        });
    }

    async fn finish(
        &self,
        dir: PathBuf,
        mut report: DownloadReport,
        sink: &dyn CourseProgressSink,
    ) -> Result<DownloadReport, PlatformError> {
        let report_path = dir.join(REPORT_FILE);
        let unsafe_destination = match tokio::fs::symlink_metadata(&report_path).await {
            Ok(metadata) => metadata.file_type().is_symlink() || !metadata.is_file(),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        };
        if unsafe_destination {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                "cannot write the download report: unsafe destination",
            ));
            report.finish();
            let marker_result = set_complete_async(dir, false).await;
            if marker_result.is_err() {
                report.push(Issue::error(
                    IssueKind::Course,
                    None,
                    None,
                    "cannot clear the completion marker",
                ));
                report.finish();
            }
            sink.on_report(&report);
            return Err(PlatformError::Io(
                "Pluralsight download report could not be written".into(),
            ));
        }

        let (report, report_failed) = tokio::task::spawn_blocking(move || {
            finalize_folder(&dir, &mut report);
            let report_failed = report.issues.iter().any(|issue| {
                issue.kind == IssueKind::Course
                    && issue.message.contains("cannot write the download report")
            });
            (report, report_failed)
        })
        .await
        .map_err(|_| PlatformError::Io("cannot finalize Pluralsight download report".into()))?;
        sink.on_report(&report);
        if report_failed {
            return Err(PlatformError::Io(
                "Pluralsight download report could not be written".into(),
            ));
        }
        Ok(report)
    }
}

enum ClipFailure {
    Cancelled,
    Issue(Issue),
}

fn video_retryable(error: &VideoError) -> bool {
    let VideoError::Failed(message) = error else {
        return false;
    };
    let lower = message.to_ascii_lowercase();
    let status = lower
        .find("http error ")
        .and_then(|start| lower[start + "http error ".len()..].get(..3))
        .and_then(|digits| digits.parse::<u16>().ok());
    status == Some(429)
        || status.is_some_and(|code| (500..600).contains(&code))
        || [
            "timed out",
            "connection reset",
            "temporary failure",
            "network is unreachable",
        ]
        .iter()
        .any(|indicator| lower.contains(indicator))
}

fn valid_identifier(value: &str) -> bool {
    !value.trim().is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', '\0'])
}

fn safe_api_error(error: &PluralsightError) -> &'static str {
    match error {
        PluralsightError::Unauthorized(_) => "session expired or unauthorized",
        PluralsightError::HttpStatus(_) | PluralsightError::Network(_) => "service request failed",
        PluralsightError::ProtectedContent => "clip is protected or encrypted",
        PluralsightError::NoStream => "no supported HLS stream is available",
        _ => "invalid course response",
    }
}

fn retry_delay(retry: &Retry, attempt: u32) -> Duration {
    retry
        .base_delay
        .saturating_mul(attempt)
        .min(retry.max_delay)
}

async fn sleep_or_cancel(delay: Duration, cancel: &CancelFlag) -> Result<(), ClipFailure> {
    tokio::select! {
        _ = tokio::time::sleep(delay) => Ok(()),
        _ = async {
            while !cancel.is_cancelled() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        } => Err(ClipFailure::Cancelled),
    }
}

async fn prepare_directory(root: &Path, dir: &Path) -> Result<(), PlatformError> {
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|_| PlatformError::Io("cannot create library root".into()))?;
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|_| PlatformError::Io("cannot resolve library root".into()))?;
    let relative = dir.strip_prefix(root).map_err(|_| {
        PlatformError::InvalidInput("Pluralsight output directory escapes the library root".into())
    })?;
    let mut current = canonical_root.clone();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(PlatformError::InvalidInput(
                "Pluralsight output path contains an invalid component".into(),
            ));
        };
        let next = current.join(name);
        match tokio::fs::symlink_metadata(&next).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(PlatformError::InvalidInput(
                    "Pluralsight output path contains an unsafe ancestor".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir(&next).await.map_err(|_| {
                    PlatformError::Io("cannot create Pluralsight output directory".into())
                })?;
            }
            Err(_) => {
                return Err(PlatformError::Io(
                    "cannot inspect Pluralsight output path".into(),
                ))
            }
        }
        current = tokio::fs::canonicalize(&next)
            .await
            .map_err(|_| PlatformError::Io("cannot resolve Pluralsight output directory".into()))?;
        if !current.starts_with(&canonical_root) {
            return Err(PlatformError::InvalidInput(
                "Pluralsight output directory escapes the library root".into(),
            ));
        }
    }
    Ok(())
}

async fn set_complete_async(dir: PathBuf, complete: bool) -> Result<(), PlatformError> {
    tokio::task::spawn_blocking(move || set_complete(&dir, complete))
        .await
        .map_err(|_| PlatformError::Io("cannot update Pluralsight completion marker".into()))?
        .map_err(|_| PlatformError::Io("cannot update Pluralsight completion marker".into()))
}

async fn wait_for_cancel(cancel: &CancelFlag) {
    while !cancel.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn runner_temporary_symlink(output: &Path) -> std::io::Result<bool> {
    let parent = output
        .parent()
        .ok_or_else(|| std::io::Error::other("output has no parent"))?;
    let name = output
        .file_stem()
        .ok_or_else(|| std::io::Error::other("output has no name"))?
        .to_string_lossy();
    let prefix = format!("{name}.");
    let mut entries = tokio::fs::read_dir(parent).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = file_name.strip_prefix(&prefix) else {
            continue;
        };
        let fragment = rest
            .strip_prefix('f')
            .and_then(|tail| tail.chars().next())
            .is_some_and(|digit| digit.is_ascii_digit());
        let temporary = rest.contains(".part")
            || rest.ends_with(".ytdl")
            || rest.starts_with("temp.")
            || fragment;
        if temporary
            && tokio::fs::symlink_metadata(entry.path())
                .await?
                .file_type()
                .is_symlink()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use lumen_core::{DownloadProgress, IssueKind, NullSink};
    use lumen_download::{VideoJob, VideoRunner};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeApi {
        toc: Result<Vec<crate::ModuleInfo>, PluralsightError>,
        delivery: Result<crate::ClipDelivery, PluralsightError>,
        delivery_calls: Mutex<usize>,
        toc_calls: Mutex<usize>,
        toc_delay: Duration,
        delivery_delay: Duration,
    }

    #[async_trait]
    impl PluralsightApi for FakeApi {
        async fn validate(&self) -> Result<(), PluralsightError> {
            Ok(())
        }
        async fn courses(&self) -> Result<Vec<CourseInfo>, PluralsightError> {
            Ok(vec![])
        }
        async fn toc(&self, _: &str) -> Result<Vec<crate::ModuleInfo>, PluralsightError> {
            *self.toc_calls.lock().unwrap() += 1;
            tokio::time::sleep(self.toc_delay).await;
            self.toc.clone()
        }
        async fn clip_delivery(
            &self,
            _: &str,
            _: &str,
        ) -> Result<crate::ClipDelivery, PluralsightError> {
            *self.delivery_calls.lock().unwrap() += 1;
            tokio::time::sleep(self.delivery_delay).await;
            self.delivery.clone()
        }
    }

    #[derive(Default)]
    struct FakeVideo {
        jobs: Mutex<Vec<VideoJob>>,
        outcomes: Mutex<VecDeque<Result<(), VideoError>>>,
        cancel_on_run: Option<CancelFlag>,
    }

    #[async_trait]
    impl VideoRunner for FakeVideo {
        async fn download(
            &self,
            job: &VideoJob,
            cancel: &CancelFlag,
            progress: &(dyn Fn(f32) + Send + Sync),
        ) -> Result<(), VideoError> {
            self.jobs.lock().unwrap().push(job.clone());
            if let Some(flag) = &self.cancel_on_run {
                flag.cancel();
            }
            progress(50.0);
            if cancel.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            let result = self.outcomes.lock().unwrap().pop_front().unwrap_or(Ok(()));
            if result.is_ok() {
                let output = job
                    .output_template
                    .to_string_lossy()
                    .replace("%(ext)s", "mp4");
                tokio::fs::write(output, b"synthetic video")
                    .await
                    .map_err(|_| VideoError::Failed("fake write failed".into()))?;
            }
            result
        }
    }

    #[derive(Default)]
    struct Sink {
        progress: Mutex<Vec<DownloadProgress>>,
        reports: Mutex<Vec<DownloadReport>>,
    }
    impl CourseProgressSink for Sink {
        fn on_progress(&self, value: &DownloadProgress) {
            self.progress.lock().unwrap().push(value.clone());
        }
        fn on_report(&self, value: &DownloadReport) {
            self.reports.lock().unwrap().push(value.clone());
        }
    }

    fn course() -> CourseInfo {
        CourseInfo {
            id: Some("course-1".into()),
            title: "Rust Course".into(),
            level: None,
            image_url: None,
            issue: None,
        }
    }
    fn modules() -> Vec<crate::ModuleInfo> {
        vec![crate::ModuleInfo {
            title: "Start".into(),
            issue: None,
            clips: vec![crate::Clip {
                id: Some("clip-1".into()),
                version: Some("v1".into()),
                title: "Intro".into(),
                issue: None,
            }],
        }]
    }
    fn api() -> Arc<FakeApi> {
        Arc::new(FakeApi {
            toc: Ok(modules()),
            delivery: Ok(crate::ClipDelivery {
                stream_url: "https://cdn.example.test/clip.m3u8?sig=SENSITIVE".into(),
            }),
            delivery_calls: Mutex::new(0),
            toc_calls: Mutex::new(0),
            toc_delay: Duration::ZERO,
            delivery_delay: Duration::ZERO,
        })
    }
    fn downloader(api: Arc<FakeApi>, video: Arc<FakeVideo>, retry: Retry) -> PluralsightDownloader {
        PluralsightDownloader {
            api,
            videos: video,
            retry,
        }
    }
    fn quick_retry(attempts: u32) -> Retry {
        Retry {
            attempts,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn downloads_into_numbered_course_module_clip_tree_and_redacts_signed_url() {
        let temp = tempfile::tempdir().unwrap();
        let api = api();
        let video = Arc::new(FakeVideo::default());
        let sink = Sink::default();
        let report = downloader(api, video.clone(), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &sink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        let path = plan_clips(&course_dir(temp.path(), &course()).unwrap(), &modules()).unwrap()[0]
            .output
            .clone();
        assert!(path.is_file());
        assert!(report.complete);
        assert_eq!(report.total_lessons, 1);
        assert_eq!(report.completed_lessons, 1);
        assert_eq!(sink.reports.lock().unwrap().len(), 1);
        assert!(sink
            .progress
            .lock()
            .unwrap()
            .iter()
            .any(|progress| progress.percent == 50.0));
        assert_eq!(video.jobs.lock().unwrap()[0].referer, None);
        assert!(!format!("{report:?}").contains("SENSITIVE"));
    }

    #[tokio::test]
    async fn malformed_course_and_clip_rows_are_reported_not_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let mut bad = course();
        bad.id = None;
        bad.issue = Some("course record is missing an id".into());
        let report = downloader(api(), Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &bad,
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert_eq!(report.error_count(), 2);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == IssueKind::Course));
        assert!(!report.complete);

        let temp = tempfile::tempdir().unwrap();
        let fake = Arc::new(FakeApi {
            toc: Ok(vec![crate::ModuleInfo {
                title: "M".into(),
                issue: None,
                clips: vec![crate::Clip {
                    id: None,
                    version: None,
                    title: "Broken".into(),
                    issue: Some("clip record is missing an id".into()),
                }],
            }]),
            ..(*api()).clone_for_test()
        });
        let video = Arc::new(FakeVideo::default());
        let sink = Sink::default();
        let report = downloader(fake, video.clone(), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &sink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == IssueKind::VideoNotResolved));
        assert!(video.jobs.lock().unwrap().is_empty());
        assert_eq!(report.total_lessons, 1);
        assert_eq!(report.completed_lessons, 1);
        assert!(sink.progress.lock().unwrap().iter().any(|progress| {
            progress.percent == 100.0 && progress.completed == 1 && progress.total == 1
        }));
    }

    #[tokio::test]
    async fn empty_course_and_protected_delivery_leave_error_reports_without_runner() {
        let temp = tempfile::tempdir().unwrap();
        let empty_api = Arc::new(FakeApi {
            toc: Ok(vec![]),
            delivery: Err(PluralsightError::NoStream),
            delivery_calls: Mutex::new(0),
            toc_calls: Mutex::new(0),
            toc_delay: Duration::ZERO,
            delivery_delay: Duration::ZERO,
        });
        let report = downloader(empty_api, Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.message.contains("no modules")));

        let temp = tempfile::tempdir().unwrap();
        let api = Arc::new(FakeApi {
            toc: Ok(modules()),
            delivery: Err(PluralsightError::ProtectedContent),
            delivery_calls: Mutex::new(0),
            toc_calls: Mutex::new(0),
            toc_delay: Duration::ZERO,
            delivery_delay: Duration::ZERO,
        });
        let video = Arc::new(FakeVideo::default());
        let report = downloader(api, video.clone(), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == IssueKind::VideoNotResolved
                && issue.message.contains("protected")));
        assert!(video.jobs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_module_is_reported_and_never_marks_course_complete() {
        let temp = tempfile::tempdir().unwrap();
        let api = Arc::new(FakeApi {
            toc: Ok(vec![crate::ModuleInfo {
                title: "Empty module".into(),
                clips: vec![],
                issue: None,
            }]),
            ..(*api()).clone_for_test()
        });
        let report = downloader(api, Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(!report.complete);
        assert_eq!(report.total_lessons, 1);
        assert_eq!(report.completed_lessons, 1);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == IssueKind::ModulePage));
        assert!(!course_dir(temp.path(), &course())
            .unwrap()
            .join(".complete")
            .exists());
    }

    #[tokio::test]
    async fn retries_video_failure_and_exhaustion_is_reported_without_signed_query() {
        let temp = tempfile::tempdir().unwrap();
        let api = api();
        let video = Arc::new(FakeVideo {
            outcomes: Mutex::new(VecDeque::from([
                Err(VideoError::Failed("HTTP Error 503: sig=SENSITIVE".into())),
                Ok(()),
            ])),
            ..FakeVideo::default()
        });
        let report = downloader(api.clone(), video.clone(), quick_retry(2))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(report.complete);
        assert_eq!(video.jobs.lock().unwrap().len(), 2);
        assert!(report
            .issues
            .iter()
            .any(|i| i.severity == lumen_core::Severity::Warning));
        assert!(!format!("{report:?}").contains("SENSITIVE"));

        let temp = tempfile::tempdir().unwrap();
        let video = Arc::new(FakeVideo {
            outcomes: Mutex::new(VecDeque::from([
                Err(VideoError::Failed("HTTP Error 503: secret".into())),
                Err(VideoError::Failed("HTTP Error 503: secret".into())),
            ])),
            ..FakeVideo::default()
        });
        let report = downloader(api, video, quick_retry(2))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(!report.complete);
        assert!(report
            .issues
            .iter()
            .any(|i| i.severity == lumen_core::Severity::Error && i.kind == IssueKind::Video));
        assert!(!format!("{report:?}").contains("secret"));
        assert_eq!(report.completed_lessons, 1);
    }

    #[tokio::test]
    async fn permanent_video_spawn_error_is_not_retried() {
        let temp = tempfile::tempdir().unwrap();
        let video = Arc::new(FakeVideo {
            outcomes: Mutex::new(VecDeque::from([Err(VideoError::NotFound("yt-dlp".into()))])),
            ..FakeVideo::default()
        });
        let report = downloader(api(), video.clone(), quick_retry(3))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert_eq!(video.jobs.lock().unwrap().len(), 1);
        assert!(!report.complete);
        assert_eq!(report.completed_lessons, 1);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.message.contains("on attempt 1/3")));
    }

    #[tokio::test]
    async fn existing_nonempty_video_is_skipped_and_stale_marker_is_cleared_on_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = plan_clips(&course_dir(temp.path(), &course()).unwrap(), &modules()).unwrap()[0]
            .output
            .clone();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"existing").await.unwrap();
        let video = Arc::new(FakeVideo::default());
        let report = downloader(api(), video.clone(), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(report.complete);
        assert!(video.jobs.lock().unwrap().is_empty());

        let temp = tempfile::tempdir().unwrap();
        let mut malformed = course();
        malformed.id = None;
        let course_dir = course_dir(temp.path(), &malformed).unwrap();
        tokio::fs::create_dir_all(&course_dir).await.unwrap();
        tokio::fs::write(course_dir.join(".complete"), b"stale")
            .await
            .unwrap();
        let report = downloader(api(), Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &malformed,
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(!report.complete);
        assert!(!course_dir.join(".complete").exists());
    }

    #[tokio::test]
    async fn prior_clip_version_is_preserved_but_new_version_is_downloaded() {
        let temp = tempfile::tempdir().unwrap();
        let course = course();
        let previous_plan =
            plan_clips(&course_dir(temp.path(), &course).unwrap(), &modules()).unwrap();
        let previous_output = previous_plan[0].output.clone();
        tokio::fs::create_dir_all(previous_output.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&previous_output, b"previous-version")
            .await
            .unwrap();

        let mut latest_modules = modules();
        latest_modules[0].clips[0].version = Some("v2".into());
        let api = Arc::new(FakeApi {
            toc: Ok(latest_modules),
            ..(*api()).clone_for_test()
        });
        let video = Arc::new(FakeVideo::default());
        let report = downloader(api.clone(), video.clone(), quick_retry(1))
            .download(
                &course,
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        let latest = plan_clips(&course_dir(temp.path(), &course).unwrap(), &{
            let mut modules = modules();
            modules[0].clips[0].version = Some("v2".into());
            modules
        })
        .unwrap();

        assert_ne!(previous_output, latest[0].output);
        assert_eq!(
            tokio::fs::read(&previous_output).await.unwrap(),
            b"previous-version"
        );
        assert!(latest[0].output.is_file());
        assert!(report.complete);
        assert_eq!(*api.delivery_calls.lock().unwrap(), 1);
        assert_eq!(video.jobs.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn impossible_unicode_library_root_fails_before_filesystem_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("目录🌞".repeat(62));
        let result = downloader(api(), Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &course(),
                &root,
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await;

        assert!(matches!(result, Err(PlatformError::InvalidInput(_))));
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn cancel_is_reported_and_output_symlink_cannot_escape_root() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        let video = Arc::new(FakeVideo {
            cancel_on_run: Some(cancel.clone()),
            ..FakeVideo::default()
        });
        let report = downloader(api(), video, quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &cancel,
            )
            .await
            .unwrap();
        assert!(report.cancelled);
        assert!(!report.complete);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.message == "download cancelled"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let temp = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            tokio::fs::create_dir_all(temp.path().join("Pluralsight"))
                .await
                .unwrap();
            symlink(outside.path(), course_dir(temp.path(), &course()).unwrap()).unwrap();
            assert!(
                prepare_directory(temp.path(), &course_dir(temp.path(), &course()).unwrap())
                    .await
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn course_symlink_escape_is_rejected_before_creating_outside_course_folder() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(temp.path()).await.unwrap();
        symlink(outside.path(), temp.path().join("Pluralsight")).unwrap();
        let result = downloader(api(), Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await;
        assert!(result.is_err());
        assert!(!outside
            .path()
            .join(
                course_dir(temp.path(), &course())
                    .unwrap()
                    .file_name()
                    .unwrap()
            )
            .exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_yt_dlp_part_file_is_rejected_before_runner_invocation() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), b"sentinel").unwrap();
        let part = plan_clips(&course_dir(temp.path(), &course()).unwrap(), &modules()).unwrap()[0]
            .output
            .with_extension("mp4.part");
        tokio::fs::create_dir_all(part.parent().unwrap())
            .await
            .unwrap();
        symlink(outside.path(), &part).unwrap();
        let video = Arc::new(FakeVideo::default());
        let report = downloader(api(), video.clone(), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await
            .unwrap();
        assert!(video.jobs.lock().unwrap().is_empty());
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.kind == IssueKind::Video));
        assert_eq!(tokio::fs::read(outside.path()).await.unwrap(), b"sentinel");
    }

    #[tokio::test]
    async fn pre_cancel_and_cancellation_during_toc_are_reported_and_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let pre_cancel = CancelFlag::new();
        pre_cancel.cancel();
        let fake_api = api();
        let report = downloader(
            fake_api.clone(),
            Arc::new(FakeVideo::default()),
            quick_retry(1),
        )
        .download(
            &course(),
            temp.path(),
            &DownloadOptions::default(),
            &NullSink,
            &pre_cancel,
        )
        .await
        .unwrap();
        assert!(report.cancelled);
        assert_eq!(*fake_api.toc_calls.lock().unwrap(), 0);
        assert!(course_dir(temp.path(), &course())
            .unwrap()
            .join("download-report.json")
            .is_file());

        let temp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        let api = Arc::new(FakeApi {
            toc_delay: Duration::from_secs(1),
            ..(*api()).clone_for_test()
        });
        let signal_flag = cancel.clone();
        let signal = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            signal_flag.cancel();
        });
        let started = std::time::Instant::now();
        let report = downloader(api, Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &cancel,
            )
            .await
            .unwrap();
        signal.await.unwrap();
        assert!(report.cancelled);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(course_dir(temp.path(), &course())
            .unwrap()
            .join("download-report.json")
            .is_file());
        assert!(!report.complete);
    }

    #[tokio::test]
    async fn cancellation_during_delivery_stops_before_video_runner() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        let api = Arc::new(FakeApi {
            delivery_delay: Duration::from_secs(1),
            ..(*api()).clone_for_test()
        });
        let signal_flag = cancel.clone();
        let signal = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            signal_flag.cancel();
        });
        let started = std::time::Instant::now();
        let video = Arc::new(FakeVideo::default());
        let report = downloader(api, video.clone(), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &cancel,
            )
            .await
            .unwrap();
        signal.await.unwrap();
        assert!(report.cancelled);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(report.completed_lessons, 1);
        assert!(video.jobs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn report_write_failure_returns_typed_io_error_and_removes_stale_marker() {
        let temp = tempfile::tempdir().unwrap();
        let course_dir = course_dir(temp.path(), &course()).unwrap();
        tokio::fs::create_dir_all(&course_dir).await.unwrap();
        tokio::fs::create_dir_all(course_dir.join("download-report.json"))
            .await
            .unwrap();
        tokio::fs::write(course_dir.join(".complete"), b"stale")
            .await
            .unwrap();
        let result = downloader(api(), Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &NullSink,
                &CancelFlag::new(),
            )
            .await;
        assert!(matches!(result, Err(PlatformError::Io(_))));
        assert!(!course_dir.join(".complete").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn report_symlink_is_rejected_without_writing_outside_or_completing() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let course_dir = course_dir(temp.path(), &course()).unwrap();
        tokio::fs::create_dir_all(&course_dir).await.unwrap();
        let outside_report = outside.path().join("report.json");
        tokio::fs::write(&outside_report, b"sentinel")
            .await
            .unwrap();
        symlink(&outside_report, course_dir.join("download-report.json")).unwrap();
        tokio::fs::write(course_dir.join(".complete"), b"stale")
            .await
            .unwrap();
        let sink = Sink::default();

        let result = downloader(api(), Arc::new(FakeVideo::default()), quick_retry(1))
            .download(
                &course(),
                temp.path(),
                &DownloadOptions::default(),
                &sink,
                &CancelFlag::new(),
            )
            .await;

        assert!(matches!(result, Err(PlatformError::Io(_))));
        assert_eq!(tokio::fs::read(&outside_report).await.unwrap(), b"sentinel");
        assert!(!course_dir.join(".complete").exists());
        let reported = sink.reports.lock().unwrap();
        let issue = reported[0]
            .issues
            .iter()
            .find(|issue| issue.message.contains("cannot write the download report"))
            .expect("report path rejection is included in the report");
        assert!(!issue
            .message
            .contains(outside.path().to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn cancellation_interrupts_retry_backoff() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = CancelFlag::new();
        let video = Arc::new(FakeVideo {
            outcomes: Mutex::new(VecDeque::from([Err(VideoError::Failed(
                "HTTP Error 503: transient".into(),
            ))])),
            ..FakeVideo::default()
        });
        let task_cancel = cancel.clone();
        let signal = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            task_cancel.cancel();
        });
        let report = downloader(
            api(),
            video.clone(),
            Retry {
                attempts: 3,
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(6),
            },
        )
        .download(
            &course(),
            temp.path(),
            &DownloadOptions::default(),
            &NullSink,
            &cancel,
        )
        .await
        .unwrap();
        signal.await.unwrap();
        assert!(report.cancelled);
        assert_eq!(video.jobs.lock().unwrap().len(), 1);
    }

    impl FakeApi {
        fn clone_for_test(&self) -> Self {
            Self {
                toc: self.toc.clone(),
                delivery: self.delivery.clone(),
                delivery_calls: Mutex::new(0),
                toc_calls: Mutex::new(0),
                toc_delay: self.toc_delay,
                delivery_delay: self.delivery_delay,
            }
        }
    }
}
