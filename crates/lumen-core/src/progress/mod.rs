//! Learner progress persisted in SQLite: watched lessons, last lesson per
//! group and playback position per lesson.

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum ProgressError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("database schema version {found} is newer than supported {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub lesson_key: String,
    pub seconds: u32,
}

/// Everything stored for one group, ready to send to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupProgress {
    pub watched: Vec<String>,
    pub last_lesson: Option<String>,
    pub positions: Vec<Position>,
}

pub struct ProgressStore {
    conn: Connection,
}

/// Ordered, append-only migrations. `PRAGMA user_version` records how many ran.
const MIGRATIONS: [&str; 2] = [
    "CREATE TABLE watched (
         grp TEXT NOT NULL,
         lesson_key TEXT NOT NULL,
         watched_at INTEGER NOT NULL DEFAULT (unixepoch()),
         PRIMARY KEY (grp, lesson_key)
     ) WITHOUT ROWID;
     CREATE TABLE last_lesson (
         grp TEXT PRIMARY KEY,
         lesson_key TEXT NOT NULL,
         updated_at INTEGER NOT NULL DEFAULT (unixepoch())
     ) WITHOUT ROWID;
     CREATE TABLE positions (
         grp TEXT NOT NULL,
         lesson_key TEXT NOT NULL,
         seconds INTEGER NOT NULL,
         updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
         PRIMARY KEY (grp, lesson_key)
     ) WITHOUT ROWID;",
    "CREATE TABLE settings (
         key TEXT PRIMARY KEY,
         value TEXT NOT NULL
     ) WITHOUT ROWID;",
];

impl ProgressStore {
    pub fn open(path: &Path) -> Result<Self, ProgressError> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, ProgressError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, ProgressError> {
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), ProgressError> {
        let supported = MIGRATIONS.len() as i64;
        let found = self.schema_version()?;
        if found > supported {
            return Err(ProgressError::SchemaTooNew { found, supported });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i as i64 + 1)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64, ProgressError> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    pub fn set_watched(&self, group: &str, key: &str, watched: bool) -> Result<(), ProgressError> {
        if watched {
            self.conn.execute(
                "INSERT OR IGNORE INTO watched (grp, lesson_key) VALUES (?1, ?2)",
                (group, key),
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM watched WHERE grp = ?1 AND lesson_key = ?2",
                (group, key),
            )?;
        }
        Ok(())
    }

    pub fn is_watched(&self, group: &str, key: &str) -> Result<bool, ProgressError> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM watched WHERE grp = ?1 AND lesson_key = ?2)",
            (group, key),
            |r| r.get(0),
        )?)
    }

    pub fn set_last_lesson(&self, group: &str, key: &str) -> Result<(), ProgressError> {
        self.conn.execute(
            "INSERT INTO last_lesson (grp, lesson_key) VALUES (?1, ?2)
             ON CONFLICT(grp) DO UPDATE SET lesson_key = excluded.lesson_key,
                                            updated_at = unixepoch()",
            (group, key),
        )?;
        Ok(())
    }

    pub fn save_position(&self, group: &str, key: &str, seconds: u32) -> Result<(), ProgressError> {
        self.conn.execute(
            "INSERT INTO positions (grp, lesson_key, seconds) VALUES (?1, ?2, ?3)
             ON CONFLICT(grp, lesson_key) DO UPDATE SET seconds = excluded.seconds,
                                                        updated_at = unixepoch()",
            (group, key, seconds),
        )?;
        Ok(())
    }

    pub fn position(&self, group: &str, key: &str) -> Result<Option<u32>, ProgressError> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT seconds FROM positions WHERE grp = ?1 AND lesson_key = ?2")?;
        let mut rows = stmt.query((group, key))?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    pub fn group_progress(&self, group: &str) -> Result<GroupProgress, ProgressError> {
        let watched = self
            .conn
            .prepare_cached("SELECT lesson_key FROM watched WHERE grp = ?1 ORDER BY lesson_key")?
            .query_map([group], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        let positions = self
            .conn
            .prepare_cached(
                "SELECT lesson_key, seconds FROM positions WHERE grp = ?1 ORDER BY lesson_key",
            )?
            .query_map([group], |r| {
                Ok(Position {
                    lesson_key: r.get(0)?,
                    seconds: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut stmt = self
            .conn
            .prepare_cached("SELECT lesson_key FROM last_lesson WHERE grp = ?1")?;
        let mut rows = stmt.query([group])?;
        let last_lesson = match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        };
        Ok(GroupProgress {
            watched,
            last_lesson,
            positions,
        })
    }

    /// Drops every stored entry of `group` (watched, last lesson, positions)
    /// in one transaction, e.g. after its folder was deleted.
    pub fn forget_group(&self, group: &str) -> Result<(), ProgressError> {
        let tx = self.conn.unchecked_transaction()?;
        for table in ["watched", "last_lesson", "positions"] {
            tx.execute(&format!("DELETE FROM {table} WHERE grp = ?1"), [group])?;
        }
        Ok(tx.commit()?)
    }

    /// Drops the entries of `group` whose lesson key starts with `key_prefix`
    /// (a course: `"<index>-<title>/"`). The prefix is matched literally.
    pub fn forget_course(&self, group: &str, key_prefix: &str) -> Result<(), ProgressError> {
        let tx = self.conn.unchecked_transaction()?;
        for table in ["watched", "last_lesson", "positions"] {
            tx.execute(
                &format!(
                    "DELETE FROM {table} WHERE grp = ?1 AND substr(lesson_key, 1, length(?2)) = ?2"
                ),
                (group, key_prefix),
            )?;
        }
        Ok(tx.commit()?)
    }

    /// Reads an application setting (a small key/value preference).
    pub fn setting(&self, key: &str) -> Result<Option<String>, ProgressError> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), ProgressError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, value),
        )?;
        Ok(())
    }

    pub fn remove_setting(&self, key: &str) -> Result<(), ProgressError> {
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }
}
