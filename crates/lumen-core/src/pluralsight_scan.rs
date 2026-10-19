//! Pluralsight on-disk layout: `Pluralsight/<course>/<module>/<clip>/video.mp4`.
//! Course and clip folder hashes are retained in identities but omitted from
//! display titles when they match the planner's exact 16-hex suffix form.

use std::path::Path;

use crate::model::{Course, Group, LibraryPlatform, Section};
use crate::scanner::{build_lesson, group_status, name_of, ordered_subdirs, split_order_prefix};

pub(crate) fn scan_group(dir: &Path, folder: &str) -> Group {
    let title = display_name(folder);
    let mut sections = Vec::new();
    for module in ordered_subdirs(dir) {
        let module_name = name_of(&module);
        let (module_title, module_index) = split_order_prefix(&module_name);
        let mut lessons = Vec::new();
        for clip in ordered_subdirs(&module.path()) {
            let clip_name = name_of(&clip);
            let (clip_title, clip_index) = split_order_prefix(&clip_name);
            let display_title = display_name(clip_title);
            let rel = format!(
                "{}/{folder}/{module_name}/{clip_name}",
                LibraryPlatform::Pluralsight.dir_name()
            );
            let key = format!("pluralsight:{folder}/{module_name}/{clip_name}");
            lessons.push(build_lesson(
                &clip.path(),
                &rel,
                &display_title,
                clip_index,
                key,
            ));
        }
        sections.push(Section {
            title: module_title.to_string(),
            index: module_index,
            lessons,
        });
    }
    let courses = vec![Course {
        title: title.clone(),
        index: 1,
        sections,
    }];
    Group {
        id: format!("pluralsight/{folder}"),
        platform: LibraryPlatform::Pluralsight,
        name: folder.to_string(),
        title,
        seller: None,
        status: group_status(dir),
        lesson_count: Group::count_lessons(&courses),
        courses,
        collection: None,
    }
}

fn display_name(name: &str) -> String {
    let Some((title, suffix)) = name.rsplit_once('-') else {
        return name.to_string();
    };
    if suffix.len() == 16
        && suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && !title.is_empty()
    {
        title.to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn put(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn missing_root_is_not_listed_and_existing_empty_root_is_listed() {
        let tmp = TempDir::new().unwrap();
        let scanner = crate::LibraryScanner::new(tmp.path());
        assert!(scanner
            .scan()
            .unwrap()
            .platform(LibraryPlatform::Pluralsight)
            .is_none());
        fs::create_dir(tmp.path().join("Pluralsight")).unwrap();
        let library = scanner.scan().unwrap();
        assert!(library
            .platform(LibraryPlatform::Pluralsight)
            .unwrap()
            .groups
            .is_empty());
        assert!(library.groups.is_empty());
    }

    #[test]
    fn legacy_names_and_empty_course_are_preserved() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("Pluralsight/Legacy Course/Empty module")).unwrap();
        let group = crate::LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Pluralsight, "Legacy Course")
            .unwrap();
        assert_eq!(group.title, "Legacy Course");
        assert_eq!(group.lesson_count, 0);
        assert_eq!(group.courses[0].sections[0].title, "Empty module");
    }

    #[test]
    fn actual_hashed_path_is_stable_identity_but_title_is_human_readable() {
        let tmp = TempDir::new().unwrap();
        let course = "Rust Course-0123456789abcdef";
        let clip = "01 - Ownership-abcdef0123456789";
        let rel = format!("Pluralsight/{course}/01 - Foundations/{clip}");
        put(tmp.path(), &format!("{rel}/video.mp4"), "video");
        put(
            tmp.path(),
            &format!("{rel}/description.html"),
            "description",
        );
        let group = crate::LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Pluralsight, course)
            .unwrap();
        let lesson = &group.courses[0].sections[0].lessons[0];
        assert_eq!(group.title, "Rust Course");
        assert_eq!(group.id, format!("pluralsight/{course}"));
        assert_eq!(
            lesson.key,
            format!("pluralsight:{course}/01 - Foundations/{clip}")
        );
        assert_eq!(lesson.title, "Ownership");
        assert_eq!(
            lesson.video.as_deref(),
            Some(format!("{rel}/video.mp4").as_str())
        );
        assert_eq!(
            lesson.description.as_deref(),
            Some(format!("{rel}/description.html").as_str())
        );
    }

    #[test]
    fn suffix_is_stripped_only_for_exact_sixteen_hex_hash() {
        assert_eq!(display_name("Title-0123456789abcdef"), "Title");
        assert_eq!(
            display_name("Legacy-ABCDEF0123456789"),
            "Legacy-ABCDEF0123456789"
        );
        assert_eq!(
            display_name("Legacy-0123456789abcde"),
            "Legacy-0123456789abcde"
        );
        assert_eq!(display_name("Legacy-not-a-hash"), "Legacy-not-a-hash");
        assert_eq!(display_name("-0123456789abcdef"), "-0123456789abcdef");
    }

    #[test]
    fn malformed_and_hidden_entries_are_not_treated_as_courses_or_clips() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("Pluralsight/Course/01 - Module/.hidden")).unwrap();
        put(
            tmp.path(),
            "Pluralsight/Course/01 - Module/README.txt",
            "foreign file",
        );
        fs::create_dir_all(tmp.path().join("Pluralsight/.hidden-course")).unwrap();
        let library = crate::LibraryScanner::new(tmp.path()).scan().unwrap();
        let groups = &library
            .platform(LibraryPlatform::Pluralsight)
            .unwrap()
            .groups;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Course");
        assert!(groups[0].courses[0].sections[0].lessons.is_empty());
        assert!(crate::LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Pluralsight, "../Skool")
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_pluralsight_root_and_group_are_not_followed() {
        let tmp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir_all(outside.path().join("Course/01 - Module/01 - Clip")).unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("Pluralsight")).unwrap();
        assert!(crate::LibraryScanner::new(tmp.path())
            .scan()
            .unwrap()
            .platform(LibraryPlatform::Pluralsight)
            .is_none());
        fs::remove_file(tmp.path().join("Pluralsight")).unwrap();
        fs::create_dir(tmp.path().join("Pluralsight")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("Course"),
            tmp.path().join("Pluralsight/Escaped"),
        )
        .unwrap();
        assert!(crate::LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Pluralsight, "Escaped")
            .is_err());
        assert!(outside.path().join("Course/01 - Module/01 - Clip").is_dir());
    }
}
