//! On-disk layout, identical to the previous Go implementation so existing folders are
//! recognised: `<library>/Hotmart/<course - seller>/<N. module>/<N. lesson>/`.

use std::path::{Path, PathBuf};

use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_download::layout::{numbered_folder, path_units, NumberStyle};

use crate::naming::{clean_name, course_folder_name};
use crate::parse::{HotmartCourse, Navigation};

/// Platform folder inside the library.
pub const PLATFORM_DIR: &str = "Hotmart";

/// Course folder: `<library>/Hotmart/<name> - <seller>`.
pub fn course_dir(library_root: &Path, course: &HotmartCourse) -> PathBuf {
    let parent = library_root.join(PLATFORM_DIR);
    let budget = name_budget(path_units(&parent), 110).min(100);
    let name = course_folder_name(&course.name, course.seller.as_deref());
    parent.join(sanitize_file_name(&name, budget))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedLesson {
    /// Position in the whole course (0-based), stable across runs.
    pub order: usize,
    /// Module title for progress display (raw, may be empty).
    pub module_title: String,
    /// Lesson title for progress display.
    pub title: String,
    /// Empty when Hotmart gave the page no hash (it cannot be fetched).
    pub hash: String,
    pub dir: PathBuf,
}

impl PlannedLesson {
    /// Report label: `<module folder> / <lesson folder>`.
    pub fn label(&self) -> String {
        let name = |p: Option<&Path>| {
            p.and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        format!("{} / {}", name(self.dir.parent()), name(Some(&self.dir)))
    }
}

/// Plans every page of every module, numbering both from 1 like the previous Go
/// layout. Empty module names become `unnamed` and empty page names `Topic`
/// (the previous defaults).
pub fn plan_course(course_dir: &Path, nav: &Navigation) -> Vec<PlannedLesson> {
    let mut lessons = Vec::new();
    for (mi, module) in nav.modules.iter().enumerate() {
        let module_dir = course_dir.join(numbered_folder(
            course_dir,
            NumberStyle::Dotted,
            mi + 1,
            &clean_name(&module.name),
            60,
            70,
        ));
        for (pi, page) in module.pages.iter().enumerate() {
            let title = match page.name.as_str() {
                "" => "Topic",
                name => name,
            };
            let dir = module_dir.join(numbered_folder(
                &module_dir,
                NumberStyle::Dotted,
                pi + 1,
                &clean_name(title),
                45,
                90,
            ));
            lessons.push(PlannedLesson {
                order: lessons.len(),
                module_title: module.name.clone(),
                title: title.to_string(),
                hash: page.hash.clone(),
                dir,
            });
        }
    }
    lessons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{NavModule, NavPage};
    use lumen_core::sanitize::MAX_PATH;

    fn course(name: &str, seller: Option<&str>) -> HotmartCourse {
        HotmartCourse {
            id: "1".into(),
            name: name.into(),
            slug: "s".into(),
            seller: seller.map(str::to_string),
            cover_url: None,
            purchase_date: None,
        }
    }

    fn page(hash: &str, name: &str) -> NavPage {
        NavPage {
            hash: hash.into(),
            name: name.into(),
        }
    }

    fn nav(modules: Vec<(&str, Vec<NavPage>)>) -> Navigation {
        Navigation {
            modules: modules
                .into_iter()
                .map(|(name, pages)| NavModule {
                    name: name.into(),
                    pages,
                })
                .collect(),
        }
    }

    #[test]
    fn course_folder_matches_the_real_library() {
        let c = course("MÉTODO HORIZONTE", Some("Marta Ledesma"));
        assert_eq!(
            course_dir(Path::new("/lib"), &c),
            Path::new("/lib/Hotmart/MÉTODO HORIZONTE - Marta Ledesma")
        );
    }

    #[test]
    fn modules_and_lessons_are_numbered_from_one_without_padding() {
        let n = nav(vec![
            ("¡Bienvenidos!", vec![page("a", "Welcome"), page("b", "")]),
            ("", vec![page("c", "Q&A: live?")]),
            ("Empty", vec![]),
            ("Módulo 3", vec![page("d", "Guide")]),
        ]);
        let plan = plan_course(Path::new("/lib/Hotmart/C"), &n);
        let rel: Vec<String> = plan
            .iter()
            .map(|p| {
                p.dir
                    .strip_prefix("/lib/Hotmart/C")
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            rel,
            [
                "1. ¡Bienvenidos!/1. Welcome",
                "1. ¡Bienvenidos!/2. Topic",
                "2. unnamed/1. QandA live",
                "4. Módulo 3/1. Guide",
            ]
        );
        assert_eq!(
            plan.iter().map(|p| p.order).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(plan[0].hash, "a");
        assert_eq!(plan[1].title, "Topic");
        assert_eq!(plan[0].module_title, "¡Bienvenidos!");
        assert_eq!(plan[0].label(), "1. ¡Bienvenidos! / 1. Welcome");
    }

    #[test]
    fn long_names_stay_within_max_path() {
        let long = "Título muy largo ".repeat(30);
        let n = nav(vec![(&long, vec![page("a", &long)])]);
        let root = format!(
            "C:\\Users\\someone\\Videos\\Lumen\\Hotmart\\{}",
            "C".repeat(80)
        );
        let plan = plan_course(Path::new(&root), &n);
        let longest = plan[0].dir.join("Supplementary reading.html.part");
        let units = longest.to_string_lossy().encode_utf16().count();
        assert!(units < MAX_PATH, "{units}");
    }
}
