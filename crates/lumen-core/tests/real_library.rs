#![allow(clippy::expect_used)]

//! Opt-in check against a real downloaded library (read-only).
//! Run with: `cargo test -p lumen-core --test real_library -- --ignored`
//! Override the location with `LUMEN_SAMPLE_LIBRARY` (default: `$HOME/Vídeos/Lumen`).
//! Name the Skool group folder to scan with `LUMEN_SAMPLE_GROUP`.

use std::path::PathBuf;

use lumen_core::{GroupStatus, LibraryScanner};

fn sample_root() -> PathBuf {
    std::env::var_os("LUMEN_SAMPLE_LIBRARY")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Vídeos/Lumen")))
        .expect("HOME or LUMEN_SAMPLE_LIBRARY must be set")
}

#[test]
#[ignore = "needs a real downloaded library on disk"]
fn scans_sample_group() {
    let root = sample_root();
    let scanner = LibraryScanner::new(&root);
    let name = std::env::var("LUMEN_SAMPLE_GROUP").expect("LUMEN_SAMPLE_GROUP must name a group");
    let group = scanner.scan_group(&name).expect("sample library");
    assert_eq!(group.status, GroupStatus::Complete);
    assert!(group.courses.len() >= 11, "{} courses", group.courses.len());
    assert_eq!(group.courses[0].index, 1);
    let lessons: Vec<_> = group
        .courses
        .iter()
        .flat_map(|c| &c.sections)
        .flat_map(|s| &s.lessons)
        .collect();
    assert!(lessons.len() > 50, "{} lessons", lessons.len());
    assert!(lessons.iter().any(|l| l.has_video));
    for l in &lessons {
        for p in l.video.iter().chain(l.description.iter()) {
            assert!(!p.contains('\\'));
            assert!(root.join(p).is_file(), "missing {p}");
        }
        for r in &l.resources {
            assert!(root.join(&r.path).is_file(), "missing {}", r.path);
        }
    }
    assert!(lessons
        .iter()
        .flat_map(|l| &l.resources)
        .any(|r| r.url.is_some()));
}
