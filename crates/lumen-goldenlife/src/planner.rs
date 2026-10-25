//! On-disk layout, the same as Hotmart:
//! `<library>/GoldenLife/<course>/<N. section>/<N. lesson>/`.

use std::path::{Path, PathBuf};

use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_download::layout::{numbered_folder, path_units, NumberStyle};

use crate::parse::Outline;

/// Platform folder inside the library.
pub const PLATFORM_DIR: &str = "GoldenLife";

/// Course folder: `<library>/GoldenLife/<title>`.
pub fn course_dir(library_root: &Path, title: &str) -> PathBuf {
    let parent = library_root.join(PLATFORM_DIR);
    let budget = name_budget(path_units(&parent), 110).min(100);
    parent.join(sanitize_file_name(title, budget))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedLesson {
    /// Position in the whole course (0-based), stable across runs.
    pub order: usize,
    /// Section title for progress display.
    pub section_title: String,
    /// Lesson title for progress display.
    pub title: String,
    pub lesson_id: String,
    pub dir: PathBuf,
}

impl PlannedLesson {
    /// Report label: `<section folder> / <lesson folder>`.
    pub fn label(&self) -> String {
        let name = |p: Option<&Path>| {
            p.and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        format!("{} / {}", name(self.dir.parent()), name(Some(&self.dir)))
    }
}

/// Plans every lesson of every section, numbering both from 1. Empty titles
/// become `Section` and `Lesson`.
pub fn plan_course(course_dir: &Path, outline: &Outline) -> Vec<PlannedLesson> {
    let mut lessons = Vec::new();
    for (si, section) in outline.sections.iter().enumerate() {
        let section_title = or_default(&section.title, "Section");
        let section_dir = course_dir.join(numbered_folder(
            course_dir,
            NumberStyle::Dotted,
            si + 1,
            section_title,
            60,
            70,
        ));
        for (li, lesson) in section.lessons.iter().enumerate() {
            let title = or_default(&lesson.title, "Lesson");
            let dir = section_dir.join(numbered_folder(
                &section_dir,
                NumberStyle::Dotted,
                li + 1,
                title,
                45,
                90,
            ));
            lessons.push(PlannedLesson {
                order: lessons.len(),
                section_title: section_title.to_string(),
                title: title.to_string(),
                lesson_id: lesson.id.clone(),
                dir,
            });
        }
    }
    lessons
}

fn or_default<'a>(title: &'a str, fallback: &'a str) -> &'a str {
    if title.trim().is_empty() {
        fallback
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{LessonRef, Section};

    fn outline() -> Outline {
        let lesson = |id: &str, title: &str| LessonRef {
            id: id.into(),
            title: title.into(),
        };
        Outline {
            product_id: Some("1".into()),
            sections: vec![
                Section {
                    id: "s1".into(),
                    title: "Primeros pasos: bienvenida".into(),
                    lessons: vec![lesson("10", "Bienvenida"), lesson("11", "")],
                },
                Section {
                    id: "s2".into(),
                    title: "".into(),
                    lessons: vec![lesson("20", "Plantillas?")],
                },
            ],
        }
    }

    #[test]
    fn courses_live_under_the_platform_folder_with_a_safe_name() {
        let root = Path::new("/lib");
        assert_eq!(
            course_dir(root, "Meta Ads: Básico?"),
            root.join("GoldenLife/Meta Ads_ Básico_")
        );
    }

    #[test]
    fn lessons_are_numbered_per_section_like_hotmart() {
        let course = Path::new("/lib/GoldenLife/C");
        let plan = plan_course(course, &outline());
        let dirs: Vec<_> = plan
            .iter()
            .map(|l| {
                l.dir
                    .strip_prefix(course)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            dirs,
            [
                "1. Primeros pasos_ bienvenida/1. Bienvenida",
                "1. Primeros pasos_ bienvenida/2. Lesson",
                "2. Section/1. Plantillas_",
            ]
        );
        assert_eq!(plan.iter().map(|l| l.order).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(plan[2].lesson_id, "20");
        assert_eq!(
            plan[0].label(),
            "1. Primeros pasos_ bienvenida / 1. Bienvenida"
        );
    }

    #[test]
    fn an_outline_without_sections_plans_nothing() {
        assert!(plan_course(Path::new("/x"), &Outline::default()).is_empty());
    }
}
