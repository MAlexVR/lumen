//! Deleting downloaded courses from the library, defensively: the path is
//! rebuilt strictly from validated components (the UI never sends a path),
//! must be exactly a group or course folder inside the current root, never
//! crosses a symlink, and goes to the OS trash unless the caller explicitly
//! asks for permanent deletion. Plain `std` plus a trash adapter, so it is
//! tested without a window.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use lumen_core::LibraryPlatform;
use serde::{Deserialize, Serialize};

/// Stable machine-readable error codes for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteCode {
    /// The target is malformed or would leave the library.
    InvalidTarget,
    NotFound,
    /// A download of that platform is in progress.
    DownloadRunning,
    /// The OS trash refused; nothing was deleted.
    TrashFailed,
    Internal,
}

/// Serialized to the UI as `{ "code": "...", "message": "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct DeleteError {
    pub code: DeleteCode,
    pub message: String,
}

impl DeleteError {
    pub fn new(code: DeleteCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(context: &str, e: impl std::fmt::Display) -> Self {
        Self::new(DeleteCode::Internal, format!("{context}: {e}"))
    }
}

/// What the UI asks to delete. `group` is the id used by `library_scan`:
/// the Skool group name, or `hotmart/<folder>`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTarget {
    pub platform: String,
    pub group: String,
    pub course: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Group,
    Course,
}

/// A target resolved to a real folder inside the library root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub platform: LibraryPlatform,
    pub kind: TargetKind,
    /// Progress group id (`G` for Skool, `hotmart/<folder>`).
    pub group_id: String,
    pub group_folder: String,
    pub course_folder: Option<String>,
    pub path: PathBuf,
    /// Library-relative path with `/` separators.
    pub rel: String,
}

/// Rejects anything that is not a plain folder name: empty, `.`/`..`,
/// separators, control characters, hidden names, and trailing dots or spaces
/// (which Windows silently strips, aliasing another folder).
pub(crate) fn check_name(name: &str) -> Result<(), DeleteError> {
    let bad = name.is_empty()
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        );
    if bad {
        return Err(DeleteError::new(
            DeleteCode::InvalidTarget,
            format!("\"{name}\" is not a valid folder name"),
        ));
    }
    Ok(())
}

fn parse_platform(name: &str) -> Result<LibraryPlatform, DeleteError> {
    match name {
        "skool" => Ok(LibraryPlatform::Skool),
        "hotmart" => Ok(LibraryPlatform::Hotmart),
        "youtube" => Ok(LibraryPlatform::Youtube),
        "pluralsight" => Ok(LibraryPlatform::Pluralsight),
        other => Err(DeleteError::new(
            DeleteCode::InvalidTarget,
            format!("unknown platform \"{other}\""),
        )),
    }
}

fn not_found() -> DeleteError {
    DeleteError::new(DeleteCode::NotFound, "that course is not in the library")
}

/// Builds the folder for `target` under `root` and proves it is exactly a
/// group (depth 2) or course (depth 3) directory that really lives inside the
/// canonical root, with no symlink anywhere along the way.
pub fn resolve_target(root: &Path, target: &DeleteTarget) -> Result<Resolved, DeleteError> {
    let platform = parse_platform(&target.platform)?;
    let group_folder = match platform {
        LibraryPlatform::Skool => target.group.as_str(),
        LibraryPlatform::Hotmart | LibraryPlatform::Youtube | LibraryPlatform::Pluralsight => {
            let prefix = format!("{}/", target.platform);
            target.group.strip_prefix(&prefix).ok_or_else(|| {
                DeleteError::new(
                    DeleteCode::InvalidTarget,
                    format!("{} groups look like {prefix}<folder>", target.platform),
                )
            })?
        }
    };
    check_name(group_folder)?;
    let course_folder = match (&target.course, platform) {
        (None, _) => None,
        (Some(c), LibraryPlatform::Skool) => {
            check_name(c)?;
            Some(c.as_str())
        }
        (Some(_), _) => {
            return Err(DeleteError::new(
                DeleteCode::InvalidTarget,
                "this platform has no course level: delete the group",
            ))
        }
    };
    if root.as_os_str().is_empty() {
        return Err(not_found());
    }
    let canon_root = fs::canonicalize(root).map_err(|_| not_found())?;
    let mut expected = canon_root.join(platform.dir_name()).join(group_folder);
    let mut rel = format!("{}/{group_folder}", platform.dir_name());
    if let Some(c) = course_folder {
        expected.push(c);
        rel = format!("{rel}/{c}");
    }
    let canonical = fs::canonicalize(&expected).map_err(|_| not_found())?;
    // Any symlink along the way makes the canonical path differ.
    if canonical != expected {
        return Err(DeleteError::new(
            DeleteCode::InvalidTarget,
            "the folder resolves outside the library (symlink)",
        ));
    }
    if !canonical.is_dir() {
        return Err(not_found());
    }
    let group_id = match platform {
        LibraryPlatform::Skool => group_folder.to_string(),
        LibraryPlatform::Hotmart | LibraryPlatform::Youtube | LibraryPlatform::Pluralsight => {
            format!("{}/{group_folder}", target.platform)
        }
    };
    Ok(Resolved {
        platform,
        kind: if course_folder.is_some() {
            TargetKind::Course
        } else {
            TargetKind::Group
        },
        group_id,
        group_folder: group_folder.to_string(),
        course_folder: course_folder.map(str::to_string),
        path: canonical,
        rel,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirSize {
    pub bytes: u64,
    /// True when the walk stopped early (entry cap or time budget).
    pub approx: bool,
}

/// Sums file sizes below `dir` without following symlinks, giving up after
/// `max_entries` entries or `budget`.
pub fn dir_size(dir: &Path, max_entries: usize, budget: Duration) -> DirSize {
    let start = Instant::now();
    let (mut bytes, mut seen, mut approx) = (0u64, 0usize, false);
    let mut stack = vec![dir.to_path_buf()];
    'walk: while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > max_entries || start.elapsed() >= budget {
                approx = true;
                break 'walk;
            }
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                bytes += meta.len();
            }
        }
    }
    DirSize { bytes, approx }
}

/// Moves a folder to the OS trash. Behind a trait so failures are testable.
pub trait Trasher: Send + Sync {
    fn trash(&self, path: &Path) -> Result<(), String>;
}

/// freedesktop Trash on Linux, Recycle Bin on Windows, Trash on macOS.
pub struct SystemTrash;

impl Trasher for SystemTrash {
    fn trash(&self, path: &Path) -> Result<(), String> {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

/// Entry cap and time budget for size estimates.
pub const SIZE_MAX_ENTRIES: usize = 500_000;
pub const SIZE_BUDGET: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeletePreview {
    pub name: String,
    pub platform: String,
    pub kind: TargetKind,
    pub lessons: u64,
    pub lessons_with_video: u64,
    pub bytes: u64,
    pub bytes_approx: bool,
    /// Library-relative path, for display only.
    pub path: String,
    pub running_download: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    pub deleted: bool,
    pub moved_to_trash: bool,
    pub bytes_freed: u64,
    pub bytes_approx: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn lib() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for d in [
            "Skool/Growth ñ/01 - Curso/01 - Sec/01 - Lec",
            "Skool/Growth ñ/02 - Otro/01 - Sec/01 - Lec",
            "Hotmart/Águila - Ana/1. Módulo/1. Clase",
        ] {
            fs::create_dir_all(tmp.path().join(d)).unwrap();
        }
        tmp
    }

    fn skool(group: &str, course: Option<&str>) -> DeleteTarget {
        DeleteTarget {
            platform: "skool".into(),
            group: group.into(),
            course: course.map(str::to_string),
        }
    }

    fn hotmart(group: &str) -> DeleteTarget {
        DeleteTarget {
            platform: "hotmart".into(),
            group: group.into(),
            course: None,
        }
    }

    fn pluralsight(group: &str) -> DeleteTarget {
        DeleteTarget {
            platform: "pluralsight".into(),
            group: group.into(),
            course: None,
        }
    }

    fn code(r: Result<Resolved, DeleteError>) -> DeleteCode {
        r.unwrap_err().code
    }

    #[test]
    fn builds_group_and_course_paths_from_components() {
        let t = lib();
        let root = fs::canonicalize(t.path()).unwrap();
        let g = resolve_target(&root, &skool("Growth ñ", None)).unwrap();
        assert_eq!(g.path, root.join("Skool/Growth ñ"));
        assert_eq!(g.kind, TargetKind::Group);
        assert_eq!(g.rel, "Skool/Growth ñ");
        let c = resolve_target(&root, &skool("Growth ñ", Some("01 - Curso"))).unwrap();
        assert_eq!(c.path, root.join("Skool/Growth ñ/01 - Curso"));
        assert_eq!(c.kind, TargetKind::Course);
        assert_eq!(c.rel, "Skool/Growth ñ/01 - Curso");
        let h = resolve_target(&root, &hotmart("hotmart/Águila - Ana")).unwrap();
        assert_eq!(h.path, root.join("Hotmart/Águila - Ana"));
        assert_eq!(h.rel, "Hotmart/Águila - Ana");
        assert_eq!(h.group_id, "hotmart/Águila - Ana");
    }

    #[test]
    fn resolves_qualified_pluralsight_group_and_rejects_nested_course() {
        let t = lib();
        fs::create_dir_all(
            t.path()
                .join("Pluralsight/Rust Course-0123456789abcdef/01 - Module/01 - Clip"),
        )
        .unwrap();
        let group = resolve_target(
            t.path(),
            &pluralsight("pluralsight/Rust Course-0123456789abcdef"),
        )
        .unwrap();
        assert_eq!(group.platform, LibraryPlatform::Pluralsight);
        assert_eq!(group.group_id, "pluralsight/Rust Course-0123456789abcdef");
        assert_eq!(group.rel, "Pluralsight/Rust Course-0123456789abcdef");
        let mut nested = pluralsight("pluralsight/Rust Course-0123456789abcdef");
        nested.course = Some("01 - Module".into());
        assert_eq!(
            code(resolve_target(t.path(), &nested)),
            DeleteCode::InvalidTarget
        );
    }

    #[test]
    fn traversal_separators_and_odd_names_are_invalid_targets() {
        let t = lib();
        let bad = [
            "",
            ".",
            "..",
            "../Hotmart",
            "a/b",
            "a\\b",
            "x\0y",
            "Growth ñ.",
            "Growth ñ ",
            ".hidden",
            "Skool/Growth ñ",
            "/etc",
        ];
        for name in bad {
            assert_eq!(
                code(resolve_target(t.path(), &skool(name, None))),
                DeleteCode::InvalidTarget,
                "group {name:?}"
            );
            assert_eq!(
                code(resolve_target(t.path(), &skool("Growth ñ", Some(name)))),
                DeleteCode::InvalidTarget,
                "course {name:?}"
            );
        }
        for g in [
            "hotmart/",
            "hotmart/..",
            "hotmart/a/b",
            "Águila - Ana",
            "skool/x",
            "hotmart",
        ] {
            assert_eq!(
                code(resolve_target(t.path(), &hotmart(g))),
                DeleteCode::InvalidTarget,
                "{g}"
            );
        }
        let mut with_course = hotmart("hotmart/Águila - Ana");
        with_course.course = Some("x".into());
        assert_eq!(
            code(resolve_target(t.path(), &with_course)),
            DeleteCode::InvalidTarget
        );
        let mut other = skool("Growth ñ", None);
        other.platform = "udemy".into();
        assert_eq!(
            code(resolve_target(t.path(), &other)),
            DeleteCode::InvalidTarget
        );
        assert!(t.path().join("Skool/Growth ñ").is_dir());
    }

    fn youtube(group: &str) -> DeleteTarget {
        DeleteTarget {
            platform: "youtube".into(),
            group: group.into(),
            course: None,
        }
    }

    #[test]
    fn youtube_groups_are_playlist_folders_without_a_course_level() {
        let t = lib();
        let root = fs::canonicalize(t.path()).unwrap();
        fs::create_dir_all(root.join("YouTube/Mi lista ñ/01 - Video")).unwrap();
        fs::create_dir_all(root.join("YouTube/Videos sueltos")).unwrap();
        let r = resolve_target(&root, &youtube("youtube/Mi lista ñ")).unwrap();
        assert_eq!(r.path, root.join("YouTube/Mi lista ñ"));
        assert_eq!(r.rel, "YouTube/Mi lista ñ");
        assert_eq!(r.group_id, "youtube/Mi lista ñ");
        assert_eq!(r.kind, TargetKind::Group);
        assert!(resolve_target(&root, &youtube("youtube/Videos sueltos")).is_ok());
        for bad in [
            "Mi lista ñ",
            "youtube/",
            "youtube/..",
            "youtube/a/b",
            "hotmart/Mi lista ñ",
        ] {
            assert_eq!(
                code(resolve_target(&root, &youtube(bad))),
                DeleteCode::InvalidTarget,
                "{bad}"
            );
        }
        let mut with_course = youtube("youtube/Mi lista ñ");
        with_course.course = Some("01 - Video".into());
        assert_eq!(
            code(resolve_target(&root, &with_course)),
            DeleteCode::InvalidTarget
        );
        assert_eq!(
            code(resolve_target(&root, &youtube("youtube/Nope"))),
            DeleteCode::NotFound
        );
    }

    #[test]
    fn missing_things_are_not_found() {
        let t = lib();
        assert_eq!(
            code(resolve_target(t.path(), &skool("Nope", None))),
            DeleteCode::NotFound
        );
        assert_eq!(
            code(resolve_target(
                t.path(),
                &skool("Growth ñ", Some("09 - Nope"))
            )),
            DeleteCode::NotFound
        );
        assert_eq!(
            code(resolve_target(t.path(), &hotmart("hotmart/Nope"))),
            DeleteCode::NotFound
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            code(resolve_target(empty.path(), &skool("G", None))),
            DeleteCode::NotFound
        );
        assert_eq!(
            code(resolve_target(&t.path().join("missing"), &skool("G", None))),
            DeleteCode::NotFound
        );
        assert_eq!(
            code(resolve_target(Path::new(""), &skool("G", None))),
            DeleteCode::NotFound
        );
        fs::write(t.path().join("Skool/file.txt"), "x").unwrap();
        assert_eq!(
            code(resolve_target(t.path(), &skool("file.txt", None))),
            DeleteCode::NotFound
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_followed_out_of_the_library() {
        let t = lib();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("precious.txt"), "x").unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("Skool/Evil")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("Skool/Growth ñ/03 - Evil")).unwrap();
        assert_eq!(
            code(resolve_target(&root, &skool("Evil", None))),
            DeleteCode::InvalidTarget
        );
        assert_eq!(
            code(resolve_target(&root, &skool("Growth ñ", Some("03 - Evil")))),
            DeleteCode::InvalidTarget
        );
        // A symlinked platform folder is refused too.
        let other = tempfile::tempdir().unwrap();
        fs::create_dir_all(outside.path().join("G")).unwrap();
        std::os::unix::fs::symlink(outside.path(), other.path().join("Skool")).unwrap();
        assert_eq!(
            code(resolve_target(other.path(), &skool("G", None))),
            DeleteCode::InvalidTarget
        );
        assert!(outside.path().join("precious.txt").exists());
    }

    #[test]
    fn stats_count_files_bytes_and_flag_truncation() {
        let t = lib();
        fs::write(
            t.path()
                .join("Skool/Growth ñ/01 - Curso/01 - Sec/01 - Lec/a.bin"),
            vec![0; 100],
        )
        .unwrap();
        fs::write(t.path().join("Skool/Growth ñ/.complete"), "").unwrap();
        fs::write(
            t.path().join("Skool/Growth ñ/download-report.json"),
            vec![0; 7],
        )
        .unwrap();
        let dir = t.path().join("Skool/Growth ñ");
        let full = dir_size(&dir, 1000, std::time::Duration::from_secs(5));
        assert_eq!((full.bytes, full.approx), (107, false));
        let cut = dir_size(&dir, 2, std::time::Duration::from_secs(5));
        assert!(cut.approx);
        let timed = dir_size(&dir, 1000, std::time::Duration::ZERO);
        assert!(timed.approx);
    }

    #[test]
    fn errors_serialize_as_code_and_message() {
        let e = DeleteError::new(DeleteCode::DownloadRunning, "busy");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "download_running");
        assert_eq!(v["message"], "busy");
        for (c, s) in [
            (DeleteCode::InvalidTarget, "invalid_target"),
            (DeleteCode::NotFound, "not_found"),
            (DeleteCode::TrashFailed, "trash_failed"),
            (DeleteCode::Internal, "internal"),
        ] {
            assert_eq!(serde_json::to_value(c).unwrap(), s);
        }
    }
}
