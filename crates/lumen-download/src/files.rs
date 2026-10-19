//! Small file-system helpers for lesson folders.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

const VIDEO_EXTS: [&str; 3] = ["mp4", "mkv", "webm"];

/// True when a finished file named `<base>.<ext>` exists (not `.part`/`.txt`).
pub fn finished_file_exists(base: &Path) -> bool {
    let (Some(dir), Some(name)) = (base.parent(), base.file_name()) else {
        return false;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let file = e.file_name().to_string_lossy().into_owned();
        file.strip_prefix(&prefix).is_some_and(|ext| {
            let ext = ext.to_ascii_lowercase();
            !ext.contains('.') && ext != "part" && ext != "txt"
        })
    })
}

/// True when the lesson folder already holds a video the scanner will find.
pub fn video_exists(lesson_dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(lesson_dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.path()
            .extension()
            .map(|x| x.to_string_lossy().to_ascii_lowercase())
            .is_some_and(|x| VIDEO_EXTS.contains(&x.as_str()))
    })
}

/// Writes `<name>.txt` (title, then URL) unless it already exists.
pub fn write_link_file(dir: &Path, name: &str, title: &str, url: &str) -> std::io::Result<()> {
    let path = dir.join(format!("{name}.txt"));
    if path.exists() {
        return Ok(());
    }
    fs::write(path, format!("{title}\n{url}\n"))
}

/// Makes names unique within one lesson (`Name`, `Name (2)`, ...), ignoring case.
#[derive(Debug, Default)]
pub struct UniqueNames {
    used: HashSet<String>,
}

impl UniqueNames {
    pub fn claim(&mut self, name: &str) -> String {
        let mut candidate = name.to_string();
        let mut n = 2;
        while !self.used.insert(candidate.to_lowercase()) {
            candidate = format!("{name} ({n})");
            n += 1;
        }
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), "x").unwrap();
    }

    #[test]
    fn finished_files_match_the_exact_base_name() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("Plan v1");
        assert!(!finished_file_exists(&base));
        touch(tmp.path(), "Plan v1.part");
        touch(tmp.path(), "Plan v1.txt");
        touch(tmp.path(), "Plan v1.2.pdf");
        assert!(!finished_file_exists(&base));
        touch(tmp.path(), "Plan v1.xlsx");
        assert!(finished_file_exists(&base));
        assert!(!finished_file_exists(&tmp.path().join("missing/dir/x")));
    }

    #[test]
    fn video_detection_uses_scanner_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!video_exists(tmp.path()));
        touch(tmp.path(), "a.mp4.part");
        touch(tmp.path(), "notes.txt");
        assert!(!video_exists(tmp.path()));
        touch(tmp.path(), "A.MKV");
        assert!(video_exists(tmp.path()));
    }

    #[test]
    fn link_files_hold_title_then_url_and_are_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        write_link_file(tmp.path(), "Doc", "My doc", "https://x.test/d?a=1").unwrap();
        write_link_file(tmp.path(), "Doc", "Other", "https://y.test").unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join("Doc.txt")).unwrap(),
            "My doc\nhttps://x.test/d?a=1\n"
        );
    }

    #[test]
    fn unique_names_add_a_counter() {
        let mut u = UniqueNames::default();
        assert_eq!(u.claim("Guide"), "Guide");
        assert_eq!(u.claim("guide"), "guide (2)");
        assert_eq!(u.claim("Guide"), "Guide (3)");
        assert_eq!(u.claim("Other"), "Other");
    }
}
