//! Application core behind the Tauri commands. It holds no Tauri types so it
//! can be tested without a window; commands only delegate to it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use lumen_core::progress::{GroupProgress, ProgressError, ProgressStore};
use lumen_core::scanner::split_order_prefix;
use lumen_core::{Library, LibraryScanner, ScanError};
use lumen_media::{resolve_confined, MediaError, MediaServer};
use serde::{Serialize, Serializer};

use crate::library_delete::{
    dir_size, is_disposable_group, resolve_lesson, resolve_target, DeleteCode, DeleteError,
    DeleteOutcome, DeletePreview, DeleteTarget, LessonTarget, Resolved, ResolvedLesson,
    SystemTrash, TargetKind, Trasher, SIZE_BUDGET, SIZE_MAX_ENTRIES,
};
use crate::library_open::{resolve_dir, OpenError};
use crate::library_root::{
    summarize, LibrarySummary, RootError, RootInfo, RootSource, RootValidator,
};

/// Settings key under which the chosen library folder is stored.
pub const LIBRARY_ROOT_KEY: &str = "libraryRoot";

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Scan(#[from] ScanError),
    #[error(transparent)]
    Progress(#[from] ProgressError),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error("cannot prepare data directory: {0}")]
    DataDir(#[from] std::io::Error),
    #[error(transparent)]
    Root(#[from] RootError),
    #[error("progress store is unavailable")]
    StorePoisoned,
    #[error("file not found in the library")]
    NotFound,
    #[error("file is too large to display")]
    TooLarge,
    #[error("cannot open with the system application: {0}")]
    Open(String),
}

/// Largest lesson description the UI will render inline.
pub const MAX_DESCRIPTION_BYTES: usize = 2 * 1024 * 1024;

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Where the library may live when the user chose nothing: known locations,
/// most specific first, and the home folder used to validate a choice.
pub struct RootPolicy {
    candidates: Vec<PathBuf>,
    home: Option<PathBuf>,
}

impl RootPolicy {
    pub fn new(candidates: Vec<PathBuf>, home: Option<PathBuf>) -> Self {
        Self { candidates, home }
    }

    /// Detection without a saved choice: an existing library, else the first
    /// known location.
    fn detect(&self) -> (Option<PathBuf>, RootSource) {
        let found = self
            .candidates
            .iter()
            .find(|c| c.join("Skool").is_dir() || c.join("Hotmart").is_dir());
        match (found, self.candidates.first()) {
            (Some(p), _) => (Some(p.clone()), RootSource::Detected),
            (None, first) => (first.cloned(), RootSource::Default),
        }
    }

    /// Saved choice when it still points at a folder, else detection.
    fn resolve(
        &self,
        store: &ProgressStore,
    ) -> Result<(Option<PathBuf>, RootSource), CommandError> {
        if let Some(saved) = store.setting(LIBRARY_ROOT_KEY)? {
            let saved = PathBuf::from(saved);
            if saved.is_dir() {
                return Ok((Some(saved), RootSource::Saved));
            }
        }
        Ok(self.detect())
    }
}

/// Plain text as escaped HTML paragraphs (blank line = new paragraph).
fn plain_text_to_html(text: &str) -> String {
    let escape = |line: &str| {
        line.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    text.replace("\r\n", "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            format!(
                "<p>{}</p>",
                p.lines().map(escape).collect::<Vec<_>>().join("<br>")
            )
        })
        .collect()
}

pub struct AppCore {
    media: MediaServer,
    progress: Mutex<ProgressStore>,
    root: RwLock<PathBuf>,
    policy: RootPolicy,
    trasher: Mutex<Arc<dyn Trasher>>,
}

impl AppCore {
    /// Opens the progress database (creating its folder) and starts the media
    /// server rooted at `initial_root`.
    #[cfg(test)]
    pub async fn new(db_path: &Path, initial_root: PathBuf) -> Result<Self, CommandError> {
        Self::with_policy(db_path, RootPolicy::new(vec![initial_root], None)).await
    }

    /// Like [`AppCore::new`], but the starting root is the saved choice when
    /// usable, else what the policy detects (empty when nothing is known).
    pub async fn with_policy(db_path: &Path, policy: RootPolicy) -> Result<Self, CommandError> {
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let progress = ProgressStore::open(db_path)?;
        let initial = policy.resolve(&progress)?.0.unwrap_or_default();
        let media = MediaServer::start(initial.clone()).await?;
        Ok(Self {
            media,
            progress: Mutex::new(progress),
            root: RwLock::new(initial),
            policy,
            trasher: Mutex::new(Arc::new(SystemTrash)),
        })
    }

    /// Replaces the trash adapter (tests only: the real one is the OS trash).
    #[cfg(test)]
    pub fn set_trasher(&self, trasher: Arc<dyn Trasher>) {
        *self.trasher.lock().unwrap_or_else(|e| e.into_inner()) = trasher;
    }

    /// Lessons and lessons with a video inside the target, from the scanner.
    fn lesson_counts(&self, root: &Path, r: &Resolved) -> Result<(String, u64, u64), DeleteError> {
        let group = LibraryScanner::new(root)
            .scan_platform_group(r.platform, &r.group_folder)
            .map_err(|e| DeleteError::internal("cannot read the folder", e))?;
        let (name, courses) = match &r.course_folder {
            None => (group.title.clone(), group.courses),
            Some(folder) => {
                let (title, index) = split_order_prefix(folder);
                let courses: Vec<_> = group
                    .courses
                    .into_iter()
                    .filter(|c| c.title == title && c.index == index)
                    .collect();
                (title.to_string(), courses)
            }
        };
        let lessons: Vec<_> = courses
            .iter()
            .flat_map(|c| &c.sections)
            .flat_map(|s| &s.lessons)
            .collect();
        let with_video = lessons.iter().filter(|l| l.has_video).count() as u64;
        Ok((name, lessons.len() as u64, with_video))
    }

    /// What deleting `target` would remove, without touching anything.
    /// `busy` says a download of that platform is running.
    pub fn delete_preview(
        &self,
        target: &DeleteTarget,
        busy: bool,
    ) -> Result<DeletePreview, DeleteError> {
        let root = self.library_root();
        if target.lesson.is_some() {
            return self.lesson_preview(&root, target, busy);
        }
        let r = resolve_target(&root, target)?;
        let (name, lessons, lessons_with_video) = self.lesson_counts(&root, &r)?;
        let size = dir_size(&r.path, SIZE_MAX_ENTRIES, SIZE_BUDGET);
        Ok(DeletePreview {
            name,
            platform: target.platform.clone(),
            kind: r.kind,
            lessons,
            lessons_with_video,
            bytes: size.bytes,
            bytes_approx: size.approx,
            path: r.rel,
            running_download: busy,
        })
    }

    /// Deletes the target: to the OS trash, or for good when `permanent`.
    /// A failing trash deletes nothing. Stored progress of the removed
    /// lessons is cleared afterwards.
    pub fn delete(
        &self,
        target: &DeleteTarget,
        permanent: bool,
        busy: bool,
    ) -> Result<DeleteOutcome, DeleteError> {
        let root = self.library_root();
        if target.lesson.is_some() {
            return self.delete_lesson(&root, target, permanent, busy);
        }
        let r = resolve_target(&root, target)?;
        if busy {
            return Err(DeleteError::new(
                DeleteCode::DownloadRunning,
                "a download is in progress; cancel it first",
            ));
        }
        let size = dir_size(&r.path, SIZE_MAX_ENTRIES, SIZE_BUDGET);
        if permanent {
            std::fs::remove_dir_all(&r.path)
                .map_err(|e| DeleteError::internal("cannot delete the folder", e))?;
        } else {
            let trasher = self
                .trasher
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            trasher
                .trash(&r.path)
                .map_err(|e| DeleteError::new(DeleteCode::TrashFailed, e))?;
        }
        self.forget_progress(&r)?;
        Ok(DeleteOutcome {
            deleted: true,
            moved_to_trash: !permanent,
            bytes_freed: size.bytes,
            bytes_approx: size.approx,
            group_removed: r.kind == TargetKind::Group,
        })
    }

    fn lesson_bytes(target: &LessonTarget) -> (u64, bool) {
        match target {
            LessonTarget::Dir(p) => {
                let s = dir_size(p, SIZE_MAX_ENTRIES, SIZE_BUDGET);
                (s.bytes, s.approx)
            }
            LessonTarget::Files(paths) => (
                paths
                    .iter()
                    .filter_map(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum(),
                false,
            ),
        }
    }

    fn lesson_preview(
        &self,
        root: &Path,
        target: &DeleteTarget,
        busy: bool,
    ) -> Result<DeletePreview, DeleteError> {
        let r = resolve_lesson(root, target)?;
        let (bytes, approx) = Self::lesson_bytes(&r.target);
        Ok(DeletePreview {
            name: r.title,
            platform: target.platform.clone(),
            kind: TargetKind::Lesson,
            lessons: 1,
            lessons_with_video: u64::from(r.has_video),
            bytes,
            bytes_approx: approx,
            path: r.rel,
            running_download: busy,
        })
    }

    /// Moves `path` to the trash, or deletes it for good when `permanent`.
    fn dispose(&self, path: &Path, permanent: bool) -> Result<(), DeleteError> {
        if permanent {
            let result = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
            return result.map_err(|e| DeleteError::internal("cannot delete", e));
        }
        let trasher = self
            .trasher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        trasher
            .trash(path)
            .map_err(|e| DeleteError::new(DeleteCode::TrashFailed, e))
    }

    /// Deletes one lesson (its folder, or its loose files), clears its
    /// progress, and takes the group folder along when nothing else is left.
    fn delete_lesson(
        &self,
        root: &Path,
        target: &DeleteTarget,
        permanent: bool,
        busy: bool,
    ) -> Result<DeleteOutcome, DeleteError> {
        let r = resolve_lesson(root, target)?;
        if busy {
            return Err(DeleteError::new(
                DeleteCode::DownloadRunning,
                "a download is in progress; cancel it first",
            ));
        }
        let (bytes, approx) = Self::lesson_bytes(&r.target);
        match &r.target {
            LessonTarget::Dir(p) => self.dispose(p, permanent)?,
            LessonTarget::Files(paths) => {
                for p in paths {
                    self.dispose(p, permanent)?;
                }
            }
        }
        let cleanup = |e: &dyn std::fmt::Display| {
            DeleteError::internal("deleted, but progress cleanup failed", e)
        };
        self.store()
            .map_err(|e| cleanup(&e))?
            .forget_lesson(&r.group_id, &r.key)
            .map_err(|e| cleanup(&e))?;
        let group_removed = self.remove_empty_group(root, &r, permanent);
        Ok(DeleteOutcome {
            deleted: true,
            moved_to_trash: !permanent,
            bytes_freed: bytes,
            bytes_approx: approx,
            group_removed,
        })
    }

    /// After the last lesson left, the leftover housekeeping folder goes too,
    /// by the same rules. A failure only leaves the folder in place.
    fn remove_empty_group(&self, root: &Path, r: &ResolvedLesson, permanent: bool) -> bool {
        let Some(folder) = r
            .group_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
        else {
            return false;
        };
        let empty = LibraryScanner::new(root)
            .scan_platform_group(r.platform, &folder)
            .is_ok_and(|g| g.lesson_count == 0);
        if !empty
            || !is_disposable_group(&r.group_dir)
            || self.dispose(&r.group_dir, permanent).is_err()
        {
            return false;
        }
        if let Ok(store) = self.store() {
            let _ = store.forget_group(&r.group_id);
        }
        true
    }

    fn forget_progress(&self, r: &Resolved) -> Result<(), DeleteError> {
        let store = self
            .store()
            .map_err(|e| DeleteError::internal("deleted, but progress cleanup failed", e))?;
        let done = match &r.course_folder {
            None => store.forget_group(&r.group_id),
            Some(folder) => {
                let (title, index) = split_order_prefix(folder);
                store.forget_course(&r.group_id, &format!("{index}-{title}/"))
            }
        };
        done.map_err(|e| DeleteError::internal("deleted, but progress cleanup failed", e))
    }

    fn apply_root(&self, root: &Path) {
        self.media.set_root(root);
        *self.root.write().unwrap_or_else(|e| e.into_inner()) = root.to_path_buf();
    }

    fn info_for(resolved: (Option<PathBuf>, RootSource)) -> RootInfo {
        RootInfo::describe(resolved.0.as_deref(), resolved.1)
    }

    /// Effective library folder with where it came from and its health.
    pub fn root_info(&self) -> Result<RootInfo, CommandError> {
        Ok(Self::info_for(self.policy.resolve(&*self.store()?)?))
    }

    /// Validates `path` (creating it when missing), persists it and switches
    /// scan, media server, confinement and new downloads to it. Downloads
    /// already running keep the root they received when they started, and
    /// existing data is never moved.
    pub fn set_root(&self, path: &str) -> Result<RootInfo, CommandError> {
        let root = RootValidator::new(self.policy.home.clone()).validate(path)?;
        self.store()?
            .set_setting(LIBRARY_ROOT_KEY, &root.to_string_lossy())?;
        self.apply_root(&root);
        Ok(Self::info_for((Some(root), RootSource::Saved)))
    }

    /// Forgets the saved choice and goes back to detection.
    pub fn reset_root(&self) -> Result<RootInfo, CommandError> {
        let resolved = {
            let store = self.store()?;
            store.remove_setting(LIBRARY_ROOT_KEY)?;
            self.policy.resolve(&store)?
        };
        if let Some(root) = &resolved.0 {
            self.apply_root(root);
        }
        Ok(Self::info_for(resolved))
    }

    /// Which platform folders `path` already holds (names only, no scan).
    /// A relative or unusable path reports no library.
    pub fn library_summary(&self, path: &str) -> LibrarySummary {
        match RootValidator::new(self.policy.home.clone()).resolve_input(path) {
            Some(p) => summarize(&p),
            None => summarize(Path::new("")),
        }
    }

    /// Directory to show in the file manager: the current root or a folder
    /// inside it (see [`resolve_dir`]).
    pub fn library_dir(&self, rel: Option<&str>) -> Result<PathBuf, OpenError> {
        resolve_dir(&self.library_root(), rel)
    }

    /// Folder to start the native chooser in.
    pub fn picker_start_dir(&self) -> Option<PathBuf> {
        self.root_info()
            .ok()
            .and_then(|i| i.path)
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| self.policy.home.clone())
    }

    pub fn media_base(&self) -> String {
        self.media.base_url()
    }

    /// Reads an application setting (small key/value preference).
    pub fn setting(&self, key: &str) -> Result<Option<String>, CommandError> {
        Ok(self.store()?.setting(key)?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), CommandError> {
        Ok(self.store()?.set_setting(key, value)?)
    }

    /// Serves thumbnails from `dir` through the media server's `/t/` route.
    pub fn set_thumb_dir(&self, dir: &Path) {
        self.media.set_thumb_dir(dir);
    }

    /// URL prefix of the thumbnail route (append `/<hash>.jpg`).
    pub fn thumb_base(&self) -> String {
        self.media.thumb_base_url()
    }

    /// Scans `root` and points the media server at it so lesson paths resolve.
    pub fn library_scan(&self, root: &str) -> Result<Library, CommandError> {
        self.media.set_root(root);
        *self.root.write().unwrap_or_else(|e| e.into_inner()) = PathBuf::from(root);
        Ok(LibraryScanner::new(root).scan()?)
    }

    /// Library folder currently in use (empty when none was chosen yet).
    pub fn library_root(&self) -> PathBuf {
        self.root.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn store(&self) -> Result<std::sync::MutexGuard<'_, ProgressStore>, CommandError> {
        self.progress
            .lock()
            .map_err(|_| CommandError::StorePoisoned)
    }

    pub fn progress_group(&self, group: &str) -> Result<GroupProgress, CommandError> {
        Ok(self.store()?.group_progress(group)?)
    }

    pub fn progress_set_watched(
        &self,
        group: &str,
        key: &str,
        watched: bool,
    ) -> Result<(), CommandError> {
        Ok(self.store()?.set_watched(group, key, watched)?)
    }

    pub fn progress_set_last(&self, group: &str, key: &str) -> Result<(), CommandError> {
        Ok(self.store()?.set_last_lesson(group, key)?)
    }

    pub fn progress_save_position(
        &self,
        group: &str,
        key: &str,
        seconds: u32,
    ) -> Result<(), CommandError> {
        Ok(self.store()?.save_position(group, key, seconds)?)
    }

    /// Reads a lesson description (`.html`) inside the scanned library so the
    /// UI can render it in a sandboxed frame with its own styles.
    pub fn read_description(&self, rel: &str) -> Result<String, CommandError> {
        let name = Path::new(rel).file_name().map(|n| n.to_string_lossy());
        let ext = Path::new(rel).extension().map(|e| e.to_ascii_lowercase());
        let is_txt = ext.as_deref() == Some("txt".as_ref())
            && name.is_some_and(|n| n.eq_ignore_ascii_case("description.txt"));
        if ext.as_deref() != Some("html".as_ref()) && !is_txt {
            return Err(CommandError::NotFound);
        }
        let path = self.library_path(rel)?;
        let len = std::fs::metadata(&path)
            .map_err(|_| CommandError::NotFound)?
            .len();
        if len > MAX_DESCRIPTION_BYTES as u64 {
            return Err(CommandError::TooLarge);
        }
        let bytes = std::fs::read(&path).map_err(|_| CommandError::NotFound)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(if is_txt {
            plain_text_to_html(&text)
        } else {
            text
        })
    }

    /// Absolute path of a library-relative file, refusing anything that
    /// resolves outside the scanned root.
    pub fn library_path(&self, rel: &str) -> Result<PathBuf, CommandError> {
        let root = self.root.read().unwrap_or_else(|e| e.into_inner()).clone();
        resolve_confined(&root, rel).ok_or(CommandError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    async fn core(dir: &Path) -> AppCore {
        AppCore::new(&dir.join("data/progress.db"), dir.to_path_buf())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn scan_returns_library_and_media_base_is_loopback() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(lib.join("Skool/G/01 - C/01 - S/01 - L")).unwrap();
        fs::write(lib.join("Skool/G/01 - C/01 - S/01 - L/v.mp4"), "v").unwrap();
        let core = core(tmp.path()).await;
        assert!(core.media_base().starts_with("http://127.0.0.1:"));
        let library = core.library_scan(lib.to_str().unwrap()).unwrap();
        assert_eq!(library.groups[0].name, "G");
        assert!(library.groups[0].courses[0].sections[0].lessons[0].has_video);
    }

    #[tokio::test]
    async fn library_root_follows_the_last_scanned_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let core = core(tmp.path()).await;
        assert_eq!(core.library_root(), tmp.path());
        let lib = tmp.path().join("lib");
        core.library_scan(lib.to_str().unwrap()).unwrap();
        assert_eq!(core.library_root(), lib);
    }

    #[tokio::test]
    async fn scan_of_missing_root_is_empty_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let core = core(tmp.path()).await;
        let missing = tmp.path().join("nope");
        assert!(core
            .library_scan(missing.to_str().unwrap())
            .unwrap()
            .groups
            .is_empty());
    }

    #[tokio::test]
    async fn progress_commands_round_trip_and_persist_across_restart() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let core = core(tmp.path()).await;
            core.progress_set_watched("G", "k1", true).unwrap();
            core.progress_set_last("G", "k1").unwrap();
            core.progress_save_position("G", "k1", 42).unwrap();
            let p = core.progress_group("G").unwrap();
            assert_eq!(p.watched, vec!["k1"]);
            assert_eq!(p.last_lesson.as_deref(), Some("k1"));
            assert_eq!(p.positions[0].seconds, 42);
        }
        let core = core(tmp.path()).await;
        assert_eq!(core.progress_group("G").unwrap().watched, vec!["k1"]);
    }

    #[test]
    fn errors_serialize_as_plain_strings_for_the_ui() {
        let e = CommandError::from(ScanError::InvalidGroup);
        assert_eq!(serde_json::to_string(&e).unwrap(), "\"invalid group name\"");
    }

    #[tokio::test]
    async fn description_is_read_from_the_scanned_root() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(lib.join("Skool/G")).unwrap();
        fs::write(lib.join("Skool/G/description.html"), "<p>hola</p>").unwrap();
        let core = core(tmp.path()).await;
        core.library_scan(lib.to_str().unwrap()).unwrap();
        assert_eq!(
            core.read_description("Skool/G/description.html").unwrap(),
            "<p>hola</p>"
        );
    }

    #[tokio::test]
    async fn hotmart_description_and_materials_resolve_from_the_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        let lesson = lib.join("Hotmart/Águila - Ana/1. Módulo/1. Clase");
        fs::create_dir_all(lesson.join("Materials")).unwrap();
        fs::write(lesson.join("Description.html"), "<p>hola</p>").unwrap();
        fs::write(lesson.join("Materials/drm_Plan.pdf"), "p").unwrap();
        let core = core(tmp.path()).await;
        let library = core.library_scan(lib.to_str().unwrap()).unwrap();
        let hotmart = &library.platforms[0].groups[0];
        let l = &hotmart.courses[0].sections[0].lessons[0];
        assert_eq!(
            core.read_description(l.description.as_deref().unwrap())
                .unwrap(),
            "<p>hola</p>"
        );
        assert!(core.library_path(&l.resources[0].path).is_ok());
    }

    #[tokio::test]
    async fn description_rejects_escapes_non_html_and_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("notes.txt"), "x").unwrap();
        fs::write(tmp.path().join("secret.html"), "x").unwrap();
        let core = core(tmp.path()).await;
        core.library_scan(lib.to_str().unwrap()).unwrap();
        assert!(matches!(
            core.read_description("../secret.html"),
            Err(CommandError::NotFound)
        ));
        assert!(matches!(
            core.read_description("notes.txt"),
            Err(CommandError::NotFound)
        ));
        assert!(matches!(
            core.read_description("missing.html"),
            Err(CommandError::NotFound)
        ));
    }

    #[tokio::test]
    async fn oversized_description_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("big.html"), vec![b'a'; MAX_DESCRIPTION_BYTES + 1]).unwrap();
        let core = core(tmp.path()).await;
        core.library_scan(lib.to_str().unwrap()).unwrap();
        assert!(matches!(
            core.read_description("big.html"),
            Err(CommandError::TooLarge)
        ));
    }

    #[tokio::test]
    async fn library_path_is_confined_to_the_scanned_root() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("v.mp4"), "v").unwrap();
        let core = core(tmp.path()).await;
        core.library_scan(lib.to_str().unwrap()).unwrap();
        let p = core.library_path("v.mp4").unwrap();
        assert!(p.ends_with("v.mp4"));
        assert!(matches!(
            core.library_path("../data/progress.db"),
            Err(CommandError::NotFound)
        ));
    }

    // ---- library root selection ----

    use crate::library_root::{RootError, RootSource};

    struct Env {
        tmp: tempfile::TempDir,
        home: PathBuf,
        videos: PathBuf,
        other: PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(tmp.path()).unwrap().join("home");
        fs::create_dir_all(&home).unwrap();
        Env {
            videos: home.join("Videos/Lumen"),
            other: home.join("Otra biblioteca"),
            home,
            tmp,
        }
    }

    impl Env {
        fn policy(&self) -> RootPolicy {
            RootPolicy::new(
                vec![self.videos.clone(), self.other.clone()],
                Some(self.home.clone()),
            )
        }

        async fn core(&self) -> AppCore {
            AppCore::with_policy(&self.tmp.path().join("data/progress.db"), self.policy())
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn root_source_precedence_saved_then_detected_then_default() {
        let e = env();
        let core = e.core().await;
        let info = core.root_info().unwrap();
        assert_eq!(info.source, RootSource::Default);
        assert_eq!(info.path.as_deref(), e.videos.to_str());
        assert!(!info.exists);
        drop(core);

        fs::create_dir_all(e.other.join("Hotmart")).unwrap();
        let core = e.core().await;
        let info = core.root_info().unwrap();
        assert_eq!(info.source, RootSource::Detected);
        assert_eq!(core.library_root(), e.other);

        let saved = e.home.join("Cursos");
        let info = core.set_root(saved.to_str().unwrap()).unwrap();
        assert_eq!(info.source, RootSource::Saved);
        assert!(info.exists && info.writable);
        assert_eq!(core.library_root(), saved);
    }

    #[tokio::test]
    async fn no_candidates_and_nothing_saved_means_no_root() {
        let tmp = tempfile::tempdir().unwrap();
        let core = AppCore::with_policy(&tmp.path().join("p.db"), RootPolicy::new(vec![], None))
            .await
            .unwrap();
        let info = core.root_info().unwrap();
        assert_eq!(info.path, None);
        assert!(!info.exists && !info.writable);
        assert!(core.library_root().as_os_str().is_empty());
    }

    #[tokio::test]
    async fn saved_root_persists_across_restart_and_reset_returns_to_detection() {
        let e = env();
        fs::create_dir_all(e.other.join("Skool")).unwrap();
        let saved = e.home.join("Mis cursos ñ");
        {
            let core = e.core().await;
            core.set_root(saved.to_str().unwrap()).unwrap();
        }
        let core = e.core().await;
        assert_eq!(core.library_root(), saved);
        assert_eq!(core.root_info().unwrap().source, RootSource::Saved);
        let info = core.reset_root().unwrap();
        assert_eq!(info.source, RootSource::Detected);
        assert_eq!(core.library_root(), e.other);
        drop(core);
        assert_eq!(e.core().await.library_root(), e.other);
        assert!(
            saved.is_dir(),
            "existing data and folders are never touched"
        );
    }

    #[tokio::test]
    async fn a_saved_root_that_vanished_falls_back_to_detection() {
        let e = env();
        let saved = e.home.join("usb");
        {
            let core = e.core().await;
            core.set_root(saved.to_str().unwrap()).unwrap();
        }
        fs::remove_dir_all(&saved).unwrap();
        let core = e.core().await;
        assert_eq!(core.root_info().unwrap().source, RootSource::Default);
        assert_eq!(core.library_root(), e.videos);
    }

    #[tokio::test]
    async fn invalid_roots_are_refused_and_not_persisted() {
        let e = env();
        let core = e.core().await;
        let before = core.library_root();
        assert!(matches!(
            core.set_root("relative/dir"),
            Err(CommandError::Root(RootError::NotAbsolute))
        ));
        assert!(matches!(
            core.set_root(e.home.to_str().unwrap()),
            Err(CommandError::Root(RootError::Forbidden(_)))
        ));
        assert_eq!(core.library_root(), before);
        assert_eq!(core.root_info().unwrap().source, RootSource::Default);
    }

    #[tokio::test]
    async fn downloads_in_flight_keep_the_root_they_started_with() {
        let e = env();
        let core = e.core().await;
        let a = e.home.join("A");
        let b = e.home.join("B");
        core.set_root(a.to_str().unwrap()).unwrap();
        // Downloads receive the root by value when they start.
        let in_flight = core.library_root();
        core.set_root(b.to_str().unwrap()).unwrap();
        let new_download = core.library_root();
        assert_eq!(in_flight, a);
        assert_eq!(new_download, b);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scan_confinement_and_media_server_follow_the_new_root() {
        let e = env();
        let core = e.core().await;
        let a = e.home.join("A");
        let b = e.home.join("B");
        for (dir, name) in [(&a, "a.mp4"), (&b, "b.mp4")] {
            fs::create_dir_all(dir.join("Skool/G/01 - C/01 - S/01 - L")).unwrap();
            fs::write(dir.join(name), name).unwrap();
        }
        core.set_root(a.to_str().unwrap()).unwrap();
        assert!(core.library_path("a.mp4").is_ok());
        core.set_root(b.to_str().unwrap()).unwrap();
        assert!(core.library_path("b.mp4").is_ok());
        assert!(matches!(
            core.library_path("a.mp4"),
            Err(CommandError::NotFound)
        ));
        let library = core
            .library_scan(core.library_root().to_str().unwrap())
            .unwrap();
        assert_eq!(library.platforms[0].groups[0].name, "G");

        let url = format!("{}/b.mp4", core.media_base());
        let status = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let rest = url.trim_start_matches("http://");
            let (host, path) = rest.split_once('/').unwrap();
            let mut s = std::net::TcpStream::connect(host).unwrap();
            write!(
                s,
                "GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        })
        .await
        .unwrap();
        assert!(
            status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.1 206"),
            "{status}"
        );
        assert!(status.ends_with("b.mp4"));
    }

    #[tokio::test]
    async fn summary_tells_whether_a_folder_already_has_courses() {
        let e = env();
        let core = e.core().await;
        fs::create_dir_all(e.home.join("X/Skool")).unwrap();
        let s = core.library_summary(e.home.join("X").to_str().unwrap());
        assert!(s.has_library);
        assert_eq!(s.platforms, vec!["Skool"]);
        assert!(!core.library_summary(e.home.to_str().unwrap()).has_library);
        let tilde = core.library_summary("~/X");
        assert!(tilde.has_library);
        assert!(!core.library_summary("relative").has_library);
    }

    // ---- deleting courses ----

    use crate::library_delete::{DeleteCode, DeleteTarget, SystemTrash, TargetKind, Trasher};
    use std::sync::Arc;

    /// Moves the folder into a sibling "trash" folder, like a real trash would.
    struct MoveTrash(PathBuf);
    impl Trasher for MoveTrash {
        fn trash(&self, path: &Path) -> Result<(), String> {
            fs::create_dir_all(&self.0).map_err(|e| e.to_string())?;
            let name = path.file_name().ok_or("no name")?;
            fs::rename(path, self.0.join(name)).map_err(|e| e.to_string())
        }
    }

    struct FailTrash;
    impl Trasher for FailTrash {
        fn trash(&self, _: &Path) -> Result<(), String> {
            Err("no trash on this filesystem".into())
        }
    }

    fn put(root: &Path, rel: &str, bytes: usize) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![b'x'; bytes]).unwrap();
    }

    fn skool(group: &str, course: Option<&str>) -> DeleteTarget {
        DeleteTarget {
            platform: "skool".into(),
            group: group.into(),
            course: course.map(str::to_string),
            lesson: None,
        }
    }

    /// Library at `e.home/lib` with two Skool groups and one Hotmart course.
    async fn lib_core(e: &Env) -> (AppCore, PathBuf) {
        let lib = e.home.join("lib");
        put(&lib, "Skool/G/01 - C/01 - S/01 - L1/v.mp4", 10);
        put(&lib, "Skool/G/01 - C/01 - S/02 - L2/desc.html", 5);
        put(&lib, "Skool/G/02 - D/01 - S/01 - L1/v.mp4", 20);
        put(&lib, "Skool/G/.complete", 0);
        put(&lib, "Skool/H/01 - C/01 - S/01 - L1/v.mp4", 1);
        put(&lib, "Hotmart/Águila - Ana/1. Módulo/1. Clase/v.mp4", 7);
        let core = e.core().await;
        core.set_root(lib.to_str().unwrap()).unwrap();
        core.set_trasher(Arc::new(MoveTrash(e.home.join("trash"))));
        (core, lib)
    }

    #[tokio::test]
    async fn preview_counts_lessons_videos_and_bytes_without_deleting() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        let p = core.delete_preview(&skool("G", None), false).unwrap();
        assert_eq!(
            (
                p.name.as_str(),
                p.platform.as_str(),
                p.kind,
                p.path.as_str()
            ),
            ("G", "skool", TargetKind::Group, "Skool/G")
        );
        assert_eq!((p.lessons, p.lessons_with_video, p.bytes), (3, 2, 35));
        assert!(!p.bytes_approx && !p.running_download);
        let c = core
            .delete_preview(&skool("G", Some("01 - C")), true)
            .unwrap();
        assert_eq!(c.kind, TargetKind::Course);
        assert_eq!(
            (c.name.as_str(), c.lessons, c.lessons_with_video, c.bytes),
            ("C", 2, 1, 15)
        );
        assert_eq!(c.path, "Skool/G/01 - C");
        assert!(c.running_download);
        let h = DeleteTarget {
            platform: "hotmart".into(),
            group: "hotmart/Águila - Ana".into(),
            course: None,
            lesson: None,
        };
        let h = core.delete_preview(&h, false).unwrap();
        assert_eq!((h.lessons, h.lessons_with_video, h.bytes), (1, 1, 7));
        assert!(lib.join("Skool/G/01 - C").is_dir());
    }

    #[tokio::test]
    async fn deleting_a_group_trashes_it_and_clears_its_progress_only() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        core.progress_set_watched("G", "1-C/1-S/1-L1", true)
            .unwrap();
        core.progress_save_position("G", "2-D/1-S/1-L1", 9).unwrap();
        core.progress_set_last("G", "1-C/1-S/1-L1").unwrap();
        core.progress_set_watched("H", "1-C/1-S/1-L1", true)
            .unwrap();
        let out = core.delete(&skool("G", None), false, false).unwrap();
        assert!(out.deleted && out.moved_to_trash);
        assert_eq!(out.bytes_freed, 35);
        assert!(!lib.join("Skool/G").exists());
        assert!(
            e.home.join("trash/G/.complete").exists(),
            "the marker travels with the group"
        );
        assert!(lib.join("Skool/H").is_dir() && lib.join("Hotmart").is_dir());
        assert_eq!(core.progress_group("G").unwrap(), Default::default());
        assert_eq!(
            core.progress_group("H").unwrap().watched,
            vec!["1-C/1-S/1-L1"]
        );
    }

    #[tokio::test]
    async fn deleting_a_course_keeps_the_rest_of_the_group_and_its_progress() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        core.progress_set_watched("G", "1-C/1-S/1-L1", true)
            .unwrap();
        core.progress_set_watched("G", "2-D/1-S/1-L1", true)
            .unwrap();
        core.progress_save_position("G", "1-C/1-S/1-L1", 4).unwrap();
        core.progress_set_last("G", "1-C/1-S/1-L1").unwrap();
        let out = core
            .delete(&skool("G", Some("01 - C")), false, false)
            .unwrap();
        assert_eq!(out.bytes_freed, 15);
        assert!(!lib.join("Skool/G/01 - C").exists());
        assert!(lib.join("Skool/G/02 - D").is_dir() && lib.join("Skool/G/.complete").exists());
        let p = core.progress_group("G").unwrap();
        assert_eq!(p.watched, vec!["2-D/1-S/1-L1"]);
        assert!(p.positions.is_empty());
        assert_eq!(p.last_lesson, None);
    }

    #[tokio::test]
    async fn deleting_a_hotmart_group_clears_its_qualified_progress() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        let g = "hotmart/Águila - Ana";
        core.progress_set_watched(g, "hotmart:1-Águila/1-Módulo/1-Clase", true)
            .unwrap();
        let t = DeleteTarget {
            platform: "hotmart".into(),
            group: g.into(),
            course: None,
            lesson: None,
        };
        core.delete(&t, false, false).unwrap();
        assert!(!lib.join("Hotmart/Águila - Ana").exists());
        assert_eq!(core.progress_group(g).unwrap(), Default::default());
    }

    #[tokio::test]
    async fn a_failing_trash_deletes_nothing_and_never_falls_back_to_permanent() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        core.set_trasher(Arc::new(FailTrash));
        core.progress_set_watched("G", "1-C/1-S/1-L1", true)
            .unwrap();
        let err = core.delete(&skool("G", None), false, false).unwrap_err();
        assert_eq!(err.code, DeleteCode::TrashFailed);
        assert!(lib.join("Skool/G/01 - C/01 - S/01 - L1/v.mp4").exists());
        assert_eq!(core.progress_group("G").unwrap().watched.len(), 1);
    }

    #[tokio::test]
    async fn permanent_deletion_is_explicit_and_skips_the_trash() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        core.set_trasher(Arc::new(FailTrash));
        let out = core
            .delete(&skool("G", Some("02 - D")), true, false)
            .unwrap();
        assert!(out.deleted && !out.moved_to_trash);
        assert_eq!(out.bytes_freed, 20);
        assert!(!lib.join("Skool/G/02 - D").exists());
        assert!(lib.join("Skool/G/01 - C").is_dir());
    }

    #[tokio::test]
    async fn a_running_download_blocks_deletion_and_missing_targets_are_not_found() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        let err = core.delete(&skool("G", None), true, true).unwrap_err();
        assert_eq!(err.code, DeleteCode::DownloadRunning);
        assert!(lib.join("Skool/G").is_dir());
        let err = core.delete(&skool("Nope", None), false, false).unwrap_err();
        assert_eq!(err.code, DeleteCode::NotFound);
        let err = core.delete(&skool("..", None), true, false).unwrap_err();
        assert_eq!(err.code, DeleteCode::InvalidTarget);
        assert!(lib.join("Skool").is_dir());
    }

    #[tokio::test]
    async fn deletion_follows_the_effective_root_not_a_stale_one() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        let other = e.home.join("other");
        put(&other, "Skool/G/01 - C/01 - S/01 - L/v.mp4", 3);
        core.set_root(other.to_str().unwrap()).unwrap();
        core.delete(&skool("G", None), false, false).unwrap();
        assert!(!other.join("Skool/G").exists());
        assert!(
            lib.join("Skool/G").is_dir(),
            "the previous root is untouched"
        );
    }

    #[tokio::test]
    async fn the_real_system_trash_takes_a_folder_under_home() {
        // Trash across filesystems can fail, so this lives under $HOME rather
        // than /tmp. Where the environment has no usable trash it is skipped.
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            eprintln!("SKIP: no $HOME");
            return;
        };
        let Ok(dir) = tempfile::Builder::new()
            .prefix("lumen-trash-test-")
            .tempdir_in(&home)
        else {
            eprintln!("SKIP: $HOME is not writable");
            return;
        };
        let lib = dir.path().join("lib");
        put(
            &lib,
            "Skool/Lumen delete test ñ/01 - C/01 - S/01 - L/v.mp4",
            4,
        );
        let core = AppCore::with_policy(
            &dir.path().join("p.db"),
            RootPolicy::new(vec![], Some(home.clone())),
        )
        .await
        .unwrap();
        core.set_root(lib.to_str().unwrap()).unwrap();
        core.set_trasher(Arc::new(SystemTrash));
        match core.delete(&skool("Lumen delete test ñ", None), false, false) {
            Ok(out) => {
                assert!(out.moved_to_trash);
                assert!(!lib.join("Skool/Lumen delete test ñ").exists());
            }
            Err(e) if e.code == DeleteCode::TrashFailed => {
                eprintln!("SKIP: system trash unavailable here: {e}");
                assert!(lib.join("Skool/Lumen delete test ñ").is_dir());
            }
            Err(e) => panic!("unexpected: {e}"),
        }
    }

    #[tokio::test]
    async fn library_dir_follows_the_effective_root() {
        let e = env();
        let core = e.core().await;
        let a = e.home.join("A");
        put(&a, "Skool/G/01 - C/v.mp4", 1);
        core.set_root(a.to_str().unwrap()).unwrap();
        assert_eq!(core.library_dir(None).unwrap(), a);
        assert_eq!(
            core.library_dir(Some("Skool/G")).unwrap(),
            a.join("Skool/G")
        );
        let b = e.home.join("B");
        core.set_root(b.to_str().unwrap()).unwrap();
        assert_eq!(core.library_dir(None).unwrap(), b);
        assert!(core.library_dir(Some("Skool/G")).is_err());
    }

    #[tokio::test]
    async fn a_youtube_description_txt_is_shown_as_escaped_html() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(lib.join("YouTube/L/01 - V")).unwrap();
        fs::write(
            lib.join("YouTube/L/01 - V/description.txt"),
            "Hola <b>&</b>\nsegunda línea\n\nOtro párrafo",
        )
        .unwrap();
        fs::write(lib.join("YouTube/L/01 - V/notes.txt"), "x").unwrap();
        let core = core(tmp.path()).await;
        core.library_scan(lib.to_str().unwrap()).unwrap();
        assert_eq!(
            core.read_description("YouTube/L/01 - V/description.txt")
                .unwrap(),
            "<p>Hola &lt;b&gt;&amp;&lt;/b&gt;<br>segunda línea</p><p>Otro párrafo</p>"
        );
        assert!(matches!(
            core.read_description("YouTube/L/01 - V/notes.txt"),
            Err(CommandError::NotFound)
        ));
    }

    #[tokio::test]
    async fn deleting_a_youtube_group_clears_its_progress() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        put(&lib, "YouTube/Mi lista/01 - Uno/Uno.mp4", 5);
        put(&lib, "YouTube/Mi lista/02 - Dos/Dos.mp4", 6);
        let g = "youtube/Mi lista";
        core.progress_set_watched(g, "youtube:1-Mi lista/1-Mi lista/1-Uno", true)
            .unwrap();
        let t = DeleteTarget {
            platform: "youtube".into(),
            group: g.into(),
            course: None,
            lesson: None,
        };
        let p = core.delete_preview(&t, false).unwrap();
        assert_eq!((p.lessons, p.lessons_with_video, p.bytes), (2, 2, 11));
        assert_eq!(p.path, "YouTube/Mi lista");
        let out = core.delete(&t, false, false).unwrap();
        assert_eq!(out.bytes_freed, 11);
        assert!(!lib.join("YouTube/Mi lista").exists());
        assert_eq!(core.progress_group(g).unwrap(), Default::default());
        assert!(lib.join("Skool/G").is_dir());
    }

    // ---- deleting one lesson ----

    fn lesson_target(platform: &str, group: &str, key: &str) -> DeleteTarget {
        DeleteTarget {
            platform: platform.into(),
            group: group.into(),
            course: None,
            lesson: Some(key.into()),
        }
    }

    #[tokio::test]
    async fn a_skool_lesson_folder_goes_to_the_trash_with_only_its_own_progress() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        let t = lesson_target("skool", "G", "1-C/1-S/1-L1");
        let p = core.delete_preview(&t, false).unwrap();
        assert_eq!(
            (
                p.kind,
                p.name.as_str(),
                p.lessons,
                p.lessons_with_video,
                p.bytes
            ),
            (TargetKind::Lesson, "L1", 1, 1, 10)
        );
        assert_eq!(p.path, "Skool/G/01 - C/01 - S/01 - L1");
        let nov = core
            .delete_preview(&lesson_target("skool", "G", "1-C/1-S/2-L2"), false)
            .unwrap();
        assert_eq!((nov.lessons_with_video, nov.bytes), (0, 5));
        core.progress_set_watched("G", "1-C/1-S/1-L1", true)
            .unwrap();
        core.progress_save_position("G", "1-C/1-S/1-L1", 5).unwrap();
        core.progress_set_last("G", "1-C/1-S/1-L1").unwrap();
        core.progress_set_watched("G", "1-C/1-S/2-L2", true)
            .unwrap();
        let out = core.delete(&t, false, false).unwrap();
        assert!(out.deleted && out.moved_to_trash && !out.group_removed);
        assert_eq!(out.bytes_freed, 10);
        assert!(!lib.join("Skool/G/01 - C/01 - S/01 - L1").exists());
        assert!(e.home.join("trash/01 - L1/v.mp4").exists());
        assert!(
            lib.join("Skool/G/01 - C/01 - S/02 - L2").is_dir()
                && lib.join("Skool/G/02 - D").is_dir()
        );
        assert!(lib.join("Skool/G/.complete").exists());
        let pr = core.progress_group("G").unwrap();
        assert_eq!(pr.watched, vec!["1-C/1-S/2-L2"]);
        assert!(pr.positions.is_empty());
        assert_eq!(pr.last_lesson, None);
    }

    #[tokio::test]
    async fn the_last_lesson_of_a_group_takes_the_empty_group_with_it() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        let g = "hotmart/Águila - Ana";
        let t = lesson_target("hotmart", g, "hotmart:1-Águila/1-Módulo/1-Clase");
        core.progress_set_watched(g, "hotmart:1-Águila/1-Módulo/1-Clase", true)
            .unwrap();
        let out = core.delete(&t, false, false).unwrap();
        assert!(out.group_removed);
        assert!(!lib.join("Hotmart/Águila - Ana").exists());
        assert!(e.home.join("trash/Águila - Ana").is_dir());
        assert_eq!(core.progress_group(g).unwrap(), Default::default());
        assert!(lib.join("Skool/G").is_dir());
    }

    #[tokio::test]
    async fn youtube_videos_are_removed_one_by_one_until_the_group_is_empty() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        for n in ["01 - Uno", "02 - Dos"] {
            let t = n.split(" - ").nth(1).unwrap();
            put(&lib, &format!("YouTube/Videos sueltos/{n}/{t}.mp4"), 4);
            put(
                &lib,
                &format!("YouTube/Videos sueltos/{n}/description.txt"),
                2,
            );
            put(
                &lib,
                &format!("YouTube/Videos sueltos/{n}/thumbnail.jpg"),
                1,
            );
            put(&lib, &format!("YouTube/Videos sueltos/{n}/.youtube-id"), 1);
        }
        put(&lib, "YouTube/Videos sueltos/download-report.json", 3);
        put(&lib, "YouTube/Videos sueltos/.complete", 0);
        let g = "youtube/Videos sueltos";
        let key = |n: &str| format!("youtube:1-Videos sueltos/1-Videos sueltos/{n}");
        let p = core
            .delete_preview(&lesson_target("youtube", g, &key("1-Uno")), false)
            .unwrap();
        assert_eq!((p.lessons, p.lessons_with_video, p.bytes), (1, 1, 8));
        let first = core
            .delete(&lesson_target("youtube", g, &key("1-Uno")), false, false)
            .unwrap();
        assert!(!first.group_removed);
        assert!(
            lib.join("YouTube/Videos sueltos/.complete").exists()
                && lib.join("YouTube/Videos sueltos/02 - Dos").is_dir()
        );
        let second = core
            .delete(&lesson_target("youtube", g, &key("2-Dos")), false, false)
            .unwrap();
        assert!(
            second.group_removed,
            "only .complete and the report were left"
        );
        assert!(!lib.join("YouTube/Videos sueltos").exists());
        assert!(lib.join("YouTube").is_dir());
    }

    #[tokio::test]
    async fn a_group_with_files_of_its_own_is_never_removed_with_its_last_lesson() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        put(&lib, "YouTube/L/01 - Uno/Uno.mp4", 4);
        put(&lib, "YouTube/L/notes.txt", 9);
        let out = core
            .delete(
                &lesson_target("youtube", "youtube/L", "youtube:1-L/1-L/1-Uno"),
                false,
                false,
            )
            .unwrap();
        assert!(!out.group_removed);
        assert!(lib.join("YouTube/L/notes.txt").exists());
    }

    #[tokio::test]
    async fn a_loose_file_lesson_deletes_only_its_video_and_description() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        put(&lib, "Hotmart/H - Me/1. M/2. Suelta.mp4", 6);
        put(&lib, "Hotmart/H - Me/1. M/2. Suelta.html", 2);
        put(&lib, "Hotmart/H - Me/1. M/3. Otra.mp4", 7);
        let t = lesson_target("hotmart", "hotmart/H - Me", "hotmart:1-H/1-M/2-Suelta");
        let p = core.delete_preview(&t, false).unwrap();
        assert_eq!((p.lessons, p.lessons_with_video, p.bytes), (1, 1, 8));
        assert_eq!(p.path, "Hotmart/H - Me/1. M/2. Suelta.mp4");
        let out = core.delete(&t, false, false).unwrap();
        assert_eq!((out.bytes_freed, out.group_removed), (8, false));
        assert!(
            !lib.join("Hotmart/H - Me/1. M/2. Suelta.mp4").exists()
                && !lib.join("Hotmart/H - Me/1. M/2. Suelta.html").exists()
        );
        assert!(lib.join("Hotmart/H - Me/1. M/3. Otra.mp4").exists());
        assert!(e.home.join("trash/2. Suelta.mp4").exists());
        let perm = lesson_target("hotmart", "hotmart/H - Me", "hotmart:1-H/1-M/3-Otra");
        core.set_trasher(Arc::new(FailTrash));
        assert_eq!(
            core.delete(&perm, false, false).unwrap_err().code,
            DeleteCode::TrashFailed
        );
        assert!(lib.join("Hotmart/H - Me/1. M/3. Otra.mp4").exists());
        let out = core.delete(&perm, true, false).unwrap();
        assert!(
            !out.moved_to_trash && out.group_removed,
            "permanent, and nothing left in the group"
        );
    }

    #[tokio::test]
    async fn lesson_deletion_refuses_unknown_lessons_running_downloads_and_failing_trash() {
        let e = env();
        let (core, lib) = lib_core(&e).await;
        let t = lesson_target("skool", "G", "1-C/1-S/1-L1");
        assert_eq!(
            core.delete(&lesson_target("skool", "G", "9-X/9-X/9-X"), false, false)
                .unwrap_err()
                .code,
            DeleteCode::LessonNotFound
        );
        assert_eq!(
            core.delete_preview(&lesson_target("skool", "G", "../../x"), false)
                .unwrap_err()
                .code,
            DeleteCode::LessonNotFound
        );
        assert_eq!(
            core.delete(&t, true, true).unwrap_err().code,
            DeleteCode::DownloadRunning
        );
        core.set_trasher(Arc::new(FailTrash));
        core.progress_set_watched("G", "1-C/1-S/1-L1", true)
            .unwrap();
        assert_eq!(
            core.delete(&t, false, false).unwrap_err().code,
            DeleteCode::TrashFailed
        );
        assert!(lib.join("Skool/G/01 - C/01 - S/01 - L1/v.mp4").exists());
        assert_eq!(
            core.progress_group("G").unwrap().watched.len(),
            1,
            "progress stays when nothing was deleted"
        );
        // The course the UI may send is ignored for lesson targets.
        let mut with_course = t.clone();
        with_course.course = Some("../../etc".into());
        assert!(core.delete_preview(&with_course, false).is_ok());
    }

    #[tokio::test]
    async fn deleting_a_whole_group_reports_it_as_removed() {
        let e = env();
        let (core, _lib) = lib_core(&e).await;
        let g = core.delete(&skool("G", None), false, false).unwrap();
        assert!(g.group_removed);
        let e2 = env();
        let (core2, _l2) = lib_core(&e2).await;
        let c = core2
            .delete(&skool("G", Some("01 - C")), false, false)
            .unwrap();
        assert!(!c.group_removed);
    }
}
