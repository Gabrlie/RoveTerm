//! Durable, append-only event journal for RoveTerm.
//!
//! Defines the [`JournalStore`] seam, a SQLite implementation, and [`JournalPlugin`], which records
//! session lifecycle live events as journal records. See `docs/subsystems/journal.md`.

mod recorder;
mod sqlite;

use std::path::PathBuf;
use std::sync::Arc;

use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use serde::{Deserialize, Serialize};

pub use recorder::{KIND_SESSION_EXITED, KIND_SESSION_OPENED, Recorder};
pub use sqlite::{SCHEMA_VERSION, SqliteJournal};

/// A record to append.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRecord {
    /// Event kind, `domain/name` style, for example `session/opened`.
    pub kind: String,
    /// Session the record belongs to, if any.
    pub session_id: Option<String>,
    /// Event payload.
    pub payload: serde_json::Value,
}

/// A stored record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalRecord {
    /// Monotonically increasing sequence number, unique within a journal.
    pub seq: i64,
    /// Append time, Unix milliseconds.
    pub ts: i64,
    /// Event kind.
    pub kind: String,
    /// Session the record belongs to, if any.
    pub session_id: Option<String>,
    /// Event payload.
    pub payload: serde_json::Value,
}

/// Filter for [`JournalStore::read`].
#[derive(Debug, Clone, Default)]
pub struct JournalQuery {
    /// Only records with `seq` greater than this.
    pub after_seq: Option<i64>,
    /// Only records of this session.
    pub session_id: Option<String>,
    /// Only records of this kind.
    pub kind: Option<String>,
    /// Maximum number of records; absent means 1000.
    pub limit: Option<u32>,
    /// Return the newest records first (descending `seq`), for example to read the latest record
    /// of a kind with `limit: Some(1)`.
    pub newest_first: bool,
}

/// Errors returned by journal stores.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// SQLite error.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// Payload encoding error.
    #[error("payload encoding error: {0}")]
    Encoding(#[from] serde_json::Error),
    /// The database was written by a newer RoveTerm.
    #[error("journal schema version {0} is newer than this build supports")]
    FutureSchema(i32),
    /// Filesystem error.
    #[error("filesystem error: {0}")]
    Io(String),
}

/// Storage of durable events, appended to and read back; records are removed only on the user's
/// request (deleting a conversation). Definition of the `JournalStore` seam.
pub trait JournalStore: Send + Sync {
    /// Appends `record` and returns its sequence number.
    fn transaction(
        &self,
        action: &mut dyn FnMut() -> Result<(), JournalError>,
    ) -> Result<(), JournalError>;
    fn sync_receipt(&self, space: &str) -> Result<serde_json::Value, JournalError>;
    fn set_sync_receipt(&self, space: &str, value: &serde_json::Value) -> Result<(), JournalError>;

    fn append(&self, record: NewRecord) -> Result<i64, JournalError>;

    /// Appends `record` that happened at `ts` (Unix milliseconds) elsewhere, such as a record
    /// synced from another device; returns its sequence number.
    fn import(&self, ts: i64, record: NewRecord) -> Result<i64, JournalError>;

    /// Returns records matching `query` in ascending sequence order, or descending with
    /// [`JournalQuery::newest_first`].
    fn read(&self, query: &JournalQuery) -> Result<Vec<JournalRecord>, JournalError>;

    /// Removes the records of session `session_id` whose kind starts with `kind_prefix` (such as
    /// `ai/`), when the user deletes them; returns how many were removed.
    fn delete_session(&self, session_id: &str, kind_prefix: &str) -> Result<usize, JournalError>;
}

/// `[plugin.journal]` configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalConfig {
    /// Path of the journal database. The application sets it from its data directory.
    pub database: PathBuf,
}

/// Registers a [`SqliteJournal`] as `dyn JournalStore` and records session lifecycle events.
pub struct JournalPlugin;

impl Plugin for JournalPlugin {
    fn name(&self) -> &'static str {
        "journal"
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: JournalConfig = config.parse()?;
        let store: Arc<dyn JournalStore> = Arc::new(
            SqliteJournal::open(&config.database)
                .map_err(|error| KernelError::plugin("journal", error))?,
        );
        let recorder = Recorder::start(store.clone())
            .map_err(|error| KernelError::plugin("journal", error))?;
        let mut disposers = vec![ctx.provide::<dyn JournalStore>(store)?];
        disposers.extend(recorder.subscribe(ctx.events()));
        Ok(disposers)
    }
}
