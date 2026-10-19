//! YouTube on-disk layout (written by `lumen-youtube`):
//! `YouTube/<playlist title>/<NN - video title>/<video title>.<ext>` plus
//! `description.txt` and a thumbnail. `YouTube/Videos sueltos/` collects
//! single videos the same way.
//!
//! A playlist is ONE group with ONE course and ONE section; every video is a
//! lesson, so the tree stays uniform (`Group > Course > Section > Lesson`).

use std::path::Path;

use crate::model::{youtube_lesson_key, Collection, Course, Group, LibraryPlatform, Section};
use crate::scanner::{
    build_lesson, extension, group_status, name_of, ordered_subdirs, split_order_prefix,
};

/// Folder of single videos (`lumen-youtube` writes it).
const STANDALONE_FOLDER: &str = "Videos sueltos";
const DESCRIPTION_TXT: &str = "description.txt";
const THUMBNAIL_STEM: &str = "thumbnail";
/// Audio-only downloads (the "audio" preset) play like video.
const AUDIO_EXTS: [&str; 5] = ["m4a", "mp3", "opus", "ogg", "aac"];
const IMAGE_EXTS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];

pub(crate) fn scan_group(dir: &Path, folder: &str) -> Group {
    let mut lessons = Vec::new();
    for entry in ordered_subdirs(dir) {
        let name = name_of(&entry);
        let (title, index) = split_order_prefix(&name);
        let rel = format!("{}/{folder}/{name}", LibraryPlatform::Youtube.dir_name());
        let key = youtube_lesson_key((1, folder), (1, folder), (index, title));
        let mut lesson = build_lesson(&entry.path(), &rel, title, index, key);
        let mut thumbnails: Vec<String> = Vec::new();
        lesson.resources.retain(|r| {
            let file = r.path.rsplit('/').next().unwrap_or_default();
            if file.eq_ignore_ascii_case(DESCRIPTION_TXT) {
                lesson.description.get_or_insert_with(|| r.path.clone());
                return false;
            }
            let stem = file.split('.').next().unwrap_or_default();
            let thumb = stem == THUMBNAIL_STEM && IMAGE_EXTS.contains(&extension(file).as_str());
            if thumb {
                thumbnails.push(r.path.clone());
            }
            !thumb
        });
        // IMAGE_EXTS is ordered by preference.
        lesson.thumbnail = IMAGE_EXTS
            .iter()
            .find_map(|ext| thumbnails.iter().find(|p| extension(p) == *ext).cloned());
        if lesson.video.is_none() {
            let audio = lesson
                .resources
                .iter()
                .position(|r| AUDIO_EXTS.contains(&extension(&r.name).as_str()));
            if let Some(at) = audio {
                lesson.video = Some(lesson.resources.remove(at).path);
                lesson.has_video = true;
            }
        }
        lessons.push(lesson);
    }
    let sections = vec![Section {
        title: folder.to_string(),
        index: 1,
        lessons,
    }];
    let courses = vec![Course {
        title: folder.to_string(),
        index: 1,
        sections,
    }];
    Group {
        id: format!("youtube/{folder}"),
        platform: LibraryPlatform::Youtube,
        name: folder.to_string(),
        title: folder.to_string(),
        seller: None,
        status: group_status(dir),
        lesson_count: Group::count_lessons(&courses),
        courses,
        collection: Some(if folder == STANDALONE_FOLDER {
            Collection::Standalone
        } else {
            Collection::Playlist
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LibraryPlatform, LibraryScanner};
    use std::fs;
    use tempfile::TempDir;

    fn put(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn playlist(tmp: &TempDir) -> crate::Group {
        let root = tmp.path();
        put(root, "YouTube/Mi lista ñ/01 - Intro/Intro.mp4", "v");
        put(
            root,
            "YouTube/Mi lista ñ/01 - Intro/description.txt",
            "hola",
        );
        put(root, "YouTube/Mi lista ñ/01 - Intro/thumbnail.jpg", "j");
        put(root, "YouTube/Mi lista ñ/02 - Segundo/Segundo.webm", "v");
        put(
            root,
            "YouTube/Mi lista ñ/02 - Segundo/Segundo.mp4.part",
            "p",
        );
        put(root, "YouTube/Mi lista ñ/10 - Décimo/Décimo.mkv", "v");
        put(root, "YouTube/Mi lista ñ/.complete", "done");
        put(root, "YouTube/Mi lista ñ/download-report.json", "{}");
        LibraryScanner::new(root)
            .scan_platform_group(LibraryPlatform::Youtube, "Mi lista ñ")
            .unwrap()
    }

    #[test]
    fn a_playlist_is_one_group_one_course_one_section() {
        let tmp = TempDir::new().unwrap();
        let g = playlist(&tmp);
        assert_eq!(g.id, "youtube/Mi lista ñ");
        assert_eq!(g.platform, LibraryPlatform::Youtube);
        assert_eq!(
            (g.name.as_str(), g.title.as_str()),
            ("Mi lista ñ", "Mi lista ñ")
        );
        assert_eq!(g.seller, None);
        assert_eq!(g.status, crate::GroupStatus::Complete);
        assert_eq!(g.lesson_count, 3);
        assert_eq!(g.courses.len(), 1);
        assert_eq!(g.courses[0].sections.len(), 1);
        let titles: Vec<_> = g.courses[0].sections[0]
            .lessons
            .iter()
            .map(|l| (l.index, l.title.as_str()))
            .collect();
        assert_eq!(titles, [(1, "Intro"), (2, "Segundo"), (10, "Décimo")]);
    }

    #[test]
    fn lessons_carry_video_description_and_platform_qualified_keys() {
        let tmp = TempDir::new().unwrap();
        let g = playlist(&tmp);
        let l = &g.courses[0].sections[0].lessons;
        assert_eq!(
            l[0].video.as_deref(),
            Some("YouTube/Mi lista ñ/01 - Intro/Intro.mp4")
        );
        assert_eq!(
            l[0].description.as_deref(),
            Some("YouTube/Mi lista ñ/01 - Intro/description.txt")
        );
        assert!(l[0].has_video);
        assert!(
            l[0].resources.is_empty(),
            "description and thumbnail are not resources"
        );
        assert_eq!(
            l[1].video.as_deref(),
            Some("YouTube/Mi lista ñ/02 - Segundo/Segundo.webm")
        );
        assert!(l[1].resources.is_empty(), "partial files are ignored");
        assert_eq!(l[0].key, "youtube:1-Mi lista ñ/1-Mi lista ñ/1-Intro");
        assert!(l.iter().all(|x| x.key.starts_with("youtube:")));
    }

    #[test]
    fn an_unfinished_folder_is_partial() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "YouTube/L/01 - A/A.mp4", "v");
        let g = LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Youtube, "L")
            .unwrap();
        assert_eq!(g.status, crate::GroupStatus::Partial);
    }

    #[test]
    fn videos_sueltos_is_a_group_like_any_other() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "YouTube/Videos sueltos/01 - Uno/Uno.mp4", "v");
        put(tmp.path(), "YouTube/Videos sueltos/02 - Dos/Dos.mp4", "v");
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        let yt = lib.platform(LibraryPlatform::Youtube).unwrap();
        assert_eq!(yt.groups[0].id, "youtube/Videos sueltos");
        assert_eq!(yt.groups[0].lesson_count, 2);
    }

    #[test]
    fn the_youtube_entry_exists_only_when_its_folder_does_and_json_is_camel_case() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Hotmart/H - Me/1. M/1. L/1. Lesson.mp4", "v");
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        assert!(lib.platform(LibraryPlatform::Youtube).is_none());
        put(tmp.path(), "YouTube/L/01 - A/A.mp4", "v");
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        let ids: Vec<_> = lib.platforms.iter().map(|p| p.id).collect();
        assert_eq!(ids, [LibraryPlatform::Hotmart, LibraryPlatform::Youtube]);
        let json = serde_json::to_value(&lib).unwrap();
        let p = &json["platforms"][1];
        assert_eq!(p["id"], "youtube");
        assert_eq!(p["groups"][0]["platform"], "youtube");
        assert_eq!(p["groups"][0]["lessonCount"], 1);
        assert_eq!(LibraryPlatform::Youtube.dir_name(), "YouTube");
    }

    #[test]
    fn keys_never_collide_with_other_platforms() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "Skool/G/01 - G/01 - G/01 - A/a.mp4", "v");
        put(tmp.path(), "YouTube/G/01 - A/a.mp4", "v");
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        let sk = &lib.groups[0].courses[0].sections[0].lessons[0].key;
        let yt = &lib.platform(LibraryPlatform::Youtube).unwrap().groups[0].courses[0].sections[0]
            .lessons[0]
            .key;
        assert_ne!(sk, yt);
    }

    #[test]
    fn an_audio_only_download_is_the_lesson_media() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "YouTube/Música/01 - Tema/Tema.m4a", "a");
        put(tmp.path(), "YouTube/Música/02 - Otro/Otro.mp3", "a");
        put(tmp.path(), "YouTube/Música/02 - Otro/notes.pdf", "n");
        let g = LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Youtube, "Música")
            .unwrap();
        let l = &g.courses[0].sections[0].lessons;
        assert_eq!(
            l[0].video.as_deref(),
            Some("YouTube/Música/01 - Tema/Tema.m4a")
        );
        assert!(l[0].has_video && l[0].resources.is_empty());
        assert_eq!(
            l[1].video.as_deref(),
            Some("YouTube/Música/02 - Otro/Otro.mp3")
        );
        assert_eq!(l[1].resources.len(), 1, "other files stay resources");
    }

    #[test]
    fn only_videos_sueltos_is_a_standalone_collection_and_playlists_say_so() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "YouTube/Videos sueltos/01 - Uno/Uno.mp4", "v");
        put(tmp.path(), "YouTube/Mi lista/01 - A/A.mp4", "v");
        put(tmp.path(), "Skool/G/01 - G/01 - S/01 - L/a.mp4", "v");
        let lib = LibraryScanner::new(tmp.path()).scan().unwrap();
        let yt = &lib.platform(LibraryPlatform::Youtube).unwrap().groups;
        let by = |n: &str| yt.iter().find(|g| g.name == n).unwrap();
        assert_eq!(
            by("Videos sueltos").collection,
            Some(crate::Collection::Standalone)
        );
        assert_eq!(by("Mi lista").collection, Some(crate::Collection::Playlist));
        let json = serde_json::to_value(&lib).unwrap();
        let groups = json["platforms"][1]["groups"].as_array().unwrap();
        let values: Vec<_> = groups
            .iter()
            .map(|g| g["collection"].as_str().unwrap().to_string())
            .collect();
        assert!(
            values.contains(&"standalone".to_string()) && values.contains(&"playlist".to_string())
        );
        assert!(
            json["platforms"][0]["groups"][0]
                .get("collection")
                .is_none(),
            "Skool groups carry no collection"
        );
    }

    #[test]
    fn the_youtube_thumbnail_is_exposed_but_not_as_a_resource() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "YouTube/L/01 - Con/Con.mp4", "v");
        put(tmp.path(), "YouTube/L/01 - Con/thumbnail.webp", "w");
        put(tmp.path(), "YouTube/L/01 - Con/thumbnail.jpg", "j");
        put(tmp.path(), "YouTube/L/02 - Sin/Sin.mp4", "v");
        let g = LibraryScanner::new(tmp.path())
            .scan_platform_group(LibraryPlatform::Youtube, "L")
            .unwrap();
        let l = &g.courses[0].sections[0].lessons;
        assert_eq!(
            l[0].thumbnail.as_deref(),
            Some("YouTube/L/01 - Con/thumbnail.jpg"),
            "jpg is preferred"
        );
        assert!(l[0].resources.is_empty());
        assert_eq!(l[1].thumbnail, None);
        let json = serde_json::to_value(&l[0]).unwrap();
        assert_eq!(json["thumbnail"], "YouTube/L/01 - Con/thumbnail.jpg");
        assert_eq!(
            serde_json::to_value(&l[1]).unwrap()["thumbnail"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn lessons_know_their_own_folder_but_never_serialize_it() {
        let tmp = TempDir::new().unwrap();
        put(tmp.path(), "YouTube/L/01 - Uno/Uno.mp4", "v");
        put(tmp.path(), "Skool/G/01 - G/01 - S/01 - Lec/a.mp4", "v");
        put(
            tmp.path(),
            "Hotmart/H - Me/1. M/1. Carpeta/1. Carpeta.mp4",
            "v",
        );
        put(tmp.path(), "Hotmart/H - Me/1. M/2. Suelta.mp4", "v");
        let s = LibraryScanner::new(tmp.path());
        let yt = s
            .scan_platform_group(LibraryPlatform::Youtube, "L")
            .unwrap();
        assert_eq!(
            yt.courses[0].sections[0].lessons[0].dir.as_deref(),
            Some("YouTube/L/01 - Uno")
        );
        let sk = s.scan_platform_group(LibraryPlatform::Skool, "G").unwrap();
        assert_eq!(
            sk.courses[0].sections[0].lessons[0].dir.as_deref(),
            Some("Skool/G/01 - G/01 - S/01 - Lec")
        );
        let hm = s
            .scan_platform_group(LibraryPlatform::Hotmart, "H - Me")
            .unwrap();
        let lessons = &hm.courses[0].sections[0].lessons;
        assert_eq!(
            lessons[0].dir.as_deref(),
            Some("Hotmart/H - Me/1. M/1. Carpeta")
        );
        assert_eq!(
            lessons[1].dir, None,
            "a loose file has no folder of its own"
        );
        assert!(serde_json::to_value(&lessons[0])
            .unwrap()
            .get("dir")
            .is_none());
    }

    #[test]
    fn invalid_group_names_are_rejected() {
        let tmp = TempDir::new().unwrap();
        let s = LibraryScanner::new(tmp.path());
        for g in ["", "..", "a/b", "a\\b"] {
            assert!(s.scan_platform_group(LibraryPlatform::Youtube, g).is_err());
        }
    }
}
