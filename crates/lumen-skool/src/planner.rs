//! Builds the ordered lesson list and its on-disk layout:
//! `<library>/Skool/<group>/<NN - course>/<NN - section>/<NN - lesson>/`.

use std::path::{Path, PathBuf};

use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_download::layout::{numbered_folder, path_units, NumberStyle};

use crate::parse::{CourseInfo, CourseTree, ModuleEntry};
use crate::richtext::strip_emoji;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedLesson {
    /// Position in the whole group (0-based), stable across runs.
    pub order: usize,
    pub course_short_name: String,
    pub course_title: String,
    pub section_title: String,
    pub title: String,
    pub module: ModuleEntry,
    pub dir: PathBuf,
    /// Video file name without extension, inside `dir`.
    pub file_stem: String,
}

impl PlannedLesson {
    /// Human label used in reports: `<section> / <NN - lesson>`.
    pub fn label(&self) -> String {
        let folder = self
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{} / {}", self.section_title, folder)
    }
}

/// Folder name of a group: display name without emoji, Windows-safe.
pub fn group_folder_name(display_name: &str) -> String {
    sanitize_file_name(&strip_emoji(display_name), 80)
}

fn numbered(parent: &Path, number: usize, title: &str, reserve: usize, cap: usize) -> String {
    numbered_folder(parent, NumberStyle::Padded, number, title, reserve, cap)
}

/// Plans one course. `course_number` is 1-based among accessible courses and
/// `first_order` the number of lessons planned before it.
pub fn plan_course(
    group_dir: &Path,
    course_number: usize,
    course: &CourseInfo,
    tree: &CourseTree,
    first_order: usize,
) -> Vec<PlannedLesson> {
    let course_dir = group_dir.join(numbered(group_dir, course_number, &course.title, 70, 60));
    let mut lessons = Vec::new();
    let mut section: Option<&str> = None;
    let (mut section_number, mut lesson_number) = (0, 0);
    let mut section_dir = course_dir.clone();
    for module in &tree.modules {
        if section != Some(module.section.as_str()) {
            section = Some(module.section.as_str());
            section_number += 1;
            lesson_number = 0;
            section_dir = course_dir.join(numbered(
                &course_dir,
                section_number,
                &module.section,
                45,
                60,
            ));
        }
        lesson_number += 1;
        let title = match module.title.trim() {
            "" => "Lesson",
            t => t,
        };
        let dir = section_dir.join(numbered(&section_dir, lesson_number, title, 30, 90));
        let stem_budget = name_budget(path_units(&dir), 9).min(200);
        lessons.push(PlannedLesson {
            order: first_order + lessons.len(),
            course_short_name: course.short_name.clone(),
            course_title: course.title.clone(),
            section_title: module.section.clone(),
            title: title.to_string(),
            module: module.clone(),
            file_stem: sanitize_file_name(title, stem_budget),
            dir,
        });
    }
    lessons
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::sanitize::MAX_PATH;

    fn module(id: &str, section: &str, title: &str) -> ModuleEntry {
        ModuleEntry {
            id: id.into(),
            section: section.into(),
            title: title.into(),
            video_link: String::new(),
            video_id: String::new(),
            desc: String::new(),
            resources: String::new(),
            has_access: true,
        }
    }

    fn course() -> CourseInfo {
        CourseInfo {
            id: "c1".into(),
            short_name: "short".into(),
            title: "Intro: Course?".into(),
            num_modules: 0,
        }
    }

    fn tree(modules: Vec<ModuleEntry>) -> CourseTree {
        CourseTree {
            title: "Intro: Course?".into(),
            modules,
        }
    }

    #[test]
    fn group_folder_drops_emoji_and_illegal_characters() {
        assert_eq!(group_folder_name("🚀 Growth: Lab ™"), "Growth_ Lab");
        assert_eq!(group_folder_name("🚀"), "_");
    }

    #[test]
    fn layout_numbers_courses_sections_and_lessons() {
        let t = tree(vec![
            module("a", "Start", "Welcome"),
            module("b", "Start", "Setup"),
            module("c", "Advanced", "Tips/Tricks"),
        ]);
        let plan = plan_course(Path::new("/lib/Skool/G"), 2, &course(), &t, 5);
        let rel: Vec<String> = plan
            .iter()
            .map(|p| {
                p.dir
                    .strip_prefix("/lib/Skool/G")
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            rel,
            [
                "02 - Intro_ Course_/01 - Start/01 - Welcome",
                "02 - Intro_ Course_/01 - Start/02 - Setup",
                "02 - Intro_ Course_/02 - Advanced/01 - Tips_Tricks",
            ]
        );
        assert_eq!(plan.iter().map(|p| p.order).collect::<Vec<_>>(), [5, 6, 7]);
        assert_eq!(plan[0].file_stem, "Welcome");
        assert_eq!(plan[2].file_stem, "Tips_Tricks");
        assert_eq!(plan[0].module.id, "a");
        assert_eq!(plan[0].label(), "Start / 01 - Welcome");
    }

    #[test]
    fn empty_titles_get_a_placeholder() {
        let t = tree(vec![module("a", "S", "  ")]);
        let plan = plan_course(Path::new("/g"), 1, &course(), &t, 0);
        assert!(plan[0].dir.ends_with("01 - Lesson"));
        assert_eq!(plan[0].file_stem, "Lesson");
    }

    #[test]
    fn scanner_can_read_the_layout_back() {
        let t = tree(vec![module("a", "Start", "Welcome")]);
        let tmp = tempfile::tempdir().unwrap();
        let group_dir = tmp.path().join("Skool/G");
        let plan = plan_course(&group_dir, 1, &course(), &t, 0);
        std::fs::create_dir_all(&plan[0].dir).unwrap();
        std::fs::write(plan[0].dir.join("Welcome.mp4"), "v").unwrap();
        let lib = lumen_core::LibraryScanner::new(tmp.path()).scan().unwrap();
        let lesson = &lib.groups[0].courses[0].sections[0].lessons[0];
        assert_eq!(lesson.title, "Welcome");
        assert!(lesson.has_video);
        assert_eq!(lib.groups[0].courses[0].title, "Intro_ Course_");
    }

    #[test]
    fn long_names_keep_every_planned_file_within_max_path() {
        let long = "Título muy largo ".repeat(30);
        let t = tree(vec![module("a", &long, &long)]);
        let c = CourseInfo {
            title: long.clone(),
            ..course()
        };
        let group_dir = PathBuf::from(format!(
            "C:\\Users\\someone\\Videos\\Lumen\\Skool\\{}",
            "G".repeat(40)
        ));
        let plan = plan_course(&group_dir, 1, &c, &t, 0);
        let p = &plan[0];
        let longest = p.dir.join(format!("{}.mp4.part", p.file_stem));
        let units = longest.to_string_lossy().encode_utf16().count();
        assert!(units < MAX_PATH, "{units} >= {MAX_PATH}: {longest:?}");
        assert!(p.file_stem.chars().count() >= 8);
    }
}
