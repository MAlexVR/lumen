//! On-disk layout: `<library>/Harbiz/Harbiz Training/<date - workout>/` holds
//! `workout.json` and one `NN. <exercise>.<ext>` video per exercise (plus an
//! `NN. <exercise>.html` note). The library scanner reads a workout folder as a
//! module and its loose videos as lessons.

use std::path::{Path, PathBuf};

use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_download::files::UniqueNames;
use lumen_download::layout::path_units;

use crate::parse::{Exercise, Workout};

/// Platform folder inside the library.
pub const PLATFORM_DIR: &str = "Harbiz";
/// The single course every workout lives in.
pub const COURSE_TITLE: &str = "Harbiz Training";
pub const WORKOUT_FILE: &str = "workout.json";

const VIDEO_EXTS: [&str; 3] = ["mp4", "mkv", "webm"];

pub fn course_dir(library_root: &Path) -> PathBuf {
    library_root.join(PLATFORM_DIR).join(COURSE_TITLE)
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedExercise {
    /// Position in the workout, 1-based.
    pub number: usize,
    /// File stem shared by the video and its note: `NN. <exercise>`.
    pub stem: String,
    pub exercise: Exercise,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedWorkout {
    /// Position in the whole plan (0-based).
    pub order: usize,
    pub dir: PathBuf,
    /// Folder name, used as the report label.
    pub label: String,
    pub workout: Workout,
    pub exercises: Vec<PlannedExercise>,
}

/// Plans every workout, in the given order. Names depend only on a workout's own
/// date and title (plus a counter for exact repeats), never on what else is on
/// the agenda, so a new workout cannot rename the folders already downloaded.
pub fn plan_workouts(course_dir: &Path, workouts: &[Workout]) -> Vec<PlannedWorkout> {
    let folder_budget = name_budget(path_units(course_dir), 60).min(100);
    let mut folders = UniqueNames::default();
    workouts
        .iter()
        .enumerate()
        .map(|(order, workout)| {
            let title = Some(workout.title.trim())
                .filter(|t| !t.is_empty())
                .unwrap_or("Workout");
            let label = folders.claim(&sanitize_file_name(
                &format!("{} - {title}", workout.date),
                folder_budget,
            ));
            let dir = course_dir.join(&label);
            let exercises = plan_exercises(&dir, &workout.exercises);
            PlannedWorkout {
                order,
                dir,
                label,
                workout: workout.clone(),
                exercises,
            }
        })
        .collect()
}

fn plan_exercises(dir: &Path, exercises: &[Exercise]) -> Vec<PlannedExercise> {
    // Room for the extension and yt-dlp's `.part` / `.fNNN` leftovers.
    let budget = name_budget(path_units(dir), 20).min(80);
    exercises
        .iter()
        .enumerate()
        .map(|(i, exercise)| {
            let name = Some(exercise.name.trim())
                .filter(|n| !n.is_empty())
                .unwrap_or("Exercise");
            PlannedExercise {
                number: i + 1,
                stem: sanitize_file_name(&format!("{:02}. {name}", i + 1), budget),
                exercise: exercise.clone(),
            }
        })
        .collect()
}

/// True when the workout folder holds its metadata and a finished video for
/// every exercise that has a video link.
pub fn is_downloaded(planned: &PlannedWorkout) -> bool {
    planned.dir.join(WORKOUT_FILE).is_file()
        && planned
            .exercises
            .iter()
            .filter(|e| e.exercise.url.is_some())
            .all(|e| video_exists(&planned.dir, e.number))
}

/// A `.fNNN` stem suffix marks an unmerged yt-dlp format file, not a finished video.
fn is_format_leftover(stem: &str) -> bool {
    stem.rsplit_once(".f")
        .is_some_and(|(_, digits)| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// True when a finished video for exercise `number` exists in `dir`.
pub fn video_exists(dir: &Path, number: usize) -> bool {
    let prefix = format!("{number:02}. ");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((stem, ext)) = name.rsplit_once('.') else {
            return false;
        };
        name.starts_with(&prefix)
            && VIDEO_EXTS.contains(&ext.to_ascii_lowercase().as_str())
            && !is_format_leftover(stem)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn exercise(name: &str, url: Option<&str>) -> Exercise {
        Exercise {
            name: name.to_string(),
            url: url.map(str::to_string),
            section: None,
            sets: String::new(),
            target: String::new(),
            rest: String::new(),
            notes: String::new(),
            result: Value::Null,
        }
    }

    fn workout(id: &str, date: &str, title: &str, exercises: Vec<Exercise>) -> Workout {
        Workout {
            id: id.to_string(),
            title: title.to_string(),
            date: date.to_string(),
            completed: true,
            description: String::new(),
            exercises,
            result_general: Value::Null,
        }
    }

    fn names(plan: &[PlannedWorkout]) -> Vec<&str> {
        plan.iter().map(|p| p.label.as_str()).collect()
    }

    #[test]
    fn the_course_lives_under_harbiz() {
        assert_eq!(
            course_dir(Path::new("/lib")),
            Path::new("/lib/Harbiz/Harbiz Training")
        );
    }

    #[test]
    fn a_workout_folder_is_its_date_and_title() {
        let dir = Path::new("/lib/Harbiz/Harbiz Training");
        let plan = plan_workouts(dir, &[workout("a", "2026-06-02", "Hip: mobility?", vec![])]);
        assert_eq!(names(&plan), ["2026-06-02 - Hip_ mobility_"]);
        assert_eq!(plan[0].dir, dir.join("2026-06-02 - Hip_ mobility_"));
        assert_eq!(plan[0].order, 0);
    }

    #[test]
    fn an_empty_title_still_names_the_folder() {
        let plan = plan_workouts(
            Path::new("/lib"),
            &[workout("a", "2026-06-02", " ", vec![])],
        );
        assert_eq!(names(&plan), ["2026-06-02 - Workout"]);
    }

    #[test]
    fn repeated_date_and_title_get_a_counter_in_order() {
        let w = |id| workout(id, "2026-06-02", "Same", vec![]);
        let plan = plan_workouts(Path::new("/lib"), &[w("a"), w("b")]);
        assert_eq!(names(&plan), ["2026-06-02 - Same", "2026-06-02 - Same (2)"]);
    }

    #[test]
    fn a_new_workout_never_renames_the_existing_ones() {
        let old = workout("a", "2026-06-02", "A", vec![]);
        let newer = workout("b", "2026-10-31", "B", vec![]);
        let before = plan_workouts(Path::new("/lib"), std::slice::from_ref(&old));
        let after = plan_workouts(Path::new("/lib"), &[old, newer]);
        assert_eq!(before[0].dir, after[0].dir);
    }

    #[test]
    fn exercises_are_numbered_from_one_with_a_two_digit_stem() {
        let plan = plan_workouts(
            Path::new("/lib"),
            &[workout(
                "a",
                "2026-06-02",
                "t",
                vec![
                    exercise("Warm up", Some("https://youtu.be/a")),
                    exercise("", Some("https://youtu.be/b")),
                    exercise("50% effort / hard", None),
                ],
            )],
        );
        let stems: Vec<_> = plan[0].exercises.iter().map(|e| e.stem.as_str()).collect();
        assert_eq!(
            stems,
            ["01. Warm up", "02. Exercise", "03. 50% effort _ hard"]
        );
        assert_eq!(plan[0].exercises[2].number, 3);
    }

    #[test]
    fn repeated_exercise_names_stay_unique() {
        let plan = plan_workouts(
            Path::new("/lib"),
            &[workout(
                "a",
                "2026-06-02",
                "t",
                vec![exercise("Same", None), exercise("Same", None)],
            )],
        );
        assert_eq!(plan[0].exercises[0].stem, "01. Same");
        assert_eq!(plan[0].exercises[1].stem, "02. Same");
    }

    fn planned_with_two_videos(root: &Path) -> PlannedWorkout {
        plan_workouts(
            &course_dir(root),
            &[workout(
                "a",
                "2026-06-02",
                "t",
                vec![
                    exercise("One", Some("https://youtu.be/1")),
                    exercise("No video", None),
                    exercise("Two", Some("https://youtu.be/2")),
                ],
            )],
        )
        .remove(0)
    }

    #[test]
    fn video_exists_matches_finished_files_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for name in [
            "01. One.mp4.part",
            "01. One.f137.mp4",
            "01. One.html",
            "02. Two.MKV",
            "10. Ten.webm",
        ] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        assert!(!video_exists(dir, 1));
        assert!(video_exists(dir, 2));
        assert!(video_exists(dir, 10));
        assert!(!video_exists(dir, 3));
        assert!(!video_exists(&dir.join("missing"), 2));
    }

    #[test]
    fn a_workout_is_downloaded_once_metadata_and_every_video_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let planned = planned_with_two_videos(tmp.path());
        assert!(!is_downloaded(&planned));
        std::fs::create_dir_all(&planned.dir).unwrap();
        std::fs::write(planned.dir.join(WORKOUT_FILE), "{}").unwrap();
        std::fs::write(planned.dir.join("01. One.mp4"), "x").unwrap();
        assert!(!is_downloaded(&planned), "the second video is missing");
        std::fs::write(planned.dir.join("03. Two.mp4"), "x").unwrap();
        assert!(is_downloaded(&planned));
    }

    #[test]
    fn without_metadata_a_workout_is_not_downloaded() {
        let tmp = tempfile::tempdir().unwrap();
        let planned = planned_with_two_videos(tmp.path());
        std::fs::create_dir_all(&planned.dir).unwrap();
        std::fs::write(planned.dir.join("01. One.mp4"), "x").unwrap();
        std::fs::write(planned.dir.join("03. Two.mp4"), "x").unwrap();
        assert!(!is_downloaded(&planned));
    }

    #[test]
    fn a_workout_with_no_videos_is_downloaded_with_just_its_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let planned = plan_workouts(
            &course_dir(tmp.path()),
            &[workout("a", "2026-06-02", "t", vec![exercise("x", None)])],
        )
        .remove(0);
        std::fs::create_dir_all(&planned.dir).unwrap();
        std::fs::write(planned.dir.join(WORKOUT_FILE), "{}").unwrap();
        assert!(is_downloaded(&planned));
    }
}
