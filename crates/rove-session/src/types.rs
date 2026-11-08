//! Identifiers and value types shared by every terminal provider.

use std::fmt;
use std::str::FromStr;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::SessionError;

/// Opaque identifier of a live terminal session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(Uuid);

impl SessionId {
    /// Generates a new time-ordered identifier.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SessionId {
    type Err = SessionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|_| SessionError::InvalidId(s.chars().take(64).collect()))
    }
}

/// Terminal dimensions in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermSize {
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
}

impl TermSize {
    /// Largest accepted value for either dimension.
    pub const MAX: u16 = 1000;

    /// Validates dimensions: both must be in `1..=TermSize::MAX`.
    pub fn new(cols: u16, rows: u16) -> Result<Self, SessionError> {
        if (1..=Self::MAX).contains(&cols) && (1..=Self::MAX).contains(&rows) {
            Ok(Self { cols, rows })
        } else {
            Err(SessionError::InvalidSize { cols, rows })
        }
    }
}

impl Default for TermSize {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

/// How a terminal session ended.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExitStatus {
    /// Process exit code, when the provider reports one.
    pub code: Option<i32>,
    /// Signal name, when the process was terminated by a signal.
    pub signal: Option<String>,
}

/// Output produced by a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalOutput {
    /// Raw terminal bytes.
    Data(Bytes),
    /// The session ended; no output follows.
    Exit(ExitStatus),
}

/// What kind of target a session is connected to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionKind {
    /// A local PTY.
    Local {
        /// Shell program path.
        shell: String,
    },
    /// An SSH channel.
    Ssh {
        /// Saved host identifier, absent for quick connections.
        host_id: Option<String>,
        /// `user@host:port` display address.
        address: String,
    },
}

/// Descriptive metadata for a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Target kind.
    #[serde(flatten)]
    pub kind: SessionKind,
    /// Initial tab title.
    pub title: String,
}
