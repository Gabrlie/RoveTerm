//! SQLite implementation of [`JournalStore`].

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{Connection, params_from_iter, types::Value};

use crate::{JournalError, JournalQuery, JournalRecord, JournalStore, NewRecord};

/// Version of the on-disk schema, stored in `PRAGMA user_version`. Only ever increases.
pub const SCHEMA_VERSION: i32 = 1;

const DEFAULT_LIMIT: u32 = 1000;

/// Journal stored in one SQLite table.
pub struct SqliteJournal {
    conn: Mutex<Connection>,
}

impl SqliteJournal {
    /// Opens or creates the journal at `path`.
    pub fn open(path: &Path) -> Result<Self, JournalError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| JournalError::Io(error.to_string()))?;
        }
        Self::with_connection(Connection::open(path)?)
    }

    /// Opens a journal in memory, for tests.
    pub fn open_in_memory() -> Result<Self, JournalError> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<Self, JournalError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(JournalError::FutureSchema(version));
        }
        if version < 1 {
            conn.execute_batch(
                "BEGIN;
                 CREATE TABLE events (
                     seq INTEGER PRIMARY KEY AUTOINCREMENT,
                     ts INTEGER NOT NULL,
                     kind TEXT NOT NULL,
                     session_id TEXT,
                     payload TEXT NOT NULL
                 );
                 CREATE INDEX events_session ON events (session_id, seq);
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

impl JournalStore for SqliteJournal {
    fn append(&self, record: NewRecord) -> Result<i64, JournalError> {
        let payload = serde_json::to_string(&record.payload)?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO events (ts, kind, session_id, payload) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![now_millis(), record.kind, record.session_id, payload],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn read(&self, query: &JournalQuery) -> Result<Vec<JournalRecord>, JournalError> {
        let mut sql =
            String::from("SELECT seq, ts, kind, session_id, payload FROM events WHERE 1 = 1");
        let mut values: Vec<Value> = Vec::new();
        if let Some(after) = query.after_seq {
            sql.push_str(" AND seq > ?");
            values.push(Value::Integer(after));
        }
        if let Some(session) = &query.session_id {
            sql.push_str(" AND session_id = ?");
            values.push(Value::Text(session.clone()));
        }
        if let Some(kind) = &query.kind {
            sql.push_str(" AND kind = ?");
            values.push(Value::Text(kind.clone()));
        }
        sql.push_str(" ORDER BY seq LIMIT ?");
        values.push(Value::Integer(i64::from(
            query.limit.unwrap_or(DEFAULT_LIMIT),
        )));

        let conn = self.conn.lock();
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (seq, ts, kind, session_id, payload) = row?;
            Ok(JournalRecord {
                seq,
                ts,
                kind,
                session_id,
                payload: serde_json::from_str(&payload)?,
            })
        })
        .collect()
    }
}
