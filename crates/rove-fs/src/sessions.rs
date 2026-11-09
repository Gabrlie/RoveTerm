//! Open file systems addressed by [`FsId`], so IPC callers can refer to them.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{FileSystem, FsError};

/// Identifier of an open file system session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FsId(Uuid);

impl FsId {
    /// Generates a new time-ordered identifier.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for FsId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for FsId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for FsId {
    type Err = FsError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(text)
            .map(Self)
            .map_err(|_| FsError::InvalidPath(format!("invalid file system id {text:?}")))
    }
}

/// An open session as listed by [`FsSessions::list`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FsSummary {
    /// Session identifier.
    pub id: FsId,
    /// Provider label.
    pub label: String,
}

/// Table of open file systems. Removing an entry drops the table's reference; the provider closes
/// once running transfers release theirs.
#[derive(Default)]
pub struct FsSessions {
    open: Mutex<HashMap<FsId, Arc<dyn FileSystem>>>,
}

impl FsSessions {
    /// Adds `fs` and returns its identifier.
    pub fn insert(&self, fs: Arc<dyn FileSystem>) -> FsId {
        let id = FsId::new();
        self.open.lock().insert(id, fs);
        id
    }

    /// Returns the file system `id`, or [`FsError::Closed`].
    pub fn get(&self, id: FsId) -> Result<Arc<dyn FileSystem>, FsError> {
        self.open.lock().get(&id).cloned().ok_or(FsError::Closed)
    }

    /// Removes `id`. Returns `false` when it was not open.
    pub fn remove(&self, id: FsId) -> bool {
        self.open.lock().remove(&id).is_some()
    }

    /// Lists open sessions.
    pub fn list(&self) -> Vec<FsSummary> {
        self.open
            .lock()
            .iter()
            .map(|(id, fs)| FsSummary {
                id: *id,
                label: fs.label(),
            })
            .collect()
    }

    /// Removes every session.
    pub fn clear(&self) {
        self.open.lock().clear();
    }
}
