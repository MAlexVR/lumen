use super::*;

fn store() -> ProgressStore {
    ProgressStore::open_in_memory().unwrap()
}

#[test]
fn migrations_set_schema_version() {
    assert_eq!(store().schema_version().unwrap(), 2);
}

#[test]
fn watched_toggle_is_idempotent_and_per_group() {
    let s = store();
    assert!(!s.is_watched("G", "k1").unwrap());
    s.set_watched("G", "k1", true).unwrap();
    s.set_watched("G", "k1", true).unwrap();
    assert!(s.is_watched("G", "k1").unwrap());
    assert!(!s.is_watched("H", "k1").unwrap());
    s.set_watched("G", "k1", false).unwrap();
    s.set_watched("G", "k1", false).unwrap();
    assert!(!s.is_watched("G", "k1").unwrap());
}

#[test]
fn last_lesson_is_replaced() {
    let s = store();
    assert_eq!(s.group_progress("G").unwrap().last_lesson, None);
    s.set_last_lesson("G", "a").unwrap();
    s.set_last_lesson("G", "b").unwrap();
    assert_eq!(
        s.group_progress("G").unwrap().last_lesson.as_deref(),
        Some("b")
    );
}

#[test]
fn positions_are_stored_per_lesson() {
    let s = store();
    assert_eq!(s.position("G", "a").unwrap(), None);
    s.save_position("G", "a", 30).unwrap();
    s.save_position("G", "b", 7).unwrap();
    s.save_position("G", "a", 95).unwrap();
    assert_eq!(s.position("G", "a").unwrap(), Some(95));
    assert_eq!(s.position("G", "b").unwrap(), Some(7));
    assert_eq!(s.position("H", "a").unwrap(), None);
}

#[test]
fn group_progress_snapshot_is_sorted_and_scoped() {
    let s = store();
    s.set_watched("G", "b", true).unwrap();
    s.set_watched("G", "a", true).unwrap();
    s.set_watched("H", "z", true).unwrap();
    s.save_position("G", "b", 5).unwrap();
    s.save_position("G", "a", 6).unwrap();
    s.set_last_lesson("G", "a").unwrap();
    let p = s.group_progress("G").unwrap();
    assert_eq!(p.watched, vec!["a", "b"]);
    assert_eq!(p.last_lesson.as_deref(), Some("a"));
    assert_eq!(
        p.positions,
        vec![
            Position {
                lesson_key: "a".into(),
                seconds: 6
            },
            Position {
                lesson_key: "b".into(),
                seconds: 5
            },
        ]
    );
}

#[test]
fn snapshot_serializes_camel_case() {
    let s = store();
    s.set_last_lesson("G", "a").unwrap();
    let v = serde_json::to_value(s.group_progress("G").unwrap()).unwrap();
    assert_eq!(v["lastLesson"], "a");
}

#[test]
fn data_persists_on_disk_and_reopen_does_not_remigrate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("progress.db");
    {
        let s = ProgressStore::open(&path).unwrap();
        s.set_watched("G", "k", true).unwrap();
    }
    let s = ProgressStore::open(&path).unwrap();
    assert!(s.is_watched("G", "k").unwrap());
    assert_eq!(s.schema_version().unwrap(), 2);
}

#[test]
fn newer_schema_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("progress.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    assert!(matches!(
        ProgressStore::open(&path),
        Err(ProgressError::SchemaTooNew { found: 99, .. })
    ));
}

#[test]
fn settings_round_trip_overwrite_and_remove() {
    let s = store();
    assert_eq!(s.setting("libraryRoot").unwrap(), None);
    s.set_setting("libraryRoot", "/a/ñandú").unwrap();
    assert_eq!(
        s.setting("libraryRoot").unwrap().as_deref(),
        Some("/a/ñandú")
    );
    s.set_setting("libraryRoot", "/b").unwrap();
    assert_eq!(s.setting("libraryRoot").unwrap().as_deref(), Some("/b"));
    s.remove_setting("libraryRoot").unwrap();
    s.remove_setting("libraryRoot").unwrap();
    assert_eq!(s.setting("libraryRoot").unwrap(), None);
}

#[test]
fn settings_survive_reopen_and_upgrade_from_v1() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("p.db");
    {
        // A v1 database written by an older build: only the first migration ran.
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO watched (grp, lesson_key) VALUES ('G','k')", [])
            .unwrap();
    }
    let s = ProgressStore::open(&db).unwrap();
    assert_eq!(s.schema_version().unwrap(), 2);
    assert!(s.is_watched("G", "k").unwrap());
    s.set_setting("libraryRoot", "/x").unwrap();
    drop(s);
    let s = ProgressStore::open(&db).unwrap();
    assert_eq!(s.setting("libraryRoot").unwrap().as_deref(), Some("/x"));
}

fn seed(s: &ProgressStore, group: &str, key: &str) {
    s.set_watched(group, key, true).unwrap();
    s.save_position(group, key, 30).unwrap();
    s.set_last_lesson(group, key).unwrap();
}

#[test]
fn forgetting_a_group_clears_all_its_progress_and_only_its_own() {
    let s = store();
    seed(&s, "G", "1-A/1-S/1-L");
    s.set_watched("G", "2-B/1-S/1-L", true).unwrap();
    seed(&s, "H", "1-A/1-S/1-L");
    s.forget_group("G").unwrap();
    assert_eq!(s.group_progress("G").unwrap(), GroupProgress::default());
    assert!(s.is_watched("H", "1-A/1-S/1-L").unwrap());
    assert_eq!(s.position("H", "1-A/1-S/1-L").unwrap(), Some(30));
    s.forget_group("G").unwrap();
}

#[test]
fn forgetting_a_course_matches_the_key_prefix_literally() {
    let s = store();
    seed(&s, "G", "1-Curso/1-S/1-L");
    s.set_watched("G", "1-Curso/2-S/1-L", true).unwrap();
    s.set_watched("G", "10-Curso/1-S/1-L", true).unwrap();
    s.set_watched("G", "1-Curso%/1-S/1-L", true).unwrap();
    s.set_watched("G", "1_Curso/1-S/1-L", true).unwrap();
    s.set_last_lesson("G", "10-Curso/1-S/1-L").unwrap();
    s.forget_course("G", "1-Curso/").unwrap();
    let p = s.group_progress("G").unwrap();
    assert_eq!(
        p.watched,
        vec!["1-Curso%/1-S/1-L", "10-Curso/1-S/1-L", "1_Curso/1-S/1-L"]
    );
    assert!(p.positions.is_empty());
    assert_eq!(p.last_lesson.as_deref(), Some("10-Curso/1-S/1-L"));
    // The last lesson goes away only when it belonged to the course.
    s.set_last_lesson("G", "1-Curso/1-S/1-L").unwrap();
    s.forget_course("G", "1-Curso/").unwrap();
    assert_eq!(s.group_progress("G").unwrap().last_lesson, None);
}

#[test]
fn forgetting_hotmart_groups_uses_the_qualified_id() {
    let s = store();
    seed(&s, "hotmart/Águila - Ana", "hotmart:1-Águila/1-M/1-L");
    seed(&s, "Águila - Ana", "1-A/1-S/1-L");
    s.forget_group("hotmart/Águila - Ana").unwrap();
    assert_eq!(
        s.group_progress("hotmart/Águila - Ana").unwrap(),
        GroupProgress::default()
    );
    assert!(s.is_watched("Águila - Ana", "1-A/1-S/1-L").unwrap());
}

#[test]
fn forgetting_one_lesson_touches_only_that_key() {
    let s = store();
    seed(&s, "G", "1-C/1-S/1-A");
    s.set_watched("G", "1-C/1-S/2-B", true).unwrap();
    s.save_position("G", "1-C/1-S/2-B", 7).unwrap();
    seed(&s, "H", "1-C/1-S/1-A");
    s.forget_lesson("G", "1-C/1-S/1-A").unwrap();
    let p = s.group_progress("G").unwrap();
    assert_eq!(p.watched, vec!["1-C/1-S/2-B"]);
    assert_eq!(p.positions.len(), 1);
    assert_eq!(p.last_lesson, None, "the last lesson pointed to it");
    assert!(
        s.is_watched("H", "1-C/1-S/1-A").unwrap(),
        "other groups keep theirs"
    );
    s.set_last_lesson("G", "1-C/1-S/2-B").unwrap();
    s.forget_lesson("G", "1-C/1-S/1-A").unwrap();
    assert_eq!(
        s.group_progress("G").unwrap().last_lesson.as_deref(),
        Some("1-C/1-S/2-B"),
        "a different last lesson stays"
    );
    s.forget_lesson("G", "nope").unwrap();
}
