//! Hotmart on-disk layout (written by `lumen-hotmart` and by the previous Go implementation):
//! `Hotmart/<name> - <seller>/<N. module>/<N. lesson>/` with
//! `N. Lesson.mp4`, `Description.html`, `Supplementary reading.html` and
//! `Materials/`. Video files placed directly in a module folder are also read
//! as lessons.
//!
//! A purchased Hotmart course is ONE course: the tree stays uniform
//! (`Group > Course > Section > Lesson`) with the single course mirroring the
//! group title and every module mapped to a section.

use std::fs::DirEntry;
use std::path::Path;

use crate::model::{hotmart_lesson_key, Course, Group, Lesson, LibraryPlatform, Section};
use crate::scanner::{
    build_lesson, extension, group_status, is_hidden, is_temp, is_video, leading_number, name_of,
    sorted_entries,
};

const SELLER_SEPARATOR: &str = " - ";

/// Splits `<name> - <seller>` on the last separator; the whole name is the
/// title when there is no (non-empty) seller part.
fn split_title_seller(folder: &str) -> (&str, Option<&str>) {
    match folder.rsplit_once(SELLER_SEPARATOR) {
        Some((title, seller)) if !title.trim().is_empty() && !seller.trim().is_empty() => {
            (title, Some(seller))
        }
        _ => (folder, None),
    }
}

/// Splits `N. Title` into the title and N (0 when there is no prefix).
fn split_dot_prefix(name: &str) -> (&str, u64) {
    if let Some((num, title)) = name.split_once(". ") {
        if !title.is_empty() && !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) {
            return (title, num.parse().unwrap_or(0));
        }
    }
    (name, 0)
}

fn visible_dirs(dir: &Path) -> Vec<DirEntry> {
    let mut dirs: Vec<DirEntry> = sorted_entries(dir)
        .into_iter()
        .filter(|e| !is_hidden(e) && e.file_type().is_ok_and(|t| t.is_dir()))
        .collect();
    dirs.sort_by_cached_key(|e| (leading_number(&name_of(e)), name_of(e)));
    dirs
}

pub(crate) fn scan_group(dir: &Path, folder: &str) -> Group {
    let (title, seller) = split_title_seller(folder);
    let mut sections = Vec::new();
    for me in visible_dirs(dir) {
        let mname = name_of(&me);
        let (mtitle, midx) = split_dot_prefix(&mname);
        let mrel = format!("{}/{folder}/{mname}", LibraryPlatform::Hotmart.dir_name());
        let lessons = module_lessons(&me.path(), &mrel, (1, title), (midx, mtitle));
        sections.push(Section {
            title: mtitle.to_string(),
            index: midx,
            lessons,
        });
    }
    let courses = vec![Course {
        title: title.to_string(),
        index: 1,
        sections,
    }];
    Group {
        id: format!("hotmart/{folder}"),
        platform: LibraryPlatform::Hotmart,
        name: folder.to_string(),
        title: title.to_string(),
        seller: seller.map(str::to_string),
        status: group_status(dir),
        lesson_count: Group::count_lessons(&courses),
        courses,
    }
}

/// Lesson folders plus loose video files, ordered by their number then title.
fn module_lessons(dir: &Path, rel: &str, course: (u64, &str), module: (u64, &str)) -> Vec<Lesson> {
    let key = |l: (u64, &str)| hotmart_lesson_key(course, module, l);
    let mut lessons = Vec::new();
    for le in visible_dirs(dir) {
        let lname = name_of(&le);
        let (ltitle, lidx) = split_dot_prefix(&lname);
        lessons.push(build_lesson(
            &le.path(),
            &format!("{rel}/{lname}"),
            ltitle,
            lidx,
            key((lidx, ltitle)),
        ));
    }
    let entries = sorted_entries(dir);
    for e in &entries {
        let fname = name_of(e);
        if fname.starts_with('.')
            || !e.file_type().is_ok_and(|t| t.is_file())
            || is_temp(&fname)
            || !is_video(&fname)
        {
            continue;
        }
        let stem = fname
            .strip_suffix(&format!(".{}", extension(&fname)))
            .unwrap_or(&fname)
            .to_string();
        let (ltitle, lidx) = split_dot_prefix(&stem);
        let wanted = format!("{stem}.html");
        let description = entries
            .iter()
            .map(name_of)
            .find(|n| n.eq_ignore_ascii_case(&wanted))
            .map(|n| format!("{rel}/{n}"));
        lessons.push(Lesson {
            key: key((lidx, ltitle)),
            title: ltitle.to_string(),
            index: lidx,
            video: Some(format!("{rel}/{fname}")),
            description,
            has_video: true,
            resources: Vec::new(),
        });
    }
    lessons.sort_by(|a, b| (a.index, &a.title).cmp(&(b.index, &b.title)));
    lessons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GroupStatus, LibraryPlatform, ResourceKind};
    use crate::scanner::LibraryScanner;
    use std::fs;
    use tempfile::TempDir;

    fn put(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    fn group(tmp: &TempDir, folder: &str) -> Group {
        LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Hotmart, folder)
            .unwrap()
    }

    const C: &str = "Hotmart/Fotografía Nocturna - 15.1 - Estudio Lumbre SL";

    #[test]
    fn course_folder_splits_title_and_seller() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), &format!("{C}/.complete"), "done");
        put(
            tmp.path(),
            &format!("{C}/1. Mod/1. Intro/1. Lesson.mp4"),
            "v",
        );
        let g = group(&tmp, "Fotografía Nocturna - 15.1 - Estudio Lumbre SL");
        assert_eq!(g.title, "Fotografía Nocturna - 15.1");
        assert_eq!(g.seller.as_deref(), Some("Estudio Lumbre SL"));
        assert_eq!(g.platform, LibraryPlatform::Hotmart);
        assert_eq!(
            g.id,
            "hotmart/Fotografía Nocturna - 15.1 - Estudio Lumbre SL"
        );
        assert_eq!(g.name, "Fotografía Nocturna - 15.1 - Estudio Lumbre SL");
        assert_eq!(g.status, GroupStatus::Complete);
        assert_eq!(g.lesson_count, 1);
    }

    #[test]
    fn folder_without_seller_keeps_the_whole_name() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Hotmart/Solo/1. M/1. L/1. Lesson.mp4", "v");
        put(tmp.path(), "Hotmart/Trail - /1. M/1. L/x", "v");
        let g = group(&tmp, "Solo");
        assert_eq!((g.title.as_str(), g.seller.clone()), ("Solo", None));
        assert_eq!(g.status, GroupStatus::Partial);
        assert_eq!(group(&tmp, "Trail - ").seller, None);
    }

    #[test]
    fn one_course_with_modules_as_sections_in_numeric_order() {
        let tmp = TempDir::new().unwrap();
        for m in ["10. Módulo 6", "2. Informaciones", "1. Bienvenidos"] {
            put(tmp.path(), &format!("{C}/{m}/1. L/1. Lesson.mp4"), "v");
        }
        let g = group(&tmp, "Fotografía Nocturna - 15.1 - Estudio Lumbre SL");
        assert_eq!(g.courses.len(), 1);
        assert_eq!(g.courses[0].title, "Fotografía Nocturna - 15.1");
        assert_eq!(g.courses[0].index, 1);
        let got: Vec<_> = g.courses[0]
            .sections
            .iter()
            .map(|s| (s.index, s.title.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![(1, "Bienvenidos"), (2, "Informaciones"), (10, "Módulo 6")]
        );
    }

    #[test]
    fn lesson_folder_layout_video_description_materials_and_keys() {
        let tmp = TempDir::new().unwrap();
        let l = format!("{C}/3. Módulo 1 - Toma/2. Clase 3");
        put(tmp.path(), &format!("{l}/1. Lesson.mp4"), "v");
        put(tmp.path(), &format!("{l}/2. Lesson [abc].mp4"), "v");
        put(tmp.path(), &format!("{l}/Description.html"), "<p>x</p>");
        put(tmp.path(), &format!("{l}/Supplementary reading.html"), "r");
        put(tmp.path(), &format!("{l}/Materials/Plan.pdf"), "p");
        put(tmp.path(), &format!("{l}/Materials/drm_Secret.pdf"), "p");
        put(tmp.path(), &format!("{l}/.complete"), "");
        put(tmp.path(), &format!("{l}/download-report.json"), "{}");
        let g = group(&tmp, "Fotografía Nocturna - 15.1 - Estudio Lumbre SL");
        let lesson = &g.courses[0].sections[0].lessons[0];
        assert_eq!(lesson.title, "Clase 3");
        assert_eq!(lesson.index, 2);
        assert!(lesson.has_video);
        assert_eq!(
            lesson.video.as_deref(),
            Some(&*format!("{l}/1. Lesson.mp4"))
        );
        assert_eq!(
            lesson.description.as_deref(),
            Some(&*format!("{l}/Description.html"))
        );
        assert_eq!(
            lesson.key,
            "hotmart:1-Fotografía Nocturna - 15.1/3-Módulo 1 - Toma/2-Clase 3"
        );
        let got: Vec<_> = lesson
            .resources
            .iter()
            .map(|r| (r.name.as_str(), r.kind))
            .collect();
        assert_eq!(
            got,
            vec![
                ("2. Lesson [abc].mp4", ResourceKind::Video),
                ("Supplementary reading.html", ResourceKind::Reading),
                ("Plan.pdf", ResourceKind::File),
                ("Secret.pdf", ResourceKind::Drm),
            ]
        );
        assert_eq!(
            lesson.resources[3].path,
            format!("{l}/Materials/drm_Secret.pdf")
        );
    }

    #[test]
    fn short_non_ascii_material_names_do_not_panic() {
        let tmp = TempDir::new().unwrap();
        let l = format!("{C}/1. M/1. L");
        put(tmp.path(), &format!("{l}/Materials/Módulo.pdf"), "p");
        put(tmp.path(), &format!("{l}/Materials/drm_"), "p");
        let g = group(&tmp, "Fotografía Nocturna - 15.1 - Estudio Lumbre SL");
        let r = &g.courses[0].sections[0].lessons[0].resources;
        assert_eq!(r[0].kind, ResourceKind::File);
        assert_eq!(r[1].kind, ResourceKind::File);
    }

    #[test]
    fn empty_lesson_folders_and_part_files_are_handled() {
        let tmp = TempDir::new().unwrap();
        let m = format!("{C}/1. M");
        fs::create_dir_all(tmp.path().join(format!("{m}/6. Responde al Test"))).unwrap();
        put(tmp.path(), &format!("{m}/1. L/1. Lesson.mp4.part"), "x");
        put(
            tmp.path(),
            &format!("{m}/1. L/1. Lesson.mp4.part-Frag12"),
            "x",
        );
        put(
            tmp.path(),
            &format!("{m}/1. L/1. Lesson.mp4.part-Frag13.part"),
            "x",
        );
        let g = group(&tmp, "Fotografía Nocturna - 15.1 - Estudio Lumbre SL");
        let lessons = &g.courses[0].sections[0].lessons;
        assert_eq!(lessons.len(), 2);
        assert!(lessons
            .iter()
            .all(|l| !l.has_video && l.resources.is_empty()));
        assert_eq!(lessons[1].title, "Responde al Test");
    }

    #[test]
    fn loose_video_files_in_a_module_become_lessons() {
        let tmp = TempDir::new().unwrap();
        let m = "Hotmart/Older - Seller/1. Modulo";
        put(tmp.path(), &format!("{m}/2. Segunda.mp4"), "v");
        put(tmp.path(), &format!("{m}/1. Primera.mp4"), "v");
        put(tmp.path(), &format!("{m}/1. Primera.html"), "<p>d</p>");
        put(tmp.path(), &format!("{m}/10. Décima.mkv"), "v");
        put(tmp.path(), &format!("{m}/notes.pdf"), "p");
        put(tmp.path(), &format!("{m}/3. Folder/1. Lesson.mp4"), "v");
        let g = group(&tmp, "Older - Seller");
        let lessons = &g.courses[0].sections[0].lessons;
        let got: Vec<_> = lessons
            .iter()
            .map(|l| (l.index, l.title.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (1, "Primera"),
                (2, "Segunda"),
                (3, "Folder"),
                (10, "Décima")
            ]
        );
        assert_eq!(
            lessons[0].video.as_deref(),
            Some("Hotmart/Older - Seller/1. Modulo/1. Primera.mp4")
        );
        assert_eq!(
            lessons[0].description.as_deref(),
            Some("Hotmart/Older - Seller/1. Modulo/1. Primera.html")
        );
        assert!(lessons[1].description.is_none());
        assert!(lessons.iter().all(|l| l.has_video));
    }

    #[test]
    fn scan_lists_platforms_ignores_unknown_and_hidden() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Skool/S/01 - C/01 - Sec/01 - L/a.mp4", "v");
        put(tmp.path(), "Hotmart/H - Me/1. M/1. L/1. Lesson.mp4", "v");
        put(tmp.path(), "Hotmart/.hidden/x", "");
        put(tmp.path(), "Hotmart/loose.txt", "");
        put(tmp.path(), "Gumroad/G/x", "");
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        let ids: Vec<_> = lib.platforms.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![LibraryPlatform::Skool, LibraryPlatform::Hotmart]);
        assert_eq!(lib.groups.len(), 1);
        assert_eq!(lib.groups[0].id, "S");
        let hm = lib.platform(LibraryPlatform::Hotmart).unwrap();
        assert_eq!(hm.groups.len(), 1);
        assert_eq!(hm.groups[0].seller.as_deref(), Some("Me"));
    }

    #[test]
    fn empty_platform_folder_is_listed_empty_and_missing_is_absent() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("Hotmart")).unwrap();
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        assert_eq!(lib.platforms.len(), 1);
        assert_eq!(lib.platforms[0].id, LibraryPlatform::Hotmart);
        assert!(lib.platforms[0].groups.is_empty());
        assert!(lib.groups.is_empty());
    }

    #[test]
    fn keys_never_collide_across_platforms() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Skool/G/01 - G/01 - M/01 - L/a.mp4", "v");
        put(tmp.path(), "Hotmart/G/1. M/1. L/1. Lesson.mp4", "v");
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        let sk = &lib.groups[0].courses[0].sections[0].lessons[0].key;
        let hm = &lib.platform(LibraryPlatform::Hotmart).unwrap().groups[0].courses[0].sections[0]
            .lessons[0]
            .key;
        assert_eq!(sk, "1-G/1-M/1-L");
        assert_ne!(sk, hm);
        assert!(hm.starts_with("hotmart:"));
    }

    #[test]
    fn json_is_camel_case_with_platforms() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Hotmart/H - Me/1. M/1. L/1. Lesson.mp4", "v");
        let json = serde_json::to_value(LibraryScanner::new(tmp.path()).scan().unwrap()).unwrap();
        let p = &json["platforms"][0];
        assert_eq!(p["id"], "hotmart");
        let g = &p["groups"][0];
        assert_eq!(g["platform"], "hotmart");
        assert_eq!(g["lessonCount"], 1);
        assert_eq!(g["status"], "partial");
        assert_eq!(
            g["courses"][0]["sections"][0]["lessons"][0]["hasVideo"],
            true
        );
    }

    #[test]
    fn invalid_hotmart_group_names_are_rejected() {
        let tmp = TempDir::new().unwrap();
        let s = LibraryScanner::new(tmp.path());
        for g in ["", "..", "a/b", "a\\b"] {
            assert!(s.scan_platform_group(LibraryPlatform::Hotmart, g).is_err());
        }
    }

    /// Run with `cargo test -p lumen-core -- --ignored --nocapture`.
    /// Reads `$LUMEN_SAMPLE_LIBRARY` or `~/Vídeos/Lumen`; never writes.
    #[test]
    #[ignore = "needs a real library (LUMEN_SAMPLE_LIBRARY or ~/Vídeos/Lumen)"]
    fn real_library_counts() {
        let root = std::env::var_os("LUMEN_SAMPLE_LIBRARY")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join("Vídeos/Lumen")))
            .unwrap();
        let lib = LibraryScanner::new(&root).scan().unwrap();
        for p in &lib.platforms {
            let courses: usize = p.groups.iter().map(|g| g.courses.len()).sum();
            let sections: usize = p
                .groups
                .iter()
                .flat_map(|g| &g.courses)
                .map(|c| c.sections.len())
                .sum();
            let lessons: u64 = p.groups.iter().map(|g| g.lesson_count).sum();
            let videos = p
                .groups
                .iter()
                .flat_map(|g| &g.courses)
                .flat_map(|c| &c.sections)
                .flat_map(|s| &s.lessons)
                .filter(|l| l.has_video)
                .count();
            println!(
                "{:?}: groups={} courses={courses} sections={sections} lessons={lessons} videos={videos}",
                p.id,
                p.groups.len()
            );
            for g in &p.groups {
                println!(
                    "  {} | {:?} | {:?} | lessons={}",
                    g.title, g.seller, g.status, g.lesson_count
                );
            }
            for l in p
                .groups
                .iter()
                .flat_map(|g| &g.courses)
                .flat_map(|c| &c.sections)
                .flat_map(|s| &s.lessons)
                .filter(|l| l.has_video)
            {
                let v = l.video.as_deref().unwrap();
                assert!(root.join(v).is_file(), "{v}");
            }
        }
        assert!(lib
            .platforms
            .iter()
            .any(|p| p.id == LibraryPlatform::Hotmart));
    }
}
