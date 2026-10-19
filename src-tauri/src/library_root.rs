//! Choosing where the library (and new downloads) live. Everything here is
//! plain `std` so it is testable without a window; commands only delegate.
//!
//! Rejected roots (see [`RootValidator`]): empty or relative paths, the
//! filesystem root, the user's home folder itself and any ancestor of it,
//! system folders (`/etc`, `/usr`, `C:\Windows`, ...), files, and folders the
//! app cannot write to. Home subfolders are allowed.

use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};

use lumen_core::LibraryPlatform;
use serde::Serialize;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RootError {
    #[error("choose a folder")]
    Empty,
    #[error("the folder path must be absolute")]
    NotAbsolute,
    #[error("that folder cannot be used as a library: {0}")]
    Forbidden(String),
    #[error("that path is a file, not a folder")]
    NotADirectory,
    #[error("cannot use that folder: {0}")]
    Io(String),
    #[error("the folder is not writable")]
    NotWritable,
}

/// Top-level folders of a system, matched case-insensitively right under the
/// filesystem root (or drive root). Nothing inside them may be a library.
const SYSTEM_DIRS: [&str; 17] = [
    "bin",
    "boot",
    "dev",
    "etc",
    "lib",
    "lib64",
    "proc",
    "sbin",
    "sys",
    "usr",
    "system",
    "library",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
];

/// Where the effective root came from.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RootSource {
    /// Chosen by the user and stored in settings.
    Saved,
    /// Nothing saved; an existing library was found at a known location.
    Detected,
    /// Nothing saved or found; the first known location, possibly not created.
    Default,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RootInfo {
    pub path: Option<String>,
    pub source: RootSource,
    pub exists: bool,
    pub writable: bool,
}

impl RootInfo {
    pub fn describe(root: Option<&Path>, source: RootSource) -> Self {
        Self {
            path: root.map(display_path),
            source,
            exists: root.is_some_and(Path::is_dir),
            writable: root.is_some_and(is_writable),
        }
    }
}

/// Which platform folders a candidate library root already contains.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    pub platforms: Vec<String>,
    pub has_library: bool,
}

/// Cheap check: only looks at the platform folder names, no deep scan.
pub fn summarize(root: &Path) -> LibrarySummary {
    let platforms: Vec<String> = LibraryPlatform::ALL
        .iter()
        .map(|p| p.dir_name().to_string())
        .filter(|name| root.join(name).is_dir())
        .collect();
    LibrarySummary {
        has_library: !platforms.is_empty(),
        platforms,
    }
}

/// Removes the Windows verbatim prefix (`\\?\C:\x` -> `C:\x`,
/// `\\?\UNC\srv\share` -> `\\srv\share`); any other text is returned as is.
pub fn strip_verbatim(text: &str) -> String {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text.to_string()
    }
}

/// Path as shown to the user and sent to the UI.
pub fn display_path(path: &Path) -> String {
    strip_verbatim(&path.to_string_lossy())
}

/// Folds `.` and `..` lexically so a `..` cannot dodge the forbidden checks.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir) | None
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when a file can be created and removed inside `dir`.
pub fn is_writable(dir: &Path) -> bool {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let probe = dir.join(format!(".lumen-probe-{}-{nanos}", std::process::id()));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => fs::remove_file(&probe).is_ok(),
        Err(_) => false,
    }
}

pub struct RootValidator {
    home: Option<PathBuf>,
}

impl RootValidator {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self { home }
    }

    fn expand(&self, input: &str) -> Result<PathBuf, RootError> {
        let rest = input
            .strip_prefix('~')
            .filter(|r| r.is_empty() || r.starts_with(['/', '\\']));
        match (rest, &self.home) {
            (Some(rest), Some(home)) => Ok(home.join(rest.trim_start_matches(['/', '\\']))),
            (Some(_), None) => Err(RootError::NotAbsolute),
            (None, _) => Ok(PathBuf::from(input)),
        }
    }

    /// Expands `~` and returns the path only when it is absolute.
    pub fn resolve_input(&self, input: &str) -> Option<PathBuf> {
        let path = self.expand(input.trim()).ok()?;
        path.is_absolute().then_some(path)
    }

    fn check_allowed(&self, path: &Path) -> Result<(), RootError> {
        let forbid = |why: &str| Err(RootError::Forbidden(why.to_string()));
        let mut normals = path.components().filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().to_lowercase()),
            _ => None,
        });
        let first = normals.next();
        let Some(first) = first else {
            return forbid("the filesystem root");
        };
        if SYSTEM_DIRS.contains(&first.as_str()) {
            return forbid("a system folder");
        }
        if let Some(home) = &self.home {
            let home = normalize(home);
            if path == home {
                return forbid("your home folder itself, pick a subfolder");
            }
            if home.starts_with(path) {
                return forbid("a folder that contains your home folder");
            }
        }
        Ok(())
    }

    /// Validates `input`, creates the folder when missing and returns its
    /// canonical, display-safe path. Nothing is persisted here.
    pub fn validate(&self, input: &str) -> Result<PathBuf, RootError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(RootError::Empty);
        }
        let expanded = self.expand(input)?;
        if !expanded.is_absolute() {
            return Err(RootError::NotAbsolute);
        }
        let path = normalize(&expanded);
        self.check_allowed(&path)?;
        if path.exists() && !path.is_dir() {
            return Err(RootError::NotADirectory);
        }
        fs::create_dir_all(&path).map_err(|e| RootError::Io(e.to_string()))?;
        let canonical = fs::canonicalize(&path).map_err(|e| RootError::Io(e.to_string()))?;
        // Symlinks can lead somewhere forbidden; judge where it really lands.
        self.check_allowed(&canonical)?;
        if !is_writable(&canonical) {
            return Err(RootError::NotWritable);
        }
        // Keep a path usable for IO but without the Windows `\\?\` prefix.
        Ok(PathBuf::from(display_path(&canonical)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, RootValidator) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home/mauricio");
        fs::create_dir_all(&home).unwrap();
        let canonical_home = fs::canonicalize(&home).unwrap();
        let v = RootValidator::new(Some(canonical_home.clone()));
        (tmp, canonical_home, v)
    }

    #[test]
    fn empty_and_relative_paths_are_rejected() {
        let (_t, _h, v) = fixture();
        assert_eq!(v.validate(""), Err(RootError::Empty));
        assert_eq!(v.validate("   "), Err(RootError::Empty));
        for rel in ["cursos", "./cursos", "a/b", "~someone/x"] {
            assert_eq!(v.validate(rel), Err(RootError::NotAbsolute), "{rel}");
        }
        assert!(!Path::new("cursos").exists());
    }

    #[test]
    fn tilde_expands_to_a_home_subfolder_and_creates_it() {
        let (_t, home, v) = fixture();
        let got = v.validate("~/Videos/Lumen").unwrap();
        assert_eq!(got, home.join("Videos/Lumen"));
        assert!(got.is_dir());
        assert!(matches!(v.validate("~"), Err(RootError::Forbidden(_))));
        let none = RootValidator::new(None);
        assert_eq!(none.validate("~/x"), Err(RootError::NotAbsolute));
    }

    #[test]
    fn dangerous_roots_are_rejected() {
        let (t, home, v) = fixture();
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        assert!(matches!(v.validate(root), Err(RootError::Forbidden(_))));
        assert!(matches!(
            v.validate(home.to_str().unwrap()),
            Err(RootError::Forbidden(_))
        ));
        let parent = home.parent().unwrap();
        assert!(matches!(
            v.validate(parent.to_str().unwrap()),
            Err(RootError::Forbidden(_))
        ));
        assert!(matches!(
            v.validate(t.path().to_str().unwrap()),
            Err(RootError::Forbidden(_))
        ));
        for sys in ["/etc", "/usr/share/x", "/bin"] {
            if !cfg!(windows) {
                assert!(
                    matches!(v.validate(sys), Err(RootError::Forbidden(_))),
                    "{sys}"
                );
            }
        }
        // `..` cannot be used to reach a forbidden place.
        let sneaky = format!("{}/../../../../etc/x", home.display());
        assert!(matches!(v.validate(&sneaky), Err(RootError::Forbidden(_))));
        assert!(matches!(
            v.validate("/usr/../"),
            Err(RootError::Forbidden(_))
        ));
    }

    #[test]
    fn a_file_is_not_a_library_and_is_left_intact() {
        let (_t, home, v) = fixture();
        let file = home.join("notes.txt");
        fs::write(&file, "x").unwrap();
        assert_eq!(
            v.validate(file.to_str().unwrap()),
            Err(RootError::NotADirectory)
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "x");
    }

    #[test]
    fn missing_nested_folder_is_created() {
        let (_t, home, v) = fixture();
        let want = home.join("a/b/c");
        let got = v.validate(want.to_str().unwrap()).unwrap();
        assert_eq!(got, want);
        assert!(want.is_dir());
        let leftovers: Vec<_> = fs::read_dir(&want).unwrap().collect();
        assert!(leftovers.is_empty(), "the write probe must be removed");
    }

    #[test]
    fn trailing_separators_and_dots_are_normalised() {
        let (_t, home, v) = fixture();
        let want = home.join("Cursos");
        let a = v.validate(&format!("{}///", want.display())).unwrap();
        let b = v.validate(&format!("{}/./", want.display())).unwrap();
        assert_eq!(a, want);
        assert_eq!(b, want);
    }

    #[test]
    fn unicode_and_accented_names_work() {
        let (_t, home, v) = fixture();
        let want = home.join("Águila ñandú 日本 áéíóú");
        let got = v.validate(want.to_str().unwrap()).unwrap();
        assert_eq!(got, want);
        assert!(display_path(&got).contains("ñandú"));
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_folders_are_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let (_t, home, v) = fixture();
        let ro = home.join("ro");
        fs::create_dir_all(&ro).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
        if is_writable(&ro) {
            return; // running as root: permissions do not apply
        }
        assert_eq!(
            v.validate(ro.to_str().unwrap()),
            Err(RootError::NotWritable)
        );
        assert!(matches!(
            v.validate(ro.join("sub").to_str().unwrap()),
            Err(RootError::Io(_))
        ));
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_a_forbidden_place_is_rejected() {
        let (_t, home, v) = fixture();
        let link = home.join("sneaky");
        std::os::unix::fs::symlink("/usr", &link).unwrap();
        assert!(matches!(
            v.validate(link.to_str().unwrap()),
            Err(RootError::Forbidden(_))
        ));
    }

    #[test]
    fn verbatim_prefixes_are_stripped_for_display() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\ñ\Lumen"), r"C:\Users\ñ\Lumen");
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
        assert_eq!(strip_verbatim("/home/ñ"), "/home/ñ");
        assert_eq!(strip_verbatim(r"C:\plain"), r"C:\plain");
    }

    #[test]
    fn summary_only_reports_platform_folders() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            summarize(tmp.path()),
            LibrarySummary {
                platforms: vec![],
                has_library: false
            }
        );
        fs::create_dir_all(tmp.path().join("Hotmart")).unwrap();
        fs::create_dir_all(tmp.path().join("Other")).unwrap();
        let s = summarize(tmp.path());
        assert_eq!(s.platforms, vec!["Hotmart"]);
        assert!(s.has_library);
        fs::create_dir_all(tmp.path().join("Skool")).unwrap();
        assert_eq!(summarize(tmp.path()).platforms, vec!["Skool", "Hotmart"]);
        assert!(!summarize(&tmp.path().join("missing")).has_library);
    }

    #[test]
    fn info_reports_existence_and_writability() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = RootInfo::describe(Some(tmp.path()), RootSource::Saved);
        assert!(ok.exists && ok.writable);
        let missing = RootInfo::describe(Some(&tmp.path().join("nope")), RootSource::Default);
        assert!(!missing.exists && !missing.writable);
        let none = RootInfo::describe(None, RootSource::Default);
        assert_eq!(none.path, None);
        assert_eq!(serde_json::to_value(&ok).unwrap()["source"], "saved");
    }
}
