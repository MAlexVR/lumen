//! Safe Pluralsight course and clip paths.

use std::path::{Path, PathBuf};

use lumen_core::sanitize::{sanitize_file_name, MAX_COMPONENT, MAX_PATH};
use lumen_core::PlatformError;
use lumen_download::layout::{path_units, NumberStyle};
use sha2::{Digest, Sha256};

use crate::parse::{Clip, CourseInfo, ModuleInfo};

pub const PLATFORM_DIR: &str = "Pluralsight";
const IDENTITY_HEX_LEN: usize = 16;
const MIN_TEMP_OUTPUT_UNITS: usize = 10;
const MIN_CLIP_FOLDER_UNITS: usize = 23;
const COURSE_CONTENT_RESERVE: usize = 1 + 6 + 1 + MIN_CLIP_FOLDER_UNITS + 1 + MIN_TEMP_OUTPUT_UNITS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedClip {
    pub module_index: usize,
    pub clip_index: usize,
    pub module_title: String,
    pub clip: Clip,
    pub label: String,
    pub output: PathBuf,
}

pub fn course_dir(root: &Path, course: &CourseInfo) -> Result<PathBuf, PlatformError> {
    let parent = root.join(PLATFORM_DIR);
    let budget = MAX_PATH
        .saturating_sub(2 + path_units(&parent) + COURSE_CONTENT_RESERVE)
        .min(100)
        .min(MAX_COMPONENT);
    if budget < IDENTITY_HEX_LEN + 2 {
        return Err(PlatformError::InvalidInput(
            "library path is too long for a safe Pluralsight course folder".into(),
        ));
    }
    let identity = course
        .id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map(|id| (b"id\0".as_slice(), id.as_bytes()))
        .unwrap_or((b"title\0".as_slice(), course.title.as_bytes()));
    let mut digest = Sha256::new();
    digest.update(identity.0);
    digest.update(identity.1);
    let digest = format!("{:x}", digest.finalize());
    let suffix = &digest[..IDENTITY_HEX_LEN];
    let title_budget = budget.saturating_sub(IDENTITY_HEX_LEN + 1);
    let title = sanitize_file_name(&course.title, title_budget);
    let directory = parent.join(format!("{title}-{suffix}"));
    if path_units(&directory) + COURSE_CONTENT_RESERVE >= MAX_PATH {
        return Err(PlatformError::InvalidInput(
            "library path is too long for a safe Pluralsight course folder".into(),
        ));
    }
    Ok(directory)
}

pub fn plan_clips(
    course_dir: &Path,
    modules: &[ModuleInfo],
) -> Result<Vec<PlannedClip>, PlatformError> {
    let mut planned = Vec::new();
    for (module_index, module) in modules.iter().enumerate() {
        let module_title = fallback_title(&module.title, "Module");
        let module_name = numbered_component(
            course_dir,
            NumberStyle::Padded,
            module_index + 1,
            &module_title,
            "",
            1 + MIN_CLIP_FOLDER_UNITS + 1 + MIN_TEMP_OUTPUT_UNITS,
        )?;
        let module_dir = course_dir.join(module_name);
        for (clip_index, clip) in module.clips.iter().enumerate() {
            let title = fallback_title(&clip.title, "Clip");
            let identity = clip_identity(&clip.id, &clip.version);
            let clip_name = numbered_component(
                &module_dir,
                NumberStyle::Padded,
                clip_index + 1,
                &title,
                &format!("-{identity}"),
                1 + MIN_TEMP_OUTPUT_UNITS,
            )?;
            let clip_dir = module_dir.join(clip_name);
            let output_budget = MAX_PATH
                .saturating_sub(2 + path_units(&clip_dir))
                .min(MAX_COMPONENT);
            let title_budget = output_budget.saturating_sub(9);
            if title_budget == 0 {
                return Err(PlatformError::InvalidInput(
                    "library path is too long for a safe Pluralsight clip file".into(),
                ));
            }
            let output = clip_dir.join(format!("{}.mp4", sanitize_file_name(&title, title_budget)));
            if path_units(&output.with_extension("mp4.part")) >= MAX_PATH {
                return Err(PlatformError::InvalidInput(
                    "library path is too long for a safe Pluralsight clip file".into(),
                ));
            }
            let label = format!(
                "{} / {}",
                module_dir.file_name().unwrap_or_default().to_string_lossy(),
                clip_dir.file_name().unwrap_or_default().to_string_lossy()
            );
            planned.push(PlannedClip {
                module_index,
                clip_index,
                module_title: module.title.clone(),
                clip: clip.clone(),
                label,
                output,
            });
        }
    }
    Ok(planned)
}

fn numbered_component(
    parent: &Path,
    style: NumberStyle,
    number: usize,
    title: &str,
    suffix: &str,
    reserve_after: usize,
) -> Result<String, PlatformError> {
    let prefix = style.label(number);
    let budget = MAX_PATH
        .saturating_sub(2 + path_units(parent) + reserve_after)
        .min(MAX_COMPONENT);
    let fixed_units = prefix.encode_utf16().count() + suffix.encode_utf16().count();
    if budget <= fixed_units {
        return Err(PlatformError::InvalidInput(
            "library path is too long for a safe Pluralsight clip folder".into(),
        ));
    }
    let name = format!(
        "{}{}{}",
        prefix,
        sanitize_file_name(title, budget - fixed_units),
        suffix
    );
    if name.encode_utf16().count() > budget {
        return Err(PlatformError::InvalidInput(
            "library path is too long for a safe Pluralsight clip folder".into(),
        ));
    }
    Ok(name)
}

fn clip_identity(id: &Option<String>, version: &Option<String>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"pluralsight-clip-identity\0");
    for value in [
        id.as_deref().unwrap_or(""),
        version.as_deref().unwrap_or(""),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())[..IDENTITY_HEX_LEN].to_string()
}

pub fn fallback_title(title: &str, fallback: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        fallback.to_string()
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_numbers_and_sanitizes_paths() {
        let modules = vec![ModuleInfo {
            title: "Intro/..".into(),
            clips: vec![crate::Clip {
                id: Some("c".into()),
                version: Some("v".into()),
                title: "A:B".into(),
                issue: None,
            }],
            issue: None,
        }];
        let planned = plan_clips(Path::new("/lib/Pluralsight/C"), &modules).unwrap();
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].output.file_name().unwrap(), "A_B.mp4");
        assert!(planned[0]
            .output
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("01 - A_B-"));
        assert_eq!(
            planned[0]
                .output
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .file_name()
                .unwrap(),
            "01 - Intro_"
        );
    }

    #[test]
    fn course_directories_include_stable_identity_for_sanitized_and_truncated_collisions() {
        let root = Path::new("C:\\Users\\person\\Videos\\Lumen");
        let first = CourseInfo {
            id: Some("course-1".into()),
            title: "A:B".into(),
            level: None,
            image_url: None,
            issue: None,
        };
        let second = CourseInfo {
            id: Some("course-2".into()),
            title: "A?B".into(),
            ..first.clone()
        };
        let first_dir = course_dir(root, &first).unwrap();
        let second_dir = course_dir(root, &second).unwrap();
        assert_ne!(first_dir, second_dir);
        assert_eq!(first_dir, course_dir(root, &first).unwrap());

        let long_title = "A very long course title ".repeat(20);
        let long_first = CourseInfo {
            title: long_title.clone(),
            ..first
        };
        let long_second = CourseInfo {
            title: long_title,
            ..second
        };
        let long_first_dir = course_dir(root, &long_first).unwrap();
        let long_second_dir = course_dir(root, &long_second).unwrap();
        assert_ne!(long_first_dir, long_second_dir);
        assert_eq!(long_first_dir, course_dir(root, &long_first).unwrap());
        assert!(path_units(&long_first_dir) < 260);
        assert!(path_units(&long_second_dir) < 260);
    }

    #[test]
    fn missing_or_invalid_course_ids_still_produce_safe_paths() {
        let root = Path::new("/library");
        for id in [None, Some(String::new()), Some("../course".into())] {
            let course = CourseInfo {
                id,
                title: "A:B".into(),
                level: None,
                image_url: None,
                issue: None,
            };
            let directory = course_dir(root, &course).unwrap();
            assert_eq!(directory.parent(), Some(root.join(PLATFORM_DIR).as_path()));
            assert!(directory.file_name().is_some());
        }
    }

    #[test]
    fn duplicate_and_long_titles_stay_distinct_and_within_path_budget() {
        let title = "Very long course title ".repeat(20);
        let course = CourseInfo {
            id: Some("id".into()),
            title: title.clone(),
            level: None,
            image_url: None,
            issue: None,
        };
        let root = Path::new("C:\\Users\\person\\Videos\\Lumen");
        let dir = course_dir(root, &course).unwrap();
        let modules = vec![
            ModuleInfo {
                title: title.clone(),
                issue: None,
                clips: vec![
                    Clip {
                        id: Some("one".into()),
                        version: Some("1".into()),
                        title: "same".into(),
                        issue: None,
                    },
                    Clip {
                        id: Some("two".into()),
                        version: Some("1".into()),
                        title: "same".into(),
                        issue: None,
                    },
                ],
            },
            ModuleInfo {
                title: "Advanced".into(),
                issue: None,
                clips: vec![Clip {
                    id: Some("three".into()),
                    version: Some("1".into()),
                    title: "Next".into(),
                    issue: None,
                }],
            },
        ];
        let planned = plan_clips(&dir, &modules).unwrap();
        assert_eq!(planned.len(), 3);
        assert_ne!(planned[0].output, planned[1].output);
        assert!(planned[2]
            .output
            .to_string_lossy()
            .contains("02 - Advanced/01 - Next-"));
        let longest = planned[1].output.with_extension("mp4.part");
        assert!(path_units(&longest) < 260);
        assert!(planned.iter().all(|item| item.output.starts_with(&dir)));
    }

    #[test]
    fn clip_output_identity_tracks_id_and_version_not_only_position_and_title() {
        let clip = |id: &str, version: &str| Clip {
            id: Some(id.into()),
            version: Some(version.into()),
            title: "Same title".into(),
            issue: None,
        };
        let plan = |clip| {
            plan_clips(
                Path::new("/library/Pluralsight/Course"),
                &[ModuleInfo {
                    title: "Module".into(),
                    clips: vec![clip],
                    issue: None,
                }],
            )
            .unwrap()
        };

        let first = plan(clip("clip-a", "v1"));
        let other_id = plan(clip("clip-b", "v1"));
        let other_version = plan(clip("clip-a", "v2"));
        assert_ne!(first[0].output, other_id[0].output);
        assert_ne!(first[0].output, other_version[0].output);
        assert_eq!(first[0].output, plan(clip("clip-a", "v1"))[0].output);
    }

    #[test]
    fn unicode_root_paths_fit_max_path_or_are_rejected() {
        let root = PathBuf::from(format!("C:\\{}", "目录🌞".repeat(45)));
        let course = CourseInfo {
            id: Some("stable-course".into()),
            title: "A long but ordinary course title".into(),
            level: None,
            image_url: None,
            issue: None,
        };
        let dir = course_dir(&root, &course).unwrap();
        let course_component = dir.file_name().unwrap().to_string_lossy();
        assert_eq!(
            course_component.rsplit('-').next().unwrap().len(),
            IDENTITY_HEX_LEN
        );
        let plan = plan_clips(
            &dir,
            &[ModuleInfo {
                title: "Módulo".into(),
                clips: vec![Clip {
                    id: Some("clip-id".into()),
                    version: Some("version-1".into()),
                    title: "Título".into(),
                    issue: None,
                }],
                issue: None,
            }],
        )
        .unwrap();
        let part_path = plan[0].output.with_extension("mp4.part");
        assert!(path_units(&part_path) < 260, "{}", path_units(&part_path));
        assert!(path_units(&part_path) >= root.to_string_lossy().encode_utf16().count());
    }

    #[test]
    fn impossible_unicode_root_is_rejected_without_shortening_identity() {
        let root = PathBuf::from(format!("C:\\{}", "目录🌞".repeat(70)));
        let course = CourseInfo {
            id: Some("stable-course".into()),
            title: "Course".into(),
            level: None,
            image_url: None,
            issue: None,
        };
        let error = course_dir(&root, &course).unwrap_err();
        assert!(matches!(error, PlatformError::InvalidInput(_)));
        assert!(error.to_string().contains("library path is too long"));
    }
}
