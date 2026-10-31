//! Rewrites, without any network, the notes of the workouts already on disk
//! from the `.workout.json` each class keeps, so a library downloaded by an
//! older version shows per-exercise notes without running an update.

use std::path::{Path, PathBuf};

use crate::downloader::write_notes;
use crate::parse::Workout;
use crate::planner::{course_dir, plan_exercises, PlannedWorkout, WORKOUT_FILE};

fn visible_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_dir())
                && !e.file_name().to_string_lossy().starts_with('.')
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// Number of files written. Zero means everything was already up to date.
pub fn refresh_notes(library_root: &Path) -> usize {
    let mut written = 0;
    for month in visible_dirs(&course_dir(library_root)) {
        for class in visible_dirs(&month) {
            let Some(workout) = std::fs::read_to_string(class.join(WORKOUT_FILE))
                .ok()
                .and_then(|json| serde_json::from_str::<Workout>(&json).ok())
            else {
                continue;
            };
            let planned = PlannedWorkout {
                order: 0,
                month_dir: month.clone(),
                legacy_dir: class.clone(),
                staging_dir: class.clone(),
                label: class
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                exercises: plan_exercises(&class, &workout.exercises),
                dir: class.clone(),
                workout,
            };
            written += write_notes(&planned, &class, &mut Vec::new());
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Exercise, Workout};
    use crate::planner::{course_dir, WORKOUT_FILE};
    use serde_json::{json, Value};

    fn exercise(name: &str, url: Option<&str>) -> Exercise {
        Exercise {
            name: name.to_string(),
            url: url.map(str::to_string),
            section: None,
            sets: "2".to_string(),
            target: "10".to_string(),
            rest: "30".to_string(),
            notes: format!("How to do {name}"),
            result: json!({"sets": [{"reps": "10", "weight": "0"}]}),
        }
    }

    /// A class folder as an older version left it: the metadata, one video and
    /// one page that lists every exercise.
    fn old_class(root: &Path) -> std::path::PathBuf {
        let dir = course_dir(root).join("2026-06/2026-06-02 - Legs");
        std::fs::create_dir_all(&dir).unwrap();
        let workout = Workout {
            id: "w1".to_string(),
            title: "Legs".to_string(),
            date: "2026-06-02".to_string(),
            completed: true,
            description: "Warm up".to_string(),
            exercises: vec![
                exercise("Squat", Some("https://youtu.be/aaaaaaaaaaa")),
                exercise("Lunge", Some("https://youtu.be/bbbbbbbbbbb")),
            ],
            result_general: Value::Null,
        };
        std::fs::write(
            dir.join(WORKOUT_FILE),
            serde_json::to_string_pretty(&workout).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("01. Squat.mp4"), "video").unwrap();
        std::fs::write(
            dir.join("Description.html"),
            "<ol><li>Squat</li><li>Lunge</li></ol>",
        )
        .unwrap();
        dir
    }

    #[test]
    fn an_old_class_gets_a_note_per_video_and_a_general_page_without_the_exercise_list() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = old_class(tmp.path());
        assert!(refresh_notes(tmp.path()) > 0);
        let note = std::fs::read_to_string(dir.join("01. Squat.html")).unwrap();
        assert!(note.contains("How to do Squat") && !note.contains("Lunge"));
        assert!(!dir.join("02. Lunge.html").exists(), "no video, no note");
        let page = std::fs::read_to_string(dir.join("Description.html")).unwrap();
        assert!(page.contains("Legs") && page.contains("Warm up"));
        assert!(
            page.contains("How to do Lunge"),
            "the exercise without a video is described here"
        );
        assert!(!page.contains("How to do Squat"));
    }

    #[test]
    fn running_it_again_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        old_class(tmp.path());
        assert!(refresh_notes(tmp.path()) > 0);
        assert_eq!(refresh_notes(tmp.path()), 0);
    }

    #[test]
    fn a_library_without_harbiz_or_with_unreadable_metadata_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(refresh_notes(tmp.path()), 0);
        let dir = course_dir(tmp.path()).join("2026-06/broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(WORKOUT_FILE), "not json").unwrap();
        std::fs::write(dir.join("Description.html"), "keep").unwrap();
        assert_eq!(refresh_notes(tmp.path()), 0);
        assert_eq!(
            std::fs::read_to_string(dir.join("Description.html")).unwrap(),
            "keep"
        );
    }
}
