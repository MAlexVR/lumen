//! On-disk layout: `<library>/Harbiz/Harbiz Training/<YYYY-MM>/<date - workout>/`
//! holds one `NN. <exercise>.<ext>` video per exercise, a `Description.html`
//! and the hidden `.workout.json`. The library scanner reads a month as a
//! module and a workout folder as one class, so classes count workouts.

use std::path::{Path, PathBuf};

use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_download::files::UniqueNames;
use lumen_download::layout::path_units;

use crate::parse::{Exercise, Workout};

/// Platform folder inside the library.
pub const PLATFORM_DIR: &str = "Harbiz";
/// The single course every workout lives in.
pub const COURSE_TITLE: &str = "Harbiz Training";
/// Hidden so the library does not list it as a resource of the class.
pub const WORKOUT_FILE: &str = ".workout.json";
/// Shown by the library as the description of the class.
pub const DESCRIPTION_FILE: &str = "Description.html";
/// Name of the metadata file before it was hidden.
pub const LEGACY_WORKOUT_FILE: &str = "workout.json";
/// Hidden folder where a video that no longer matches its exercise is kept.
pub const REPLACED_DIR: &str = ".replaced";

pub(crate) const VIDEO_EXTS: [&str; 3] = ["mp4", "mkv", "webm"];

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
    /// `<course>/<YYYY-MM>`.
    pub month_dir: PathBuf,
    pub dir: PathBuf,
    /// Where earlier versions put the workout, straight under the course.
    pub legacy_dir: PathBuf,
    /// Hidden folder a new workout downloads into; it is published as `dir`
    /// when done, so the library never lists a class that is still arriving.
    pub staging_dir: PathBuf,
    /// Folder name, used as the report label.
    pub label: String,
    pub workout: Workout,
    pub exercises: Vec<PlannedExercise>,
}

/// Plans every workout, in the given order. Names depend only on a workout's own
/// date and title (plus a counter for exact repeats), never on what else is on
/// the agenda, so a new workout cannot rename the folders already downloaded.
pub fn plan_workouts(course_dir: &Path, workouts: &[Workout]) -> Vec<PlannedWorkout> {
    let mut folders = UniqueNames::default();
    workouts
        .iter()
        .enumerate()
        .map(|(order, workout)| {
            let month_dir = course_dir.join(workout.date.get(..7).unwrap_or("0000-00"));
            let folder_budget = name_budget(path_units(&month_dir), 60).min(100);
            let title = Some(workout.title.trim())
                .filter(|t| !t.is_empty())
                .unwrap_or("Workout");
            let label = folders.claim(&sanitize_file_name(
                &format!("{} - {title}", workout.date),
                folder_budget,
            ));
            let dir = month_dir.join(&label);
            let exercises = plan_exercises(&dir, &workout.exercises);
            PlannedWorkout {
                order,
                legacy_dir: course_dir.join(&label),
                staging_dir: month_dir.join(format!(".{label}.downloading")),
                month_dir,
                dir,
                label,
                workout: workout.clone(),
                exercises,
            }
        })
        .collect()
}

pub(crate) fn plan_exercises(dir: &Path, exercises: &[Exercise]) -> Vec<PlannedExercise> {
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
            .all(|e| video_exists(&planned.dir, &e.stem))
}

/// A `.fNNN` stem suffix marks an unmerged yt-dlp format file, not a finished video.
pub(crate) fn is_format_leftover(stem: &str) -> bool {
    stem.rsplit_once(".f")
        .is_some_and(|(_, digits)| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether a file stem on disk belongs to the exercise with stem `planned`.
/// Names are cut to fit the path-length budget, and that budget depends on where
/// the library lives, so a cut name still matches: same number, and one name is
/// the start of the other (long enough not to be a coincidence).
pub fn same_exercise_stem(file_stem: &str, planned: &str) -> bool {
    let (a, b) = (file_stem.to_lowercase(), planned.to_lowercase());
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    let (Some((short_number, short_name)), Some((long_number, _))) =
        (short.split_once(". "), long.split_once(". "))
    else {
        return false;
    };
    short_number == long_number
        && short_name.chars().count() >= 8
        && long.starts_with(short.as_str())
}

/// The finished video of the exercise whose file stem is `stem` (`NN. name`).
/// Matching the whole stem, not just the number, keeps a video tied to the
/// exercise it was downloaded for: if the exercise is renamed or reordered, the
/// old file no longer counts as its video.
pub fn find_video(dir: &Path, stem: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries.flatten().find_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let (file_stem, ext) = name.rsplit_once('.')?;
        (same_exercise_stem(file_stem, stem)
            && VIDEO_EXTS.contains(&ext.to_ascii_lowercase().as_str())
            && !is_format_leftover(file_stem))
        .then(|| entry.path())
    })
}

/// True when the finished video of the exercise with stem `stem` is in `dir`.
pub fn video_exists(dir: &Path, stem: &str) -> bool {
    find_video(dir, stem).is_some()
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
        assert_eq!(plan[0].month_dir, dir.join("2026-06"));
        assert_eq!(plan[0].dir, dir.join("2026-06/2026-06-02 - Hip_ mobility_"));
        assert_eq!(plan[0].legacy_dir, dir.join("2026-06-02 - Hip_ mobility_"));
        assert_eq!(plan[0].order, 0);
    }

    #[test]
    fn a_new_workout_downloads_into_a_hidden_folder_next_to_its_final_one() {
        let plan = plan_workouts(
            Path::new("/lib"),
            &[workout("a", "2026-06-02", "Legs", vec![])],
        );
        assert_eq!(
            plan[0].staging_dir,
            Path::new("/lib/2026-06/.2026-06-02 - Legs.downloading")
        );
    }

    #[test]
    fn workouts_are_grouped_by_the_month_of_their_date() {
        let plan = plan_workouts(
            Path::new("/lib"),
            &[
                workout("a", "2026-06-30", "A", vec![]),
                workout("b", "2026-07-02", "B", vec![]),
            ],
        );
        assert_eq!(plan[0].month_dir, Path::new("/lib/2026-06"));
        assert_eq!(plan[1].month_dir, Path::new("/lib/2026-07"));
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
        assert!(!video_exists(dir, "01. One"));
        assert!(video_exists(dir, "02. Two"));
        assert!(video_exists(dir, "10. Ten"));
        assert!(!video_exists(dir, "03. Three"));
        assert!(!video_exists(&dir.join("missing"), "02. Two"));
    }

    #[test]
    fn a_name_cut_to_fit_a_longer_path_still_belongs_to_its_exercise() {
        let long = "02. Calf raise with a pause and a twist";
        let cut = "02. Calf raise";
        assert!(same_exercise_stem(cut, long) && same_exercise_stem(long, cut));
        assert!(same_exercise_stem(
            "02. CALF RAISE",
            long
        ));
    }

    #[test]
    fn different_exercises_never_match_even_when_the_start_is_alike() {
        assert!(!same_exercise_stem("02. Old name", "02. New name"));
        assert!(!same_exercise_stem(
            "03. Calf raise",
            "02. Calf raise"
        ));
        assert!(
            !same_exercise_stem("02. Lunge", "02. Lunge with a long weighted twist"),
            "too short to be a cut"
        );
    }

    #[test]
    fn a_cut_video_name_is_found_for_the_full_exercise_stem() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("02. Calf raise.mp4"),
            "x",
        )
        .unwrap();
        let found = find_video(
            tmp.path(),
            "02. Calf raise with a pause and a twist",
        );
        assert!(found.is_some());
        assert!(find_video(tmp.path(), "02. Another exercise entirely").is_none());
    }

    #[test]
    fn a_video_belongs_to_its_exercise_by_full_name_not_just_by_number() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("02. Old name.mp4"), "x").unwrap();
        assert!(!video_exists(tmp.path(), "02. New name"));
        assert!(video_exists(tmp.path(), "02. Old name"));
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
