//! What the player needs to guide a workout: for each exercise of a class, how
//! many sets and how many repetitions it asks for, read from the class's
//! `.workout.json`.

use std::path::Path;

use serde::Serialize;

use crate::parse::Workout;
use crate::planner::WORKOUT_FILE;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExercisePlan {
    /// Position in the workout, 1-based: the `NN` of the video file name.
    pub number: usize,
    pub name: String,
    /// Sets the coach asked for, when the agenda says.
    pub sets: Option<u32>,
    /// Repetitions (or time) per set, as the coach wrote it.
    pub target: String,
    /// Rest between sets in seconds, when given.
    pub rest: Option<u32>,
}

fn count(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|n| *n > 0)
}

pub fn exercise_plan(class_dir: &Path) -> Vec<ExercisePlan> {
    let Some(workout) = std::fs::read_to_string(class_dir.join(WORKOUT_FILE))
        .ok()
        .and_then(|json| serde_json::from_str::<Workout>(&json).ok())
    else {
        return Vec::new();
    };
    workout
        .exercises
        .iter()
        .enumerate()
        .map(|(i, e)| ExercisePlan {
            number: i + 1,
            name: e.name.trim().to_string(),
            sets: count(&e.sets),
            target: e.target.trim().to_string(),
            rest: count(&e.rest),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Exercise;
    use serde_json::Value;

    fn exercise(name: &str, sets: &str, target: &str, rest: &str) -> Exercise {
        Exercise {
            name: name.to_string(),
            url: None,
            section: None,
            sets: sets.to_string(),
            target: target.to_string(),
            rest: rest.to_string(),
            notes: String::new(),
            result: Value::Null,
        }
    }

    fn class_with(exercises: Vec<Exercise>) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let workout = Workout {
            id: "w".to_string(),
            title: "t".to_string(),
            date: "2026-06-02".to_string(),
            completed: true,
            description: String::new(),
            exercises,
            result_general: Value::Null,
        };
        std::fs::write(
            tmp.path().join(WORKOUT_FILE),
            serde_json::to_string(&workout).unwrap(),
        )
        .unwrap();
        tmp
    }

    #[test]
    fn each_exercise_has_its_number_sets_repetitions_and_rest() {
        let tmp = class_with(vec![
            exercise("Squat", "3", "10", "30"),
            exercise("Plank", "2", "45", "60"),
        ]);
        assert_eq!(
            exercise_plan(tmp.path()),
            [
                ExercisePlan {
                    number: 1,
                    name: "Squat".into(),
                    sets: Some(3),
                    target: "10".into(),
                    rest: Some(30)
                },
                ExercisePlan {
                    number: 2,
                    name: "Plank".into(),
                    sets: Some(2),
                    target: "45".into(),
                    rest: Some(60)
                },
            ]
        );
    }

    #[test]
    fn missing_or_nonsense_values_become_none_instead_of_failing() {
        let tmp = class_with(vec![
            exercise("A", "", " 12 ", "abc"),
            exercise("B", "0", "", ""),
        ]);
        let plan = exercise_plan(tmp.path());
        assert_eq!(
            (plan[0].sets, plan[0].target.as_str(), plan[0].rest),
            (None, "12", None)
        );
        assert_eq!(plan[1].sets, None, "zero sets means unknown");
    }

    #[test]
    fn a_class_without_readable_metadata_has_no_plan() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(exercise_plan(tmp.path()).is_empty());
        std::fs::write(tmp.path().join(WORKOUT_FILE), "not json").unwrap();
        assert!(exercise_plan(tmp.path()).is_empty());
    }

    #[test]
    fn it_serializes_in_camel_case() {
        let plan = ExercisePlan {
            number: 1,
            name: "A".into(),
            sets: Some(2),
            target: "10".into(),
            rest: None,
        };
        let json = serde_json::to_value(plan).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"number": 1, "name": "A", "sets": 2, "target": "10", "rest": null})
        );
    }
}
