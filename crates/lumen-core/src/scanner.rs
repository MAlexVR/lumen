//! Read-only scanner for the on-disk library layout:
//! `<root>/Skool/<group>/<NN - course>/<NN - section>/<NN - lesson>/` and the
//! Hotmart layout handled by [`crate::hotmart_scan`].

use std::fs::{self, DirEntry};
use std::path::{Path, PathBuf};

use crate::error::ScanError;
use crate::model::{
    lesson_key, Course, Group, GroupStatus, Lesson, Library, LibraryPlatform, PlatformLibrary,
    Resource, ResourceKind, Section,
};

const VIDEO_EXTS: [&str; 3] = ["mp4", "mkv", "webm"];
/// Partial or temporary files left by interrupted downloads.
const TEMP_EXTS: [&str; 4] = ["part", "ytdl", "tmp", "temp"];
const DESCRIPTION_FILE: &str = "description.html";
const READING_FILE: &str = "supplementary reading.html";
const DRM_PREFIX: &str = "drm_";
const REPORT_FILE: &str = "download-report.json";
pub(crate) const COMPLETE_MARKER: &str = ".complete";
const RESOURCE_DIRS: [&str; 3] = ["attachments", "resources", "materials"];
const MAX_LINK_FILE_SIZE: u64 = 2048;

pub struct LibraryScanner {
    root: PathBuf,
}

impl LibraryScanner {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn platform_root(&self, platform: LibraryPlatform) -> PathBuf {
        self.root.join(platform.dir_name())
    }

    /// Scans every platform folder that exists (unknown folders are ignored).
    /// A root without any platform folder yields an empty library.
    pub fn scan(&self) -> Result<Library, ScanError> {
        let mut library = Library::default();
        for platform in LibraryPlatform::ALL {
            let dir = self.platform_root(platform);
            if !fs::symlink_metadata(&dir)
                .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
            {
                continue;
            }
            let mut groups = Vec::new();
            for entry in sorted_entries(&dir) {
                if is_visible_dir(&entry) {
                    groups.push(self.scan_platform_group(platform, &name_of(&entry))?);
                }
            }
            if platform == LibraryPlatform::Skool {
                library.groups.clone_from(&groups);
            }
            library.platforms.push(PlatformLibrary {
                id: platform,
                groups,
            });
        }
        Ok(library)
    }

    /// Scans one Skool group by folder name (no path separators allowed).
    pub fn scan_group(&self, group: &str) -> Result<Group, ScanError> {
        self.scan_platform_group(LibraryPlatform::Skool, group)
    }

    /// Scans one group of `platform` by folder name.
    pub fn scan_platform_group(
        &self,
        platform: LibraryPlatform,
        group: &str,
    ) -> Result<Group, ScanError> {
        let dir = self.group_dir(platform, group)?;
        match platform {
            LibraryPlatform::Skool => Ok(scan_skool_group(&dir, group)),
            LibraryPlatform::Hotmart => Ok(crate::hotmart_scan::scan_group(&dir, group)),
            LibraryPlatform::Youtube => Ok(crate::youtube_scan::scan_group(&dir, group)),
            LibraryPlatform::Pluralsight => Ok(crate::pluralsight_scan::scan_group(&dir, group)),
            LibraryPlatform::Goldenlife => Ok(crate::hotmart_scan::scan_group_of(
                LibraryPlatform::Goldenlife,
                &dir,
                group,
            )),
        }
    }

    fn group_dir(&self, platform: LibraryPlatform, group: &str) -> Result<PathBuf, ScanError> {
        if group.is_empty() || group == "." || group == ".." || group.contains(['/', '\\', '\0']) {
            return Err(ScanError::InvalidGroup);
        }
        let dir = self.platform_root(platform).join(group);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() && !m.file_type().is_symlink() => Ok(dir),
            Ok(_) => Err(ScanError::GroupNotFound),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ScanError::GroupNotFound),
            Err(source) => Err(ScanError::Io { path: dir, source }),
        }
    }
}

fn scan_skool_group(dir: &Path, group: &str) -> Group {
    let status = group_status(dir);
    let mut courses = Vec::new();
    for ce in ordered_subdirs(dir) {
        let cname = name_of(&ce);
        let (ctitle, cidx) = split_order_prefix(&cname);
        let mut sections = Vec::new();
        for se in ordered_subdirs(&ce.path()) {
            let sname = name_of(&se);
            let (stitle, sidx) = split_order_prefix(&sname);
            let mut lessons = Vec::new();
            for le in ordered_subdirs(&se.path()) {
                let lname = name_of(&le);
                let rel = [
                    LibraryPlatform::Skool.dir_name(),
                    group,
                    &cname,
                    &sname,
                    &lname,
                ]
                .join("/");
                let (ltitle, lidx) = split_order_prefix(&lname);
                let key = lesson_key((cidx, ctitle), (sidx, stitle), (lidx, ltitle));
                lessons.push(build_lesson(&le.path(), &rel, ltitle, lidx, key));
            }
            sections.push(Section {
                title: stitle.to_string(),
                index: sidx,
                lessons,
            });
        }
        courses.push(Course {
            title: ctitle.to_string(),
            index: cidx,
            sections,
        });
    }
    Group {
        id: group.to_string(),
        platform: LibraryPlatform::Skool,
        name: group.to_string(),
        title: group.to_string(),
        seller: None,
        status,
        lesson_count: Group::count_lessons(&courses),
        courses,
        collection: None,
    }
}

/// `.complete` marks a finished download; anything else is partial.
pub(crate) fn group_status(dir: &Path) -> GroupStatus {
    if dir.join(COMPLETE_MARKER).exists() {
        GroupStatus::Complete
    } else {
        GroupStatus::Partial
    }
}
/// Splits `NN - Title` into the title and NN (0 when there is no prefix).
pub fn split_order_prefix(name: &str) -> (&str, u64) {
    if let Some((num, title)) = name.split_once(" - ") {
        if !title.is_empty() && !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) {
            return (title, num.parse().unwrap_or(0));
        }
    }
    (name, 0)
}

pub(crate) fn name_of(e: &DirEntry) -> String {
    e.file_name().to_string_lossy().into_owned()
}

pub(crate) fn is_hidden(e: &DirEntry) -> bool {
    e.file_name().to_string_lossy().starts_with('.')
}

pub(crate) fn is_visible_dir(e: &DirEntry) -> bool {
    !is_hidden(e) && e.file_type().is_ok_and(|t| t.is_dir())
}

/// Entries of `dir` sorted by file name; unreadable directories yield nothing.
pub(crate) fn sorted_entries(dir: &Path) -> Vec<DirEntry> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<DirEntry> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(DirEntry::file_name);
    entries
}

/// Visible subdirectories sorted by numeric prefix, then name.
pub(crate) fn ordered_subdirs(dir: &Path) -> Vec<DirEntry> {
    let mut dirs: Vec<DirEntry> = sorted_entries(dir)
        .into_iter()
        .filter(is_visible_dir)
        .collect();
    dirs.sort_by_cached_key(|e| (split_order_prefix(&name_of(e)).1, e.file_name()));
    dirs
}

pub(crate) fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Leading ASCII digits of a name (0 when there are none).
pub(crate) fn leading_number(name: &str) -> u64 {
    let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// Partial or temporary download leftovers, including yt-dlp `.part-FragN`.
pub(crate) fn is_temp(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let ext = extension(name);
    TEMP_EXTS.contains(&ext.as_str()) || ext.starts_with("part") || lower.contains(".part-frag")
}

pub(crate) fn is_video(name: &str) -> bool {
    VIDEO_EXTS.contains(&extension(name).as_str())
}

pub(crate) fn build_lesson(dir: &Path, rel: &str, title: &str, index: u64, key: String) -> Lesson {
    let mut lesson = Lesson {
        key,
        title: title.to_string(),
        index,
        video: None,
        description: None,
        has_video: false,
        thumbnail: None,
        resources: Vec::new(),
        dir: Some(rel.to_string()),
    };
    let mut files = sorted_entries(dir);
    // Numbered files first (by N), `N. Lesson.mp4` before `N. Lesson [id].mp4`;
    // unnumbered ones follow (0 wraps to the largest key).
    files.sort_by_cached_key(|e| {
        let n = name_of(e);
        (leading_number(&n).wrapping_sub(1), n.contains('['), n)
    });
    let mut subdirs = Vec::new();
    for e in files {
        let name = name_of(&e);
        if name.starts_with('.') {
            continue;
        }
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            if RESOURCE_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
                subdirs.push(e);
            }
            continue;
        }
        if is_temp(&name) || name == REPORT_FILE {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if lower == DESCRIPTION_FILE && lesson.description.is_none() {
            lesson.description = Some(format!("{rel}/{name}"));
            continue;
        }
        if is_video(&name) {
            let mut res = resource(&e.path(), rel, &name);
            if lesson.video.is_none() {
                lesson.video = Some(res.path);
                lesson.has_video = true;
            } else {
                res.kind = ResourceKind::Video;
                lesson.resources.push(res);
            }
            continue;
        }
        let mut res = resource(&e.path(), rel, &name);
        if lower == READING_FILE {
            res.kind = ResourceKind::Reading;
        }
        lesson.resources.push(res);
    }
    for d in subdirs {
        collect_resources(
            &d.path(),
            &format!("{rel}/{}", name_of(&d)),
            &mut lesson.resources,
        );
    }
    lesson
}

fn collect_resources(dir: &Path, rel: &str, out: &mut Vec<Resource>) {
    for e in sorted_entries(dir) {
        let name = name_of(&e);
        if name.starts_with('.') {
            continue;
        }
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            collect_resources(&e.path(), &format!("{rel}/{name}"), out);
        } else if !is_temp(&name) {
            out.push(resource(&e.path(), rel, &name));
        }
    }
}

fn resource(path: &Path, rel: &str, name: &str) -> Resource {
    let mut res = Resource {
        name: name.to_string(),
        path: format!("{rel}/{name}"),
        url: None,
        kind: ResourceKind::File,
    };
    if let Some((head, rest)) = name.split_at_checked(DRM_PREFIX.len()) {
        if !rest.is_empty() && head.eq_ignore_ascii_case(DRM_PREFIX) {
            res.kind = ResourceKind::Drm;
            res.name = rest.to_string();
        }
    }
    if extension(name) == "txt" {
        if let Some((title, url)) = read_link_file(path) {
            res.url = Some(url);
            res.kind = ResourceKind::Link;
            if !title.is_empty() {
                res.name = title;
            }
        }
    }
    res
}

/// Parses the `title\nurl` files written for external links.
fn read_link_file(path: &Path) -> Option<(String, String)> {
    let meta = fs::metadata(path).ok()?;
    if meta.len() > MAX_LINK_FILE_SIZE {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    let mut lines = text.split('\n');
    let title = lines.next()?.trim();
    let url = lines.next()?.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        Some((title.to_string(), url.to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn put(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    fn scanner(tmp: &TempDir) -> LibraryScanner {
        LibraryScanner::new(tmp.path())
    }

    const L: &str = "Skool/G/01 - C/01 - S/01 - Intro";

    fn lesson(tmp: &TempDir) -> Lesson {
        let g = scanner(tmp).scan_group("G").unwrap();
        g.courses[0].sections[0].lessons[0].clone()
    }

    #[test]
    fn split_prefix_extracts_number_and_title() {
        assert_eq!(split_order_prefix("07 - Hola - mundo"), ("Hola - mundo", 7));
        assert_eq!(split_order_prefix("Sin prefijo"), ("Sin prefijo", 0));
        assert_eq!(split_order_prefix("12 -x"), ("12 -x", 0));
        assert_eq!(
            split_order_prefix("99999999999999999999999 - Big"),
            ("Big", 0)
        );
    }

    #[test]
    fn missing_skool_folder_is_empty_library() {
        let tmp = TempDir::new().unwrap();
        assert!(scanner(&tmp).scan().unwrap().groups.is_empty());
    }

    #[test]
    fn groups_are_sorted_with_status_and_hidden_ignored() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Skool/b/.complete", "done");
        put(tmp.path(), "Skool/b/download-report.json", "{}");
        put(tmp.path(), "Skool/a/01 - C/x.txt", "");
        put(tmp.path(), "Skool/.hidden/x", "");
        put(tmp.path(), "Skool/loose-file.txt", "");
        let lib = scanner(&tmp).scan().unwrap();
        let got: Vec<_> = lib
            .groups
            .iter()
            .map(|g| (g.name.as_str(), g.status))
            .collect();
        assert_eq!(
            got,
            vec![("a", GroupStatus::Partial), ("b", GroupStatus::Complete)]
        );
        assert!(lib.groups[1].courses.is_empty());
    }

    #[test]
    fn numeric_prefix_orders_before_name() {
        let tmp = TempDir::new().unwrap();
        for c in ["10 - Ten", "2 - Two", "1 - One", "Zeta", "Alpha"] {
            put(
                tmp.path(),
                &format!("Skool/G/{c}/01 - S/01 - L/description.html"),
                "x",
            );
        }
        let g = scanner(&tmp).scan_group("G").unwrap();
        let got: Vec<_> = g
            .courses
            .iter()
            .map(|c| (c.index, c.title.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, "Alpha"),
                (0, "Zeta"),
                (1, "One"),
                (2, "Two"),
                (10, "Ten")
            ]
        );
    }

    #[test]
    fn lesson_detects_video_description_and_paths() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), &format!("{L}/Intro.mp4"), "v");
        put(tmp.path(), &format!("{L}/description.html"), "<p>x</p>");
        let l = lesson(&tmp);
        assert_eq!(l.title, "Intro");
        assert_eq!(l.index, 1);
        assert!(l.has_video);
        assert_eq!(
            l.video.as_deref(),
            Some("Skool/G/01 - C/01 - S/01 - Intro/Intro.mp4")
        );
        assert_eq!(
            l.description.as_deref(),
            Some("Skool/G/01 - C/01 - S/01 - Intro/description.html")
        );
        assert!(l.resources.is_empty());
        assert_eq!(l.key, "1-C/1-S/1-Intro");
    }

    #[test]
    fn video_extension_is_case_insensitive_and_first_video_wins() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), &format!("{L}/a.MKV"), "v");
        put(tmp.path(), &format!("{L}/b.webm"), "v");
        let l = lesson(&tmp);
        assert!(l.video.as_deref().unwrap().ends_with("/a.MKV"));
        assert_eq!(l.resources.len(), 1);
        assert_eq!(l.resources[0].name, "b.webm");
    }

    #[test]
    fn lesson_without_video_has_no_video() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), &format!("{L}/notes.pdf"), "p");
        let l = lesson(&tmp);
        assert!(!l.has_video);
        assert_eq!(l.video, None);
        assert_eq!(l.description, None);
        assert_eq!(l.resources[0].name, "notes.pdf");
    }

    #[test]
    fn hidden_temp_and_report_files_are_ignored() {
        let tmp = TempDir::new().unwrap();
        for f in [
            ".DS_Store",
            ".complete",
            "v.mp4.part",
            "v.ytdl",
            "x.tmp",
            "y.TEMP",
            "download-report.json",
        ] {
            put(tmp.path(), &format!("{L}/{f}"), "x");
        }
        put(tmp.path(), &format!("{L}/keep.pdf"), "x");
        let l = lesson(&tmp);
        let names: Vec<_> = l.resources.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["keep.pdf"]);
    }

    #[test]
    fn link_txt_files_expose_url_and_title() {
        let tmp = TempDir::new().unwrap();
        put(
            tmp.path(),
            &format!("{L}/link.txt"),
            "Mi enlace\r\nhttps://example.com/x\r\n",
        );
        put(
            tmp.path(),
            &format!("{L}/untitled.txt"),
            "\nhttp://example.com\n",
        );
        put(
            tmp.path(),
            &format!("{L}/notes.txt"),
            "just notes\nnot a url\n",
        );
        put(
            tmp.path(),
            &format!("{L}/one-line.txt"),
            "https://example.com",
        );
        put(
            tmp.path(),
            &format!("{L}/huge.txt"),
            &format!("t\nhttps://example.com/{}", "a".repeat(3000)),
        );
        let l = lesson(&tmp);
        let by = |n: &str| l.resources.iter().find(|r| r.path.ends_with(n)).unwrap();
        assert_eq!(
            by("/link.txt").url.as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(by("/link.txt").name, "Mi enlace");
        assert_eq!(
            by("/untitled.txt").url.as_deref(),
            Some("http://example.com")
        );
        assert_eq!(by("/untitled.txt").name, "untitled.txt");
        assert_eq!(by("/notes.txt").url, None);
        assert_eq!(by("/one-line.txt").url, None);
        assert_eq!(by("/huge.txt").url, None);
    }

    #[test]
    fn attachments_and_resources_folders_are_listed() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), &format!("{L}/Attachments/guide.pdf"), "p");
        put(tmp.path(), &format!("{L}/resources/sub/deep.zip"), "z");
        put(tmp.path(), &format!("{L}/resources/.hidden"), "z");
        put(tmp.path(), &format!("{L}/other/ignored.pdf"), "z");
        let l = lesson(&tmp);
        let paths: Vec<_> = l.resources.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "Skool/G/01 - C/01 - S/01 - Intro/Attachments/guide.pdf",
                "Skool/G/01 - C/01 - S/01 - Intro/resources/sub/deep.zip",
            ]
        );
        assert_eq!(l.resources[0].name, "guide.pdf");
        assert!(l.resources.iter().all(|r| !r.path.contains('\\')));
    }

    #[test]
    fn invalid_group_names_are_rejected() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Skool/G/01 - C/x", "");
        for g in ["", ".", "..", "../G", "G/..", "a/b", "a\\b"] {
            assert!(
                matches!(scanner(&tmp).scan_group(g), Err(ScanError::InvalidGroup)),
                "{g:?}"
            );
        }
        assert!(matches!(
            scanner(&tmp).scan_group("nope"),
            Err(ScanError::GroupNotFound)
        ));
    }

    #[test]
    fn pluralsight_library_is_listed_without_changing_legacy_groups() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Skool/S/01 - C/01 - Sec/01 - L/a.mp4", "v");
        let course = "Rust Course-0123456789abcdef";
        let clip = "01 - Ownership-abcdef0123456789";
        put(
            tmp.path(),
            &format!("Pluralsight/{course}/01 - Foundations/{clip}/video.mp4"),
            "video",
        );
        put(
            tmp.path(),
            &format!("Pluralsight/{course}/01 - Foundations/{clip}/description.html"),
            "description",
        );

        let library = LibraryScanner::new(tmp.path()).scan().unwrap();
        let json = serde_json::to_value(&library).unwrap();
        let platform = json["platforms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|platform| platform["id"] == "pluralsight")
            .expect("existing Pluralsight directory appears in the platform list");
        assert_eq!(library.groups.len(), 1);
        assert_eq!(library.groups[0].platform, LibraryPlatform::Skool);
        assert_eq!(platform["groups"][0]["id"], format!("pluralsight/{course}"));
        assert_eq!(platform["groups"][0]["name"], course);
        assert_eq!(platform["groups"][0]["title"], "Rust Course");
        let lesson = &platform["groups"][0]["courses"][0]["sections"][0]["lessons"][0];
        assert_eq!(
            lesson["key"],
            format!("pluralsight:{course}/01 - Foundations/{clip}")
        );
        assert_eq!(lesson["title"], "Ownership");
        assert_eq!(
            lesson["video"],
            format!("Pluralsight/{course}/01 - Foundations/{clip}/video.mp4")
        );
        assert_eq!(
            lesson["description"],
            format!("Pluralsight/{course}/01 - Foundations/{clip}/description.html")
        );
        assert_eq!(lesson["hasVideo"], true);
    }

    #[test]
    fn goldenlife_library_uses_the_hotmart_layout_with_its_own_ids() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Skool/S/01 - C/01 - Sec/01 - L/a.mp4", "v");
        let course = "Meta Ads Basico";
        let lesson = "2. Primeras campañas";
        put(
            tmp.path(),
            &format!("GoldenLife/{course}/1. Fundamentos/{lesson}/2. Lesson.mp4"),
            "video",
        );
        put(
            tmp.path(),
            &format!("GoldenLife/{course}/1. Fundamentos/{lesson}/Description.html"),
            "description",
        );
        put(
            tmp.path(),
            &format!("GoldenLife/{course}/1. Fundamentos/{lesson}/Materials/guia.pdf"),
            "pdf",
        );

        let scanner = LibraryScanner::new(tmp.path());
        let library = scanner.scan().unwrap();
        let platform = library
            .platform(LibraryPlatform::Goldenlife)
            .expect("an existing GoldenLife directory appears in the platform list");
        assert_eq!(library.groups.len(), 1, "legacy Skool groups are unchanged");
        let group = &platform.groups[0];
        assert_eq!(group.id, format!("goldenlife/{course}"));
        assert_eq!(group.platform, LibraryPlatform::Goldenlife);
        assert_eq!(group.title, course);
        assert_eq!(group.seller, None);
        let l = &group.courses[0].sections[0].lessons[0];
        assert_eq!(
            l.key,
            format!("goldenlife:1-{course}/1-Fundamentos/2-Primeras campañas")
        );
        assert_eq!(
            l.video.as_deref(),
            Some(format!("GoldenLife/{course}/1. Fundamentos/{lesson}/2. Lesson.mp4").as_str())
        );
        assert!(l.description.is_some());
        assert_eq!(l.resources[0].name, "guia.pdf");

        let json = serde_json::to_value(&library).unwrap();
        assert!(json["platforms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "goldenlife"));
        assert_eq!(
            scanner
                .scan_platform_group(LibraryPlatform::Goldenlife, course)
                .unwrap()
                .id,
            group.id
        );
    }

    #[test]
    fn json_uses_camel_case() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), &format!("{L}/a.mp4"), "v");
        let json = serde_json::to_value(scanner(&tmp).scan().unwrap()).unwrap();
        let g = &json["groups"][0];
        assert_eq!(g["status"], "partial");
        assert_eq!(
            g["courses"][0]["sections"][0]["lessons"][0]["hasVideo"],
            true
        );
    }
}
