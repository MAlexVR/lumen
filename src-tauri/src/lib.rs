mod downloads;
mod goldenlife;
mod harbiz;
mod hotmart;
mod library_delete;
mod library_open;
mod library_root;
mod pluralsight;
mod session;
mod skool;
mod state;
mod thumbs;
mod udemy;
mod update;
mod update_install;
mod update_verify;
mod youtube;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use downloads::{
    CommandError as DownloadError, DownloadEvents, ErrorCode, ProgressEvent, ReportEvent,
    RunningDownload, SessionStatus,
};
use goldenlife::{DefaultGoldenlifeFactory, GoldenlifeLoginOutcome, GoldenlifeService};
use harbiz::{DefaultHarbizFactory, HarbizLoginOutcome, HarbizService};
use hotmart::{DefaultHotmartFactory, HotmartLoginOutcome, HotmartService};
use library_delete::{DeleteCode, DeleteError, DeleteOutcome, DeletePreview, DeleteTarget};
use library_open::{OpenCode, OpenError};
use library_root::{LibrarySummary, RootInfo};
use lumen_core::progress::GroupProgress;
use lumen_core::{Library, PlatformGroup};
use lumen_download::{locate_ffmpeg, YtDlp};
use lumen_hotmart::CourseInfo;
use lumen_udemy::diagnose::{DiagnoseOptions, DiagnoseProgress, DiagnoseSink};
use lumen_udemy::parse::CourseSummary as UdemyCourse;
use lumen_youtube::{Request as YoutubeRequest, Resolved, Youtube};
use pluralsight::{
    cookie_header_from_jar, DefaultPluralsightFactory, PluralsightLoginOutcome, PluralsightService,
};
use serde::Serialize;
use skool::{header_from_jar, DefaultFactory, LoginOutcome, SkoolCommandError, SkoolService};
use state::{AppCore, CommandError, RootPolicy};
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use thumbs::{CancelRequest, ThumbInfo, Thumbnails, ThumbsRequest, ThumbsStatus};
use udemy::{DiagnoseOutcome, LoginOutcome as UdemyLoginOutcome, UdemyService};
use update::{TokenStatus, UpdateFailure, UpdateInfo, UpdateService, UpdateSettings};
use update_install::{
    clean_stale, detect_kind, InstallOutcome, KindEnv, Os, ProcessSystem, SystemProbe, UpdateEnv,
};
use youtube::YoutubeService;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

pub fn app_info_value() -> AppInfo {
    AppInfo {
        name: "Lumen",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[tauri::command]
fn app_info() -> AppInfo {
    app_info_value()
}

#[tauri::command]
fn media_base(core: State<'_, Arc<AppCore>>) -> String {
    core.media_base()
}

#[tauri::command]
async fn library_scan(
    root: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<Library, CommandError> {
    let core = Arc::clone(&core);
    tauri::async_runtime::spawn_blocking(move || core.library_scan(&root))
        .await
        .map_err(|e| CommandError::DataDir(std::io::Error::other(e)))?
}

/// Where the library lives when the user chose nothing: the Lumen folder in
/// the user's Videos folder.
fn library_candidates(app: &AppHandle) -> Vec<std::path::PathBuf> {
    app.path()
        .video_dir()
        .map(|videos| vec![videos.join("Lumen")])
        .unwrap_or_default()
}

/// Kept for the current UI: the effective root (saved choice, else detected).
#[tauri::command]
fn library_default_root(core: State<'_, Arc<AppCore>>) -> Option<String> {
    core.root_info().ok().and_then(|i| i.path)
}

#[tauri::command]
fn library_root(core: State<'_, Arc<AppCore>>) -> Result<RootInfo, CommandError> {
    core.root_info()
}

/// Validates, persists and activates `path`; the UI listens for
/// `library://root-changed` to rescan.
#[tauri::command]
fn library_set_root(
    path: String,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<RootInfo, CommandError> {
    let info = core.set_root(&path)?;
    emit_root_changed(&app, &info);
    Ok(info)
}

#[tauri::command]
fn library_reset_root(
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<RootInfo, CommandError> {
    let info = core.reset_root()?;
    emit_root_changed(&app, &info);
    Ok(info)
}

#[derive(Clone, Serialize)]
struct RootChanged {
    path: Option<String>,
}

fn emit_root_changed(app: &AppHandle, info: &RootInfo) {
    let _ = app.emit(
        "library://root-changed",
        RootChanged {
            path: info.path.clone(),
        },
    );
}

/// Native folder chooser, starting at the current root. Never persists: the
/// UI confirms and then calls `library_set_root`. Not unit-testable (needs a
/// real window); everything around it is.
#[tauri::command]
async fn library_pick_root(
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<Option<String>, CommandError> {
    let start = core.picker_start_dir();
    tauri::async_runtime::spawn_blocking(move || pick_folder(&app, start))
        .await
        .map_err(|e| CommandError::DataDir(std::io::Error::other(e)))
}

fn pick_folder(app: &AppHandle, start: Option<PathBuf>) -> Option<String> {
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Carpeta de descargas de Lumen");
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    let picked = dialog.blocking_pick_folder()?.into_path().ok()?;
    Some(library_root::display_path(&picked))
}

/// Which platform folders `path` already holds, so the UI can warn that an
/// empty folder will not show existing courses.
#[tauri::command]
fn library_summary(path: String, core: State<'_, Arc<AppCore>>) -> LibrarySummary {
    core.library_summary(&path)
}

/// Downloads write into the platform's folders and are matched to library
/// folders only by an opaque slug/product id, so any running download of the
/// target's platform blocks its deletion.
fn platform_busy(
    platform: &str,
    skool: &SkoolService,
    hotmart: &HotmartService,
    pluralsight: &PluralsightService,
    youtube: &YoutubeService,
    goldenlife: &GoldenlifeService,
) -> Result<bool, DeleteError> {
    match platform {
        "skool" => Ok(!skool.running().is_empty()),
        "hotmart" => Ok(!hotmart.running().is_empty()),
        "pluralsight" => Ok(!pluralsight.running().is_empty()),
        "goldenlife" => Ok(!goldenlife.running().is_empty()),
        "youtube" => Ok(!youtube.running().is_empty()),
        other => Err(DeleteError::new(
            DeleteCode::InvalidTarget,
            format!("unknown platform \"{other}\""),
        )),
    }
}

fn platform_busy_in(app: &AppHandle, platform: &str) -> Result<bool, DeleteError> {
    if platform == "harbiz" {
        return Ok(!app.state::<Arc<HarbizService>>().running().is_empty());
    }
    platform_busy(
        platform,
        &app.state::<Arc<SkoolService>>(),
        &app.state::<Arc<HotmartService>>(),
        &app.state::<Arc<PluralsightService>>(),
        &app.state::<Arc<YoutubeService>>(),
        &app.state::<Arc<GoldenlifeService>>(),
    )
}

/// What deleting `target` would remove (counts and size); deletes nothing.
#[tauri::command]
async fn library_delete_preview(
    target: DeleteTarget,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<DeletePreview, DeleteError> {
    let busy = platform_busy_in(&app, &target.platform)?;
    let core = Arc::clone(&core);
    tauri::async_runtime::spawn_blocking(move || core.delete_preview(&target, busy))
        .await
        .map_err(|e| DeleteError::internal("preview failed", e))?
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryChanged {
    reason: &'static str,
    platform: String,
    group: String,
    course: Option<String>,
    lesson: Option<String>,
}

/// Deletes a group or course: to the OS trash, or permanently only when the
/// UI passes `permanent: true` after its own extra confirmation. Emits
/// `library://changed` on success.
#[tauri::command]
async fn library_delete(
    target: DeleteTarget,
    permanent: Option<bool>,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<DeleteOutcome, DeleteError> {
    let busy = platform_busy_in(&app, &target.platform)?;
    let core = Arc::clone(&core);
    let t = target.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        core.delete(&t, permanent.unwrap_or(false), busy)
    })
    .await
    .map_err(|e| DeleteError::internal("delete failed", e))??;
    let _ = app.emit(
        "library://changed",
        LibraryChanged {
            reason: "deleted",
            platform: target.platform,
            group: target.group,
            course: target.course,
            lesson: target.lesson,
        },
    );
    Ok(out)
}

#[tauri::command]
async fn lesson_description(
    path: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<String, CommandError> {
    core.read_description(&path)
}

/// Sets and repetitions asked for by each exercise of the workout a video
/// belongs to (empty when the folder holds no Harbiz metadata).
#[tauri::command]
async fn lesson_workout_plan(
    video: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<Vec<lumen_harbiz::ExercisePlan>, CommandError> {
    let path = core.library_path(&video)?;
    let dir = path.parent().ok_or(CommandError::NotFound)?.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || lumen_harbiz::exercise_plan(&dir))
        .await
        .map_err(|e| CommandError::DataDir(std::io::Error::other(e)))
}

/// Opens a library file (video or resource) with the system default app.
#[tauri::command]
async fn open_library_path(
    path: String,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<(), CommandError> {
    let full = core.library_path(&path)?;
    app.opener()
        .open_path(full.to_string_lossy(), None::<&str>)
        .map_err(|e| CommandError::Open(e.to_string()))
}

/// Opens the library root (`rel` empty) or a folder inside it with the system
/// file manager. Folders only: files go through `open_library_path`.
#[tauri::command]
async fn open_library_folder(
    rel: Option<String>,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<(), OpenError> {
    let dir = core.library_dir(rel.as_deref())?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| OpenError::new(OpenCode::OpenFailed, e.to_string()))
}

#[tauri::command]
async fn progress_group(
    group: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<GroupProgress, CommandError> {
    core.progress_group(&group)
}

#[tauri::command]
async fn progress_set_watched(
    group: String,
    key: String,
    watched: bool,
    core: State<'_, Arc<AppCore>>,
) -> Result<(), CommandError> {
    core.progress_set_watched(&group, &key, watched)
}

#[tauri::command]
async fn progress_set_last(
    group: String,
    key: String,
    core: State<'_, Arc<AppCore>>,
) -> Result<(), CommandError> {
    core.progress_set_last(&group, &key)
}

#[tauri::command]
async fn progress_save_position(
    group: String,
    key: String,
    seconds: u32,
    core: State<'_, Arc<AppCore>>,
) -> Result<(), CommandError> {
    core.progress_save_position(&group, &key, seconds)
}

/// Forwards download events to the UI over the Tauri event bus.
struct TauriEvents {
    app: AppHandle,
}

impl DownloadEvents for TauriEvents {
    fn progress(&self, event: ProgressEvent) {
        let _ = self.app.emit("download://progress", event);
    }

    fn report(&self, event: ReportEvent) {
        let _ = self.app.emit("download://report", event);
    }
}

#[tauri::command]
async fn skool_login_cookies(
    text: String,
    skool: State<'_, Arc<SkoolService>>,
) -> Result<LoginOutcome, SkoolCommandError> {
    skool.login_with_cookies(&text).await
}

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const LOGIN_POLL: Duration = Duration::from_secs(1);

/// An in-app login: the window, where it starts, where the cookie jar is read
/// and how the credential is picked out of it.
type LoginValidator = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<bool, SkoolCommandError>> + Send>>
        + Send
        + Sync,
>;

struct WebLogin {
    label: &'static str,
    title: &'static str,
    login_url: &'static str,
    cookie_url: &'static str,
    extract: for<'a> fn(Vec<(&'a str, &'a str)>) -> Option<String>,
}

const SKOOL_LOGIN: WebLogin = WebLogin {
    label: "skool-login",
    title: "Skool",
    login_url: "https://www.skool.com/login",
    cookie_url: "https://www.skool.com/login",
    extract: |pairs| header_from_jar(pairs),
};

const HOTMART_LOGIN: WebLogin = WebLogin {
    label: "hotmart-login",
    title: "Hotmart",
    login_url: "https://consumer.hotmart.com/",
    cookie_url: "https://consumer.hotmart.com/",
    extract: |pairs| hotmart::token_from_jar(pairs),
};

const UDEMY_LOGIN: WebLogin = WebLogin {
    label: "udemy-login",
    title: "Udemy",
    login_url: "https://www.udemy.com/join/login-popup/",
    cookie_url: "https://www.udemy.com/",
    extract: |pairs| udemy::header_from_jar(pairs),
};

const PLURALSIGHT_LOGIN: WebLogin = WebLogin {
    label: "pluralsight-login",
    title: "Pluralsight",
    login_url: "https://app.pluralsight.com/",
    cookie_url: "https://app.pluralsight.com/",
    extract: |pairs| cookie_header_from_jar(pairs),
};

fn login_error(code: ErrorCode, message: &str) -> SkoolCommandError {
    SkoolCommandError::new(code, message)
}

/// Opens the login in a real webview and polls its cookie jar until the
/// credential appears (HttpOnly cookies included), then wipes the webview's
/// own browsing data and closes it. The window has no IPC capability.
async fn validate_login_candidate<F, Fut>(
    candidate: String,
    validate: F,
) -> Result<Option<String>, SkoolCommandError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<bool, SkoolCommandError>>,
{
    if validate(candidate.clone()).await? {
        Ok(Some(candidate))
    } else {
        Ok(None)
    }
}

async fn capture_webview_login(
    app: &AppHandle,
    spec: &WebLogin,
    validator: Option<LoginValidator>,
) -> Result<String, SkoolCommandError> {
    let parse =
        |raw: &str| Url::parse(raw).map_err(|e| login_error(ErrorCode::Internal, &e.to_string()));
    let (start, jar_url) = (parse(spec.login_url)?, parse(spec.cookie_url)?);
    match app.get_webview_window(spec.label) {
        Some(existing) => {
            let _ = existing.set_focus();
        }
        None => {
            WebviewWindowBuilder::new(app, spec.label, WebviewUrl::External(start))
                .title(spec.title)
                .inner_size(520.0, 760.0)
                .build()
                .map_err(|e| login_error(ErrorCode::Internal, &e.to_string()))?;
        }
    }
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        let Some(window) = app.get_webview_window(spec.label) else {
            return Err(login_error(
                ErrorCode::LoginCancelled,
                "the login window was closed",
            ));
        };
        // Reading cookies can deadlock on Windows outside a separate thread.
        let (w, u) = (window.clone(), jar_url.clone());
        let jar = tauri::async_runtime::spawn_blocking(move || w.cookies_for_url(u))
            .await
            .map_err(|e| login_error(ErrorCode::Internal, &e.to_string()))?;
        if let Ok(cookies) = jar {
            let pairs: Vec<(&str, &str)> = cookies.iter().map(|c| (c.name(), c.value())).collect();
            if let Some(credential) = (spec.extract)(pairs) {
                let accepted = match &validator {
                    Some(validate) => {
                        validate_login_candidate(credential, |candidate| validate(candidate))
                            .await?
                    }
                    None => Some(credential),
                };
                if let Some(credential) = accepted {
                    let _ = window.clear_all_browsing_data();
                    let _ = window.close();
                    return Ok(credential);
                }
            }
        }
        if Instant::now() >= deadline {
            if validator.is_none() {
                let _ = window.close();
            }
            return Err(login_error(ErrorCode::LoginTimeout, "login timed out"));
        }
        tokio::time::sleep(LOGIN_POLL).await;
    }
}

#[tauri::command]
async fn skool_login_webview(
    app: AppHandle,
    skool: State<'_, Arc<SkoolService>>,
) -> Result<LoginOutcome, SkoolCommandError> {
    let header = capture_webview_login(&app, &SKOOL_LOGIN, None).await?;
    skool.login_with_cookies(&header).await
}

#[tauri::command]
fn skool_session_status(
    skool: State<'_, Arc<SkoolService>>,
) -> Result<SessionStatus, SkoolCommandError> {
    skool.status()
}

#[tauri::command]
fn skool_logout(skool: State<'_, Arc<SkoolService>>) -> Result<(), SkoolCommandError> {
    skool.logout()
}

#[tauri::command]
async fn skool_groups(
    skool: State<'_, Arc<SkoolService>>,
) -> Result<Vec<PlatformGroup>, SkoolCommandError> {
    skool.groups().await
}

/// Starts a background download of `group` into the current library folder
/// and returns its id. Progress and the final report arrive as events.
#[tauri::command]
async fn skool_download_start(
    group: String,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
    skool: State<'_, Arc<SkoolService>>,
) -> Result<String, SkoolCommandError> {
    skool.start_download(&group, core.library_root(), Arc::new(TauriEvents { app }))
}

#[tauri::command]
fn skool_download_cancel(id: String, skool: State<'_, Arc<SkoolService>>) -> bool {
    skool.cancel(&id)
}

#[tauri::command]
fn skool_downloads_running(skool: State<'_, Arc<SkoolService>>) -> Vec<RunningDownload> {
    skool.running()
}

#[tauri::command]
async fn hotmart_login_credentials(
    token: String,
    cookies: Option<String>,
    hotmart: State<'_, Arc<HotmartService>>,
) -> Result<HotmartLoginOutcome, DownloadError> {
    hotmart
        .login_with_credentials(&token, cookies.as_deref())
        .await
}

/// Opens the Hotmart login in a webview and takes the `hmVlcIntegration`
/// cookie from its jar. Unverified against the live site: the paste path
/// (`hotmart_login_credentials`) is the fallback.
#[tauri::command]
async fn hotmart_login_webview(
    app: AppHandle,
    hotmart: State<'_, Arc<HotmartService>>,
) -> Result<HotmartLoginOutcome, DownloadError> {
    let token = capture_webview_login(&app, &HOTMART_LOGIN, None).await?;
    hotmart.login_with_credentials(&token, None).await
}

#[tauri::command]
fn hotmart_session_status(
    hotmart: State<'_, Arc<HotmartService>>,
) -> Result<SessionStatus, DownloadError> {
    hotmart.status()
}

#[tauri::command]
fn hotmart_logout(hotmart: State<'_, Arc<HotmartService>>) -> Result<(), DownloadError> {
    hotmart.logout()
}

#[tauri::command]
async fn hotmart_courses(
    hotmart: State<'_, Arc<HotmartService>>,
) -> Result<Vec<CourseInfo>, DownloadError> {
    hotmart.courses().await
}

/// Starts a background download of a course (its product id) into the
/// current library folder and returns its id. Progress and the final report
/// arrive as `download://progress` / `download://report` events.
#[tauri::command]
async fn hotmart_download_start(
    course: String,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
    hotmart: State<'_, Arc<HotmartService>>,
) -> Result<String, DownloadError> {
    hotmart.start_download(&course, core.library_root(), Arc::new(TauriEvents { app }))
}

#[tauri::command]
fn hotmart_download_cancel(id: String, hotmart: State<'_, Arc<HotmartService>>) -> bool {
    hotmart.cancel(&id)
}

#[tauri::command]
fn hotmart_downloads_running(hotmart: State<'_, Arc<HotmartService>>) -> Vec<RunningDownload> {
    hotmart.running()
}

/// Preview of a pasted YouTube link (video or playlist); nothing is downloaded.
#[tauri::command]
async fn youtube_resolve(
    url: String,
    youtube: State<'_, Arc<YoutubeService>>,
) -> Result<Resolved, DownloadError> {
    youtube.resolve(&url).await
}

/// Starts a background download of a YouTube video or playlist items into the
/// current library folder and returns its id. Progress and the final report
/// arrive as `download://progress` / `download://report` events.
#[tauri::command]
async fn youtube_download_start(
    request: YoutubeRequest,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
    youtube: State<'_, Arc<YoutubeService>>,
) -> Result<String, DownloadError> {
    youtube.start_download(request, core.library_root(), Arc::new(TauriEvents { app }))
}

#[tauri::command]
fn youtube_download_cancel(id: String, youtube: State<'_, Arc<YoutubeService>>) -> bool {
    youtube.cancel(&id)
}

#[tauri::command]
fn youtube_downloads_running(youtube: State<'_, Arc<YoutubeService>>) -> Vec<RunningDownload> {
    youtube.running()
}

#[tauri::command]
async fn udemy_login_cookies(
    text: String,
    udemy: State<'_, Arc<UdemyService>>,
) -> Result<UdemyLoginOutcome, DownloadError> {
    udemy.login_with_cookies(&text).await
}

/// Opens the Udemy login in a webview and takes the session cookies from its
/// jar. Unverified against the live site (Udemy may show a bot check in an
/// embedded webview): the paste path (`udemy_login_cookies`) is the fallback.
#[tauri::command]
async fn udemy_login_webview(
    app: AppHandle,
    udemy: State<'_, Arc<UdemyService>>,
) -> Result<UdemyLoginOutcome, DownloadError> {
    let header = capture_webview_login(&app, &UDEMY_LOGIN, None).await?;
    udemy.login_with_cookies(&header).await
}

#[tauri::command]
fn udemy_session_status(
    udemy: State<'_, Arc<UdemyService>>,
) -> Result<SessionStatus, DownloadError> {
    udemy.status()
}

#[tauri::command]
fn udemy_logout(udemy: State<'_, Arc<UdemyService>>) -> Result<(), DownloadError> {
    udemy.logout()
}

#[tauri::command]
async fn udemy_courses(
    udemy: State<'_, Arc<UdemyService>>,
) -> Result<Vec<UdemyCourse>, DownloadError> {
    udemy.courses().await
}

/// Forwards diagnostic progress to the UI as `udemy://diagnose-progress`.
struct TauriDiagnose {
    app: AppHandle,
}

impl DiagnoseSink for TauriDiagnose {
    fn progress(&self, progress: &DiagnoseProgress) {
        let _ = self.app.emit("udemy://diagnose-progress", progress);
    }
}

/// Runs the diagnostic (progress arrives as events) and returns where the
/// sanitized report was written plus its summary.
#[tauri::command]
async fn udemy_diagnose(
    options: Option<DiagnoseOptions>,
    app: AppHandle,
    udemy: State<'_, Arc<UdemyService>>,
) -> Result<DiagnoseOutcome, DownloadError> {
    udemy
        .diagnose(options.unwrap_or_default(), &TauriDiagnose { app })
        .await
}

#[tauri::command]
async fn pluralsight_login_credentials(
    text: String,
    core: State<'_, Arc<AppCore>>,
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> Result<PluralsightLoginOutcome, DownloadError> {
    pluralsight
        .login_with_cookies(&text, &core.library_root())
        .await
}

#[tauri::command]
fn udemy_diagnose_cancel(udemy: State<'_, Arc<UdemyService>>) -> bool {
    udemy.cancel_diagnose()
}

#[tauri::command]
fn udemy_diagnostic_last(
    udemy: State<'_, Arc<UdemyService>>,
) -> Result<Option<serde_json::Value>, DownloadError> {
    udemy.last_diagnostic()
}

#[tauri::command]
async fn pluralsight_login_webview(
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> Result<PluralsightLoginOutcome, DownloadError> {
    let service = Arc::clone(&pluralsight);
    let validator: LoginValidator = Arc::new(move |candidate| {
        let service = Arc::clone(&service);
        Box::pin(async move { service.validate_webview_candidate(&candidate).await })
    });
    let cookies = capture_webview_login(&app, &PLURALSIGHT_LOGIN, Some(validator)).await?;
    pluralsight
        .login_with_cookies(&cookies, &core.library_root())
        .await
}

#[tauri::command]
fn pluralsight_session_status(
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> Result<SessionStatus, DownloadError> {
    pluralsight.status()
}

#[tauri::command]
fn pluralsight_logout(
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> Result<(), DownloadError> {
    pluralsight.logout()
}

#[tauri::command]
async fn pluralsight_courses(
    core: State<'_, Arc<AppCore>>,
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> Result<Vec<lumen_pluralsight::platform::PluralsightCourseInfo>, DownloadError> {
    pluralsight.courses(&core.library_root()).await
}

#[tauri::command]
async fn pluralsight_download_start(
    course: String,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> Result<String, DownloadError> {
    pluralsight.start_download(&course, core.library_root(), Arc::new(TauriEvents { app }))
}

#[tauri::command]
fn pluralsight_download_cancel(
    id: String,
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> bool {
    pluralsight.cancel(&id)
}

#[tauri::command]
fn pluralsight_downloads_running(
    pluralsight: State<'_, Arc<PluralsightService>>,
) -> Vec<RunningDownload> {
    pluralsight.running()
}

/// Pauses or resumes a download of any platform: ids are unique across platforms.
fn set_download_paused(app: &AppHandle, id: &str, paused: bool) -> bool {
    app.state::<Arc<SkoolService>>().set_paused(id, paused)
        || app.state::<Arc<HotmartService>>().set_paused(id, paused)
        || app
            .state::<Arc<PluralsightService>>()
            .set_paused(id, paused)
        || app.state::<Arc<YoutubeService>>().set_paused(id, paused)
        || app.state::<Arc<GoldenlifeService>>().set_paused(id, paused)
        || app.state::<Arc<HarbizService>>().set_paused(id, paused)
}

#[tauri::command]
fn download_pause(id: String, app: AppHandle) -> bool {
    set_download_paused(&app, &id, true)
}

#[tauri::command]
fn download_resume(id: String, app: AppHandle) -> bool {
    set_download_paused(&app, &id, false)
}

#[tauri::command]
async fn goldenlife_login_credentials(
    email: String,
    password: String,
    core: State<'_, Arc<AppCore>>,
    goldenlife: State<'_, Arc<GoldenlifeService>>,
) -> Result<GoldenlifeLoginOutcome, DownloadError> {
    goldenlife
        .login_with_credentials(&email, &password, &core.library_root())
        .await
}

#[tauri::command]
fn goldenlife_session_status(
    goldenlife: State<'_, Arc<GoldenlifeService>>,
) -> Result<SessionStatus, DownloadError> {
    goldenlife.status()
}

#[tauri::command]
fn goldenlife_logout(goldenlife: State<'_, Arc<GoldenlifeService>>) -> Result<(), DownloadError> {
    goldenlife.logout()
}

#[tauri::command]
async fn goldenlife_courses(
    core: State<'_, Arc<AppCore>>,
    goldenlife: State<'_, Arc<GoldenlifeService>>,
) -> Result<Vec<lumen_goldenlife::GoldenlifeCourseInfo>, DownloadError> {
    goldenlife.courses(&core.library_root()).await
}

#[tauri::command]
async fn goldenlife_download_start(
    course: String,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
    goldenlife: State<'_, Arc<GoldenlifeService>>,
) -> Result<String, DownloadError> {
    goldenlife.start_download(&course, core.library_root(), Arc::new(TauriEvents { app }))
}

#[tauri::command]
fn goldenlife_download_cancel(id: String, goldenlife: State<'_, Arc<GoldenlifeService>>) -> bool {
    goldenlife.cancel(&id)
}

#[tauri::command]
fn goldenlife_downloads_running(
    goldenlife: State<'_, Arc<GoldenlifeService>>,
) -> Vec<RunningDownload> {
    goldenlife.running()
}

#[tauri::command]
async fn harbiz_login_credentials(
    email: String,
    password: String,
    core: State<'_, Arc<AppCore>>,
    harbiz: State<'_, Arc<HarbizService>>,
) -> Result<HarbizLoginOutcome, DownloadError> {
    harbiz
        .login_with_credentials(&email, &password, &core.library_root())
        .await
}

#[tauri::command]
fn harbiz_session_status(
    harbiz: State<'_, Arc<HarbizService>>,
) -> Result<SessionStatus, DownloadError> {
    harbiz.status()
}

#[tauri::command]
fn harbiz_logout(harbiz: State<'_, Arc<HarbizService>>) -> Result<(), DownloadError> {
    harbiz.logout()
}

#[tauri::command]
async fn harbiz_courses(
    core: State<'_, Arc<AppCore>>,
    harbiz: State<'_, Arc<HarbizService>>,
) -> Result<Vec<lumen_harbiz::HarbizCourseInfo>, DownloadError> {
    harbiz.courses(&core.library_root()).await
}

#[tauri::command]
async fn harbiz_download_start(
    course: String,
    app: AppHandle,
    core: State<'_, Arc<AppCore>>,
    harbiz: State<'_, Arc<HarbizService>>,
) -> Result<String, DownloadError> {
    harbiz.start_download(&course, core.library_root(), Arc::new(TauriEvents { app }))
}

#[tauri::command]
fn harbiz_download_cancel(id: String, harbiz: State<'_, Arc<HarbizService>>) -> bool {
    harbiz.cancel(&id)
}

#[tauri::command]
fn harbiz_downloads_running(harbiz: State<'_, Arc<HarbizService>>) -> Vec<RunningDownload> {
    harbiz.running()
}

/// Cached video thumbnails: answers at once with ready/pending/unavailable;
/// finished items arrive as `thumbnails://ready` events.
#[tauri::command]
async fn thumbnails_request(
    request: ThumbsRequest,
    thumbs: State<'_, Arc<Thumbnails>>,
) -> Result<Vec<ThumbInfo>, CommandError> {
    Ok(thumbs.request(request).await)
}

/// Drops queued (not running) thumbnail items.
#[tauri::command]
fn thumbnails_cancel(request: CancelRequest, thumbs: State<'_, Arc<Thumbnails>>) {
    thumbs.cancel(request);
}

#[tauri::command]
async fn thumbnails_status(
    thumbs: State<'_, Arc<Thumbnails>>,
) -> Result<ThumbsStatus, CommandError> {
    Ok(thumbs.status().await)
}

/// Asks GitHub for the newest release (nothing is downloaded or installed).
#[tauri::command]
async fn update_check(update: State<'_, Arc<UpdateService>>) -> Result<UpdateInfo, DownloadError> {
    update.check().await
}

/// Downloads, verifies, installs and restarts into the newest release. Progress
/// arrives as `update://progress`; a successful run ends the app.
#[tauri::command]
async fn update_install(
    update: State<'_, Arc<UpdateService>>,
) -> Result<InstallOutcome, UpdateFailure> {
    update.install().await
}

#[tauri::command]
fn update_cancel(update: State<'_, Arc<UpdateService>>) -> bool {
    update.cancel()
}

#[tauri::command]
fn update_settings(update: State<'_, Arc<UpdateService>>) -> Result<UpdateSettings, DownloadError> {
    update.settings()
}

#[tauri::command]
fn update_set_include_prereleases(
    value: bool,
    update: State<'_, Arc<UpdateService>>,
) -> Result<(), DownloadError> {
    update.set_include_prereleases(value)
}

#[tauri::command]
fn update_token_status(
    update: State<'_, Arc<UpdateService>>,
) -> Result<TokenStatus, DownloadError> {
    update.token_status()
}

#[tauri::command]
fn update_token_save(
    token: String,
    update: State<'_, Arc<UpdateService>>,
) -> Result<(), DownloadError> {
    update.save_token(&token)
}

#[tauri::command]
fn update_token_clear(update: State<'_, Arc<UpdateService>>) -> Result<(), DownloadError> {
    update.clear_token()
}

/// Directory next to the executable, where the bundled yt-dlp sidecar lives.
fn sidecar_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}

pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let policy =
                RootPolicy::new(library_candidates(app.handle()), app.path().home_dir().ok());
            let core = tauri::async_runtime::block_on(AppCore::with_policy(
                &data_dir.join("progress.db"),
                policy,
            ))?;
            let core = Arc::new(core);
            app.manage(Arc::clone(&core));
            // Notes of Harbiz classes downloaded by an older version are rewritten
            // from their saved metadata before the UI scans the library (no network).
            let _ = lumen_harbiz::refresh_notes(&core.library_root());
            app.manage(Arc::new(UdemyService::new(
                session::SessionStore::for_platform(&data_dir, "udemy"),
                data_dir.clone(),
                env!("CARGO_PKG_VERSION"),
            )));
            let ytdlp = YtDlp::locate(sidecar_dir().as_deref());
            let ffmpeg = locate_ffmpeg(
                sidecar_dir().as_deref(),
                std::env::var_os("PATH").as_deref(),
            );
            let updates_dir = app.path().app_cache_dir()?.join("updates");
            clean_stale(&updates_dir);
            let exe = std::env::current_exe().unwrap_or_default();
            let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
            let kind = detect_kind(&KindEnv {
                os: Os::current(),
                exe: &exe,
                appimage: appimage.as_deref(),
                probe: &SystemProbe,
            });
            let (exit_handle, emit_handle) = (app.handle().clone(), app.handle().clone());
            app.manage(Arc::new(UpdateService::new(
                Arc::clone(&core),
                data_dir.clone(),
                env!("CARGO_PKG_VERSION"),
                UpdateEnv {
                    kind,
                    arch: std::env::consts::ARCH.to_string(),
                    appimage,
                    updates_dir,
                    system: Arc::new(ProcessSystem::new(Arc::new(move || exit_handle.exit(0)))),
                    emit: Arc::new(move |event, payload| {
                        let _ = emit_handle.emit(event, payload);
                    }),
                },
            )));
            let cache_dir = app.path().app_cache_dir()?.join("thumbs");
            let handle = app.handle().clone();
            app.manage(Arc::new(Thumbnails::new(
                core,
                cache_dir,
                ffmpeg.clone(),
                Arc::new(move |event, payload| {
                    let _ = handle.emit(event, payload);
                }),
            )));
            app.manage(Arc::new(YoutubeService::new(Arc::new(Youtube::new(
                Arc::new(ytdlp.clone()),
                ffmpeg,
            )))));
            app.manage(Arc::new(SkoolService::new(
                session::SessionStore::for_platform(&data_dir, "skool"),
                Arc::new(DefaultFactory::new(ytdlp.clone())),
            )));
            app.manage(Arc::new(HotmartService::new(
                session::SessionStore::for_platform(&data_dir, "hotmart"),
                Arc::new(DefaultHotmartFactory::new(ytdlp.clone())),
            )));
            app.manage(Arc::new(GoldenlifeService::new(
                session::SessionStore::for_platform(&data_dir, "goldenlife"),
                Arc::new(DefaultGoldenlifeFactory::new(ytdlp.clone())),
            )));
            app.manage(Arc::new(HarbizService::new(
                session::SessionStore::for_platform(&data_dir, "harbiz"),
                Arc::new(DefaultHarbizFactory::new(ytdlp.clone())),
            )));
            app.manage(Arc::new(PluralsightService::new(
                session::SessionStore::for_platform(&data_dir, "pluralsight"),
                Arc::new(DefaultPluralsightFactory::new(ytdlp)),
            )));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            media_base,
            library_scan,
            library_default_root,
            library_root,
            library_set_root,
            library_reset_root,
            library_pick_root,
            library_summary,
            library_delete_preview,
            library_delete,
            lesson_description,
            lesson_workout_plan,
            open_library_path,
            open_library_folder,
            progress_group,
            progress_set_watched,
            progress_set_last,
            progress_save_position,
            skool_login_cookies,
            skool_login_webview,
            skool_session_status,
            skool_logout,
            skool_groups,
            skool_download_start,
            skool_download_cancel,
            skool_downloads_running,
            hotmart_login_credentials,
            hotmart_login_webview,
            hotmart_session_status,
            hotmart_logout,
            hotmart_courses,
            hotmart_download_start,
            hotmart_download_cancel,
            hotmart_downloads_running,
            youtube_resolve,
            youtube_download_start,
            youtube_download_cancel,
            youtube_downloads_running,
            udemy_login_cookies,
            udemy_login_webview,
            udemy_session_status,
            udemy_logout,
            udemy_courses,
            udemy_diagnose,
            udemy_diagnose_cancel,
            udemy_diagnostic_last,
            pluralsight_login_credentials,
            pluralsight_login_webview,
            pluralsight_session_status,
            pluralsight_logout,
            pluralsight_courses,
            pluralsight_download_start,
            pluralsight_download_cancel,
            pluralsight_downloads_running,
            download_pause,
            download_resume,
            goldenlife_login_credentials,
            goldenlife_session_status,
            goldenlife_logout,
            goldenlife_courses,
            goldenlife_download_start,
            goldenlife_download_cancel,
            goldenlife_downloads_running,
            harbiz_login_credentials,
            harbiz_session_status,
            harbiz_logout,
            harbiz_courses,
            harbiz_download_start,
            harbiz_download_cancel,
            harbiz_downloads_running,
            thumbnails_request,
            thumbnails_cancel,
            thumbnails_status,
            update_check,
            update_install,
            update_cancel,
            update_settings,
            update_set_include_prereleases,
            update_token_status,
            update_token_save,
            update_token_clear
        ])
        .run(tauri::generate_context!());
    if let Err(err) = result {
        eprintln!("lumen failed to start: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DownloadManager::start` spawns on Tokio, so a synchronous command (which
    /// Tauri runs on the main thread, outside any runtime) freezes the app.
    #[test]
    fn download_start_commands_are_async() {
        let source = include_str!("lib.rs");
        let starts: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|l| {
                (l.starts_with("fn ") || l.starts_with("async fn "))
                    && l.contains("_download_start(")
            })
            .collect();
        assert!(starts.len() >= 5, "{starts:?}");
        for line in starts {
            assert!(line.starts_with("async fn "), "{line}");
        }
    }

    #[test]
    fn app_info_reports_name_and_version() {
        let info = app_info_value();
        assert_eq!(info.name, "Lumen");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn webview_cookie_candidates_are_accepted_only_after_session_validation() {
        let mut observed = Vec::new();
        let stale =
            validate_login_candidate("analytics=synthetic".to_string(), |candidate| async {
                observed.push(candidate);
                Ok(false)
            })
            .await
            .unwrap();
        assert_eq!(stale, None);

        let valid = validate_login_candidate("session=synthetic".to_string(), |candidate| async {
            observed.push(candidate);
            Ok(true)
        })
        .await
        .unwrap();
        assert_eq!(valid.as_deref(), Some("session=synthetic"));
        assert_eq!(observed, ["analytics=synthetic", "session=synthetic"]);
    }

    #[tokio::test]
    async fn webview_transient_validation_error_is_returned_without_accepting_candidate() {
        let result =
            validate_login_candidate("session=synthetic-secret".to_string(), |_candidate| async {
                Err(login_error(ErrorCode::Network, "temporary service error"))
            })
            .await;
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::Network);
        assert!(!error.message.contains("synthetic-secret"));
    }

    #[test]
    fn pluralsight_webview_login_is_scoped_to_app_origin() {
        assert_eq!(PLURALSIGHT_LOGIN.cookie_url, "https://app.pluralsight.com/");
        assert_eq!(PLURALSIGHT_LOGIN.login_url, "https://app.pluralsight.com/");
        let header = cookie_header_from_jar(vec![("session", "synthetic-cookie")]).unwrap();
        assert_eq!(header, "session=synthetic-cookie");
    }
}
