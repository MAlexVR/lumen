//! Opening a library folder in the OS file manager. The folder is rebuilt
//! from validated components under the current root, so the UI can never
//! point the file manager outside the library. (`open_library_path` stays for
//! files: it follows the media server's regular-files-only rule.)

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::library_delete::check_name;

/// Stable machine-readable error codes for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCode {
    /// Malformed path, `..`, or a symlink along the way.
    InvalidPath,
    NotFound,
    NotADirectory,
    /// The system could not launch the file manager.
    OpenFailed,
}

/// Serialized to the UI as `{ "code": "...", "message": "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct OpenError {
    pub code: OpenCode,
    pub message: String,
}

impl OpenError {
    pub fn new(code: OpenCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn not_found() -> OpenError {
    OpenError::new(OpenCode::NotFound, "that folder does not exist")
}

/// The library root (`rel` empty or absent) or an existing directory inside
/// it, as a canonical path. `rel` uses `/` and is checked segment by segment
/// like a delete target; any symlink along the way is refused.
pub fn resolve_dir(root: &Path, rel: Option<&str>) -> Result<PathBuf, OpenError> {
    let rel = rel.map(str::trim).unwrap_or_default();
    let segments: Vec<&str> = if rel.is_empty() {
        Vec::new()
    } else {
        rel.strip_suffix('/').unwrap_or(rel).split('/').collect()
    };
    for seg in &segments {
        check_name(seg).map_err(|e| OpenError::new(OpenCode::InvalidPath, e.message))?;
    }
    if root.as_os_str().is_empty() {
        return Err(not_found());
    }
    let canon_root = fs::canonicalize(root).map_err(|_| not_found())?;
    let mut expected = canon_root;
    expected.extend(&segments);
    let canonical = fs::canonicalize(&expected).map_err(|_| not_found())?;
    if canonical != expected {
        return Err(OpenError::new(
            OpenCode::InvalidPath,
            "the folder resolves outside the library (symlink)",
        ));
    }
    if !canonical.is_dir() {
        return Err(OpenError::new(
            OpenCode::NotADirectory,
            "that path is a file, not a folder",
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn lib() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap().join("lib");
        fs::create_dir_all(root.join("Skool/Growth ñ/01 - C/01 - S")).unwrap();
        fs::create_dir_all(root.join("Hotmart/Águila - Ana")).unwrap();
        fs::write(root.join("Skool/Growth ñ/.complete"), "").unwrap();
        (tmp, root)
    }

    fn code(r: Result<PathBuf, OpenError>) -> OpenCode {
        r.unwrap_err().code
    }

    #[test]
    fn empty_or_missing_rel_is_the_library_root() {
        let (_t, root) = lib();
        assert_eq!(resolve_dir(&root, None).unwrap(), root);
        assert_eq!(resolve_dir(&root, Some("")).unwrap(), root);
        assert_eq!(resolve_dir(&root, Some("  ")).unwrap(), root);
    }

    #[test]
    fn group_and_nested_directories_resolve() {
        let (_t, root) = lib();
        assert_eq!(
            resolve_dir(&root, Some("Skool")).unwrap(),
            root.join("Skool")
        );
        assert_eq!(
            resolve_dir(&root, Some("Skool/Growth ñ")).unwrap(),
            root.join("Skool/Growth ñ")
        );
        assert_eq!(
            resolve_dir(&root, Some("Skool/Growth ñ/01 - C/01 - S/")).unwrap(),
            root.join("Skool/Growth ñ/01 - C/01 - S")
        );
        assert_eq!(
            resolve_dir(&root, Some("Hotmart/Águila - Ana")).unwrap(),
            root.join("Hotmart/Águila - Ana")
        );
    }

    #[test]
    fn youtube_folders_open_like_any_other() {
        let (_t, root) = lib();
        fs::create_dir_all(root.join("YouTube/Videos sueltos/01 - Canción")).unwrap();
        assert_eq!(
            resolve_dir(&root, Some("YouTube/Videos sueltos")).unwrap(),
            root.join("YouTube/Videos sueltos")
        );
        assert!(resolve_dir(&root, Some("YouTube/Videos sueltos/01 - Canción/")).is_ok());
    }

    #[test]
    fn traversal_and_odd_paths_are_invalid() {
        let (_t, root) = lib();
        for rel in [
            "..",
            "../lib",
            "Skool/..",
            "Skool/../..",
            "/etc",
            "/Skool",
            "Skool//Growth ñ",
            "Skool\\Growth ñ",
            "Skool/\0x",
            "Skool/. ",
            "Skool/...",
            ".",
            "./Skool",
            "Skool/.hidden",
            "Skool/Growth ñ.",
        ] {
            assert_eq!(
                code(resolve_dir(&root, Some(rel))),
                OpenCode::InvalidPath,
                "{rel:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_library_is_refused() {
        let (t, root) = lib();
        let outside = t.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(outside.join("x")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Skool/Evil")).unwrap();
        assert_eq!(
            code(resolve_dir(&root, Some("Skool/Evil"))),
            OpenCode::InvalidPath
        );
        assert_eq!(
            code(resolve_dir(&root, Some("Skool/Evil/x"))),
            OpenCode::InvalidPath
        );
    }

    #[test]
    fn files_and_missing_paths_get_typed_errors() {
        let (_t, root) = lib();
        assert_eq!(
            code(resolve_dir(&root, Some("Skool/Growth ñ/.complete"))),
            OpenCode::InvalidPath, // hidden name
        );
        fs::write(root.join("Skool/notes.txt"), "x").unwrap();
        assert_eq!(
            code(resolve_dir(&root, Some("Skool/notes.txt"))),
            OpenCode::NotADirectory
        );
        assert_eq!(
            code(resolve_dir(&root, Some("Skool/Nope"))),
            OpenCode::NotFound
        );
        assert_eq!(
            code(resolve_dir(&root.join("missing"), None)),
            OpenCode::NotFound
        );
        assert_eq!(code(resolve_dir(Path::new(""), None)), OpenCode::NotFound);
    }

    #[test]
    fn errors_serialize_as_code_and_message() {
        let e = OpenError::new(OpenCode::NotADirectory, "x");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "not_a_directory");
        assert_eq!(
            serde_json::to_value(OpenCode::OpenFailed).unwrap(),
            "open_failed"
        );
    }
}
