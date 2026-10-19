use std::fs;
use std::path::{Path, PathBuf};

/// Maps a request-relative path (already percent-decoded, `/` separated) to a
/// regular file strictly inside `root`, following symlinks only when the
/// result stays inside `root`.
pub fn resolve_confined(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty()
        || rel.contains(['\0', '\\'])
        || rel.starts_with('/')
        // Drive letters and NTFS alternate data streams.
        || (cfg!(windows) && rel.contains(':'))
    {
        return None;
    }
    // Windows trims trailing dots and spaces, so ".. " and "..." mean "..".
    if rel
        .split('/')
        .any(|seg| !seg.is_empty() && seg.trim_matches(['.', ' ']).is_empty())
    {
        return None;
    }
    let real_root = fs::canonicalize(root).ok()?;
    let full = fs::canonicalize(real_root.join(rel)).ok()?;
    // `Path::starts_with` compares whole components, so `/lib-evil` is not inside `/lib`.
    if full == real_root || !full.starts_with(&real_root) {
        return None;
    }
    fs::metadata(&full).ok()?.is_file().then_some(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("lib");
        fs::create_dir_all(root.join("a b")).unwrap();
        fs::write(root.join("a b/v.mp4"), "video").unwrap();
        fs::write(tmp.path().join("secret.txt"), "secret").unwrap();
        fs::create_dir_all(tmp.path().join("lib-evil")).unwrap();
        fs::write(tmp.path().join("lib-evil/secret.txt"), "evil").unwrap();
        (tmp, root)
    }

    #[test]
    fn resolves_regular_files() {
        let (_t, root) = setup();
        let got = resolve_confined(&root, "a b/v.mp4").unwrap();
        assert!(got.ends_with("a b/v.mp4"));
    }

    #[test]
    fn rejects_traversal_and_odd_paths() {
        let (_t, root) = setup();
        for rel in [
            "",
            "/a b/v.mp4",
            "../secret.txt",
            "a b/../../secret.txt",
            "a b/..",
            "..\\secret.txt",
            "a b\\v.mp4",
            "a b/\0v.mp4",
            "a b/.. /../secret.txt",
            "a b/.../x",
            "../lib-evil/secret.txt",
        ] {
            assert!(resolve_confined(&root, rel).is_none(), "{rel:?}");
        }
    }

    #[test]
    fn rejects_directories_and_missing_files() {
        let (_t, root) = setup();
        assert!(resolve_confined(&root, "a b").is_none());
        assert!(resolve_confined(&root, "a b/missing.mp4").is_none());
        assert!(resolve_confined(&root.join("nope"), "x").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected_but_inside_link_is_allowed() {
        use std::os::unix::fs::symlink;
        let (t, root) = setup();
        symlink(t.path().join("secret.txt"), root.join("out.txt")).unwrap();
        symlink(t.path(), root.join("updir")).unwrap();
        symlink(root.join("a b/v.mp4"), root.join("in.mp4")).unwrap();
        assert!(resolve_confined(&root, "out.txt").is_none());
        assert!(resolve_confined(&root, "updir/secret.txt").is_none());
        assert!(resolve_confined(&root, "in.mp4").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn sibling_prefix_directory_is_not_inside_root() {
        use std::os::unix::fs::symlink;
        let (t, root) = setup();
        symlink(t.path().join("lib-evil"), root.join("evil")).unwrap();
        assert!(resolve_confined(&root, "evil/secret.txt").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn root_itself_may_be_a_symlink() {
        use std::os::unix::fs::symlink;
        let (t, root) = setup();
        let link = t.path().join("root-link");
        symlink(&root, &link).unwrap();
        assert!(resolve_confined(&link, "a b/v.mp4").is_some());
    }
}
