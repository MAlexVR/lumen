//! Pure parsers over the JSON that Harbiz returns for a client's agenda.

use serde::Serialize;
use serde_json::Value;

use crate::error::HarbizError;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exercise {
    pub name: String,
    /// Demo video; `None` when the coach attached none or it is not http(s).
    pub url: Option<String>,
    /// Title of the block this exercise sits under, if any.
    pub section: Option<String>,
    pub sets: String,
    pub target: String,
    pub rest: String,
    pub notes: String,
    /// The client's logged result, passed through untouched.
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workout {
    pub id: String,
    pub title: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub completed: bool,
    pub description: String,
    pub exercises: Vec<Exercise>,
    pub result_general: Value,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    /// Oldest first.
    pub workouts: Vec<Workout>,
    /// Body-photo check-ins found in the agenda; they are not downloaded.
    pub photo_checkins: usize,
    /// Events of a type this adapter does not know; surfaced, never dropped silently.
    pub unsupported_events: usize,
}

/// Harbiz stores event dates around local noon, so the UTC calendar day is the
/// client's day for every timezone from UTC-11 to UTC+11.
pub fn date_from_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn invalid(what: impl Into<String>) -> HarbizError {
    HarbizError::InvalidResponse(what.into())
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn video_url(raw: String) -> Option<String> {
    (raw.starts_with("https://") || raw.starts_with("http://")).then_some(raw)
}

fn exercise(item: &Value, section: &Option<String>) -> Exercise {
    Exercise {
        name: text(item, "name_provisional"),
        url: video_url(text(item, "url_provisional")),
        section: section.clone(),
        sets: text(item, "sets"),
        target: text(item, "target"),
        rest: text(item, "rest"),
        notes: text(item, "notes"),
        result: item.get("result").cloned().unwrap_or(Value::Null),
    }
}

fn superset_member(item: &Value, section: &Option<String>) -> Exercise {
    Exercise {
        name: text(item, "supersetName_provisional"),
        url: video_url(text(item, "supersetUrl_provisional")),
        section: section.clone(),
        sets: String::new(),
        target: text(item, "supersetTarget"),
        rest: text(item, "supersetRest"),
        notes: text(item, "supersetNotes"),
        result: item.get("result").cloned().unwrap_or(Value::Null),
    }
}

fn exercises(workout: &Value) -> Vec<Exercise> {
    let items = workout
        .get("exercises")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut section = None;
    let mut out = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("block") => section = Some(text(item, "titleBlock")).filter(|t| !t.is_empty()),
            Some("superset") => out.extend(
                item.get("supersetExercises")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|member| superset_member(member, &section)),
            ),
            _ => out.push(exercise(item, &section)),
        }
    }
    out
}

fn workout(event: &Value) -> Result<Workout, HarbizError> {
    let id = text(event, "_id");
    if id.is_empty() {
        return Err(invalid("event without _id"));
    }
    let ms = event
        .pointer("/date/$date")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("event {id} without a date")))?;
    let body = event.get("workout").unwrap_or(&Value::Null);
    Ok(Workout {
        title: text(event, "title"),
        date: date_from_ms(ms),
        completed: event.get("completed").and_then(Value::as_bool) == Some(true),
        description: text(body, "description"),
        exercises: exercises(body),
        result_general: body.get("resultGeneral").cloned().unwrap_or(Value::Null),
        id,
    })
}

pub fn parse_events(value: &Value) -> Result<Parsed, HarbizError> {
    let events = value
        .as_array()
        .ok_or_else(|| invalid("the agenda is not a list"))?;
    let mut parsed = Parsed::default();
    for event in events {
        if event.get("removed").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("workout") => parsed.workouts.push(workout(event)?),
            Some("bodyPhoto") => parsed.photo_checkins += 1,
            _ => parsed.unsupported_events += 1,
        }
    }
    parsed
        .workouts
        .sort_by(|a, b| a.date.cmp(&b.date).then(a.id.cmp(&b.id)));
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workout_event(id: &str, ms: i64, title: &str, exercises: Value) -> Value {
        json!({
            "_id": id, "type": "workout", "title": title, "completed": true,
            "removed": false, "date": {"$date": ms},
            "workout": {
                "description": "Warm up first",
                "exercises": exercises,
                "resultGeneral": {"rpe": "8", "compliance": 100}
            }
        })
    }

    fn simple(name: &str, url: &str) -> Value {
        json!({
            "type": "simple", "exerciseId": "ex1_0", "name_provisional": name,
            "url_provisional": url, "sets": "2", "target": "10", "rest": "30",
            "notes": "Slow", "result": {"sets": [{"reps": "10", "weight": "0"}]}
        })
    }

    #[test]
    fn dates_are_calendar_days_in_utc() {
        assert_eq!(date_from_ms(0), "1970-01-01");
        assert_eq!(date_from_ms(1_793_293_200_000), "2026-10-29");
        assert_eq!(date_from_ms(1_782_838_800_000), "2026-06-30");
        assert_eq!(date_from_ms(1_709_211_600_000), "2024-02-29");
    }

    #[test]
    fn parses_a_simple_workout_with_every_field() {
        let v = json!([workout_event(
            "w1",
            1_793_293_200_000,
            "Leg day",
            json!([simple("Hip mobility", "https://youtu.be/abc")])
        )]);
        let parsed = parse_events(&v).unwrap();
        assert_eq!(parsed.photo_checkins, 0);
        let w = &parsed.workouts[0];
        assert_eq!(
            (w.id.as_str(), w.title.as_str(), w.date.as_str()),
            ("w1", "Leg day", "2026-10-29")
        );
        assert!(w.completed);
        assert_eq!(w.description, "Warm up first");
        assert_eq!(w.result_general["rpe"], "8");
        let e = &w.exercises[0];
        assert_eq!(e.name, "Hip mobility");
        assert_eq!(e.url.as_deref(), Some("https://youtu.be/abc"));
        assert_eq!(
            (
                e.sets.as_str(),
                e.target.as_str(),
                e.rest.as_str(),
                e.notes.as_str()
            ),
            ("2", "10", "30", "Slow")
        );
        assert_eq!(e.section, None);
        assert_eq!(e.result["sets"][0]["reps"], "10");
    }

    #[test]
    fn a_block_titles_the_exercises_that_follow() {
        let v = json!([workout_event(
            "w1",
            1_793_293_200_000,
            "t",
            json!([
                {"type": "block", "titleBlock": "Mobility", "descriptionBlock": ""},
                simple("A", "https://youtu.be/a"),
                {"type": "block", "titleBlock": "Strength"},
                simple("B", "https://youtu.be/b"),
            ])
        )]);
        let ex = &parse_events(&v).unwrap().workouts[0].exercises;
        assert_eq!(ex.len(), 2);
        assert_eq!(ex[0].section.as_deref(), Some("Mobility"));
        assert_eq!(ex[1].section.as_deref(), Some("Strength"));
    }

    #[test]
    fn a_superset_is_flattened_in_order() {
        let v = json!([workout_event(
            "w1",
            1_793_293_200_000,
            "t",
            json!([
                simple("First", "https://youtu.be/1"),
                {"type": "superset", "exerciseId": "s", "supersetExercises": [
                    {"supersetName_provisional": "Inner A", "supersetUrl_provisional": "https://youtu.be/ia",
                     "supersetTarget": "8", "supersetRest": "0", "supersetNotes": "n",
                     "result": {"sets": []}},
                    {"supersetName_provisional": "Inner B", "supersetUrl_provisional": "https://youtu.be/ib"}
                ]},
                simple("Last", "https://youtu.be/l"),
            ])
        )]);
        let ex = &parse_events(&v).unwrap().workouts[0].exercises;
        let names: Vec<_> = ex.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["First", "Inner A", "Inner B", "Last"]);
        assert_eq!(ex[1].target, "8");
        assert_eq!(ex[1].url.as_deref(), Some("https://youtu.be/ia"));
    }

    #[test]
    fn exercises_without_a_usable_url_keep_no_url() {
        let v = json!([workout_event(
            "w1",
            1_793_293_200_000,
            "t",
            json!([simple("Empty", ""), simple("Script", "javascript:alert(1)")])
        )]);
        let ex = &parse_events(&v).unwrap().workouts[0].exercises;
        assert_eq!(ex[0].url, None);
        assert_eq!(ex[1].url, None);
    }

    #[test]
    fn photo_checkins_are_counted_and_removed_events_dropped() {
        let mut removed = workout_event("gone", 1_793_293_200_000, "x", json!([]));
        removed["removed"] = json!(true);
        let v = json!([
            {"_id": "p1", "type": "bodyPhoto", "date": {"$date": 1_793_293_200_000i64}},
            removed,
            workout_event("w1", 1_793_293_200_000, "kept", json!([])),
        ]);
        let parsed = parse_events(&v).unwrap();
        assert_eq!(parsed.photo_checkins, 1);
        assert_eq!(parsed.workouts.len(), 1);
        assert_eq!(parsed.workouts[0].id, "w1");
    }

    #[test]
    fn unknown_event_types_are_counted() {
        let v = json!([{"_id": "n1", "type": "nutrition", "date": {"$date": 0}}]);
        let parsed = parse_events(&v).unwrap();
        assert_eq!(parsed.unsupported_events, 1);
        assert!(parsed.workouts.is_empty());
    }

    #[test]
    fn workouts_come_back_oldest_first() {
        let v = json!([
            workout_event("late", 1_793_293_200_000, "b", json!([])),
            workout_event("early", 1_782_838_800_000, "a", json!([])),
        ]);
        let ids: Vec<_> = parse_events(&v)
            .unwrap()
            .workouts
            .into_iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(ids, ["early", "late"]);
    }

    #[test]
    fn a_response_that_is_not_a_list_is_an_error() {
        let err = parse_events(&json!({"error": 500})).unwrap_err();
        assert!(matches!(err, HarbizError::InvalidResponse(_)));
    }

    #[test]
    fn a_workout_without_id_or_date_is_an_error_not_a_skip() {
        let mut no_id = workout_event("x", 1_793_293_200_000, "t", json!([]));
        no_id.as_object_mut().unwrap().remove("_id");
        assert!(matches!(
            parse_events(&json!([no_id])),
            Err(HarbizError::InvalidResponse(_))
        ));
        let mut no_date = workout_event("x", 1_793_293_200_000, "t", json!([]));
        no_date.as_object_mut().unwrap().remove("date");
        assert!(matches!(
            parse_events(&json!([no_date])),
            Err(HarbizError::InvalidResponse(_))
        ));
    }
}
