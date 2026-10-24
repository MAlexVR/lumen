//! What every platform service shares: error codes, event payloads and the
//! registry of running downloads. It holds no Tauri types, so it is tested
//! without a window; `lib.rs` only adapts it to commands and events.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use lumen_core::{
    CancelFlag, CoursePlatform, CourseProgressSink, DownloadOptions, DownloadProgress,
    DownloadReport, Issue, IssueKind, PlatformError,
};
use serde::Serialize;

/// Stable machine-readable error codes for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotLoggedIn,
    SessionExpired,
    /// Skool or Pluralsight: the pasted cookies are malformed.
    InvalidCookies,
    /// Hotmart: the pasted token (or cookies) is malformed.
    InvalidToken,
    AlreadyRunning,
    NoLibrary,
    LoginCancelled,
    LoginTimeout,
    /// YouTube: the pasted text is not a web link.
    InvalidUrl,
    /// YouTube: a link, but not to a video or playlist.
    UnsupportedUrl,
    /// YouTube: the video cannot be fetched (private, removed, region...).
    Unavailable,
    /// The yt-dlp sidecar is missing or cannot start.
    YtdlpMissing,
    /// Udemy: a lecture is DRM protected (reported, never bypassed). Part of
    /// the UI contract; the phase 2 downloader will raise it.
    #[allow(dead_code)]
    DrmProtected,
    /// Udemy: a diagnostic is already running.
    DiagnoseRunning,
    /// Udemy answered with a bot/captcha challenge instead of data.
    BotChallenge,
    /// Update check: GitHub refused (private repository or bad token).
    UpdateUnauthorized,
    /// Update: a downloaded file does not match its published checksum.
    ChecksumMismatch,
    /// Update: the release publishes no signature for the installer.
    SignatureMissing,
    /// Update: the signature does not verify against the app's public key.
    SignatureInvalid,
    /// Update: the user dismissed the authorization prompt.
    InstallCancelled,
    /// Update: the installer failed or the new version did not take effect.
    InstallFailed,
    /// Update: this installation cannot update itself (the file path is given).
    ManualInstallRequired,
    /// The operation was cancelled by the user.
    Cancelled,
    /// A file could not be written.
    Disk,
    /// Update: the running version is already the newest.
    UpToDate,
    /// GitHub's API rate limit was reached.
    RateLimited,
    /// The repository has no usable release.
    NoReleases,
    /// GitHub's answer could not be understood.
    InvalidResponse,
    Network,
    Internal,
}

/// Serialized to the UI as `{ "code": "...", "message": "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(context: &str, e: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::Internal, format!("{context}: {e}"))
    }

    /// Maps a platform error; `invalid_input` is the code a platform uses for
    /// malformed credentials.
    pub fn from_platform(e: PlatformError, invalid_input: ErrorCode) -> Self {
        let code = match &e {
            PlatformError::SessionExpired => ErrorCode::SessionExpired,
            PlatformError::InvalidInput(_) => invalid_input,
            PlatformError::Network(_) => ErrorCode::Network,
            PlatformError::Parse(_) | PlatformError::Io(_) => ErrorCode::Internal,
        };
        Self::new(code, e.to_string())
    }
}

impl From<PlatformError> for CommandError {
    fn from(e: PlatformError) -> Self {
        Self::from_platform(e, ErrorCode::InvalidCookies)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub logged_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningDownload {
    pub id: String,
    pub platform: String,
    pub group: String,
}

/// Payload of the `download://progress` event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub id: String,
    pub platform: String,
    #[serde(flatten)]
    pub progress: DownloadProgress,
}

/// Payload of the `download://report` event (the report carries `platform`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportEvent {
    pub id: String,
    #[serde(flatten)]
    pub report: DownloadReport,
}

/// Where download events go (the Tauri event bus in the app).
pub trait DownloadEvents: Send + Sync {
    fn progress(&self, event: ProgressEvent);
    fn report(&self, event: ReportEvent);
}

type ActiveMap = Arc<Mutex<HashMap<String, Active>>>;

struct Active {
    group: String,
    cancel: CancelFlag,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Download ids are unique across platforms so the UI can key events by id.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Tags core events with the download id and platform, throttles progress to
/// 1% steps per lesson, and frees the group slot before the final report goes
/// out so the UI can restart a download as soon as it sees the report.
struct IdSink {
    id: String,
    platform: &'static str,
    events: Arc<dyn DownloadEvents>,
    active: ActiveMap,
    last: Mutex<HashMap<String, f32>>,
}

impl IdSink {
    fn release(&self) {
        lock(&self.active).remove(&self.id);
    }

    fn report(&self, report: DownloadReport) {
        self.release();
        self.events.report(ReportEvent {
            id: self.id.clone(),
            report,
        });
    }
}

impl CourseProgressSink for IdSink {
    fn on_progress(&self, p: &DownloadProgress) {
        {
            let mut last = lock(&self.last);
            if let Some(prev) = last.get(&p.lesson) {
                if (p.percent - prev).abs() < 1.0 && p.percent < 100.0 {
                    return;
                }
            }
            last.insert(p.lesson.clone(), p.percent);
        }
        self.events.progress(ProgressEvent {
            id: self.id.clone(),
            platform: self.platform.to_string(),
            progress: p.clone(),
        });
    }

    fn on_report(&self, r: &DownloadReport) {
        self.report(r.clone());
    }
}

fn failure_report(platform: &str, group: &str, error: &PlatformError) -> DownloadReport {
    let mut report = DownloadReport::new(platform, group);
    report.push(Issue::error(
        IssueKind::Course,
        None,
        None,
        format!("download failed: {error}"),
    ));
    report.finish();
    report
}

/// Fails early, with a message naming the folder, when downloads cannot be
/// written to `root` (for example Windows "Controlled folder access" blocking
/// `Videos`), instead of a bare "file not found" deep inside a platform.
pub fn ensure_library_ready(root: &Path) -> Result<(), CommandError> {
    let unusable = |why: &str| {
        CommandError::new(
            ErrorCode::NoLibrary,
            format!(
                "cannot write to the library folder \"{}\" ({why}); choose another folder in Settings",
                root.display()
            ),
        )
    };
    std::fs::create_dir_all(root).map_err(|e| unusable(&e.to_string()))?;
    if crate::library_root::is_writable(root) {
        Ok(())
    } else {
        Err(unusable("it is not writable"))
    }
}

/// Registry of the downloads of one platform: starts them in the background,
/// cancels them and lists the running ones.
pub struct DownloadManager {
    platform: &'static str,
    active: ActiveMap,
}

impl DownloadManager {
    pub fn new(platform: &'static str) -> Self {
        Self {
            platform,
            active: Arc::default(),
        }
    }

    /// Starts a background download of `group` and returns its id. Needs a
    /// Tokio runtime. The same group cannot run twice at once.
    pub fn start(
        &self,
        group: &str,
        platform: Arc<dyn CoursePlatform>,
        library_root: PathBuf,
        events: Arc<dyn DownloadEvents>,
    ) -> Result<String, CommandError> {
        let slug = group.to_string();
        self.start_task(group, events, move |sink, cancel| async move {
            platform
                .download_group(
                    &slug,
                    &library_root,
                    &DownloadOptions::default(),
                    sink.as_ref(),
                    &cancel,
                )
                .await
        })
    }

    /// Starts any download as a background task registered under `group`:
    /// `run` receives the event sink and the cancel flag. Use it when a
    /// platform needs more than a slug (YouTube: link, items, quality). A
    /// returned error becomes a failure report, so the UI always hears back.
    pub fn start_task<F, Fut>(
        &self,
        group: &str,
        events: Arc<dyn DownloadEvents>,
        run: F,
    ) -> Result<String, CommandError>
    where
        F: FnOnce(Arc<dyn CourseProgressSink>, CancelFlag) -> Fut + Send + 'static,
        Fut: Future<Output = Result<DownloadReport, PlatformError>> + Send + 'static,
    {
        let id = format!("dl-{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
        let cancel = CancelFlag::new();
        {
            let mut active = lock(&self.active);
            if active.values().any(|a| a.group == group) {
                return Err(CommandError::new(
                    ErrorCode::AlreadyRunning,
                    format!("\"{group}\" is already downloading"),
                ));
            }
            active.insert(
                id.clone(),
                Active {
                    group: group.to_string(),
                    cancel: cancel.clone(),
                },
            );
        }
        let sink = Arc::new(IdSink {
            id: id.clone(),
            platform: self.platform,
            events,
            active: Arc::clone(&self.active),
            last: Mutex::default(),
        });
        let slug = group.to_string();
        let platform_id = self.platform;
        tokio::spawn(async move {
            let shared: Arc<dyn CourseProgressSink> = sink.clone();
            if let Err(e) = run(shared, cancel).await {
                sink.report(failure_report(platform_id, &slug, &e));
            }
            sink.release();
        });
        Ok(id)
    }

    pub fn cancel(&self, id: &str) -> bool {
        lock(&self.active).get(id).is_some_and(|a| {
            a.cancel.cancel();
            true
        })
    }

    pub fn cancel_all(&self) {
        lock(&self.active).values().for_each(|a| a.cancel.cancel());
    }

    /// Downloads in progress, so a reloaded UI can pick them up again.
    pub fn running(&self) -> Vec<RunningDownload> {
        let mut out: Vec<_> = lock(&self.active)
            .iter()
            .map(|(id, a)| RunningDownload {
                id: id.clone(),
                platform: self.platform.to_string(),
                group: a.group.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writable_library_is_created_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Lumen");
        ensure_library_ready(&root).unwrap();
        assert!(root.is_dir());
    }

    #[test]
    fn an_unusable_library_names_the_folder_and_asks_for_another() {
        let tmp = tempfile::tempdir().unwrap();
        // A file where the folder must go: it can be neither created nor written.
        let blocked = tmp.path().join("Lumen");
        std::fs::write(&blocked, "not a folder").unwrap();
        let err = ensure_library_ready(&blocked).unwrap_err();
        assert_eq!(err.code, ErrorCode::NoLibrary);
        assert!(err.message.contains("Lumen"), "{}", err.message);
        assert!(err.message.contains("another folder"), "{}", err.message);
    }
}
