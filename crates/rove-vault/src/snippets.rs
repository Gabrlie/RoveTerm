//! Saved commands ("snippets") that can be sent to a terminal. They carry no secrets.

use std::fmt;
use std::str::FromStr;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::now_millis;
use crate::{Vault, VaultError};

/// Identifier of a snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnippetId(Uuid);

impl fmt::Display for SnippetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SnippetId {
    type Err = VaultError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self).map_err(|_| {
            VaultError::Invalid(format!(
                "invalid snippet id `{}`",
                s.chars().take(64).collect::<String>()
            ))
        })
    }
}

/// A saved command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    /// Identifier.
    pub id: SnippetId,
    /// Display name.
    pub name: String,
    /// Text sent to the terminal; may span several lines.
    pub command: String,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
    /// Last modification time, Unix milliseconds.
    pub updated_at: i64,
}

/// Input for creating or updating a snippet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetDraft {
    /// Display name; empty uses the first line of `command`.
    #[serde(default)]
    pub name: String,
    /// Text sent to the terminal.
    pub command: String,
}

impl SnippetDraft {
    /// Trims the name, defaults it, and checks lengths.
    pub fn validate(mut self) -> Result<Self, VaultError> {
        self.name = self.name.trim().to_owned();
        if self.command.trim().is_empty() || self.command.len() > 16 * 1024 {
            return Err(VaultError::Invalid(
                "command must be 1 byte to 16 KiB".into(),
            ));
        }
        if self.name.is_empty() {
            self.name = self
                .command
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .chars()
                .take(64)
                .collect();
        }
        if self.name.chars().count() > 64 {
            return Err(VaultError::Invalid("snippet name is too long".into()));
        }
        Ok(self)
    }
}

type SnippetRow = Result<Snippet, VaultError>;

fn row_to_snippet(row: &rusqlite::Row<'_>) -> rusqlite::Result<SnippetRow> {
    let id: String = row.get(0)?;
    Ok((|| {
        Ok(Snippet {
            id: id.parse()?,
            name: row.get(1)?,
            command: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    })())
}

impl Vault {
    /// Lists snippets by name.
    pub fn list_snippets(&self) -> Result<Vec<Snippet>, VaultError> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(
            "SELECT id, name, command, created_at, updated_at FROM snippets
             ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], row_to_snippet)?;
        rows.map(|row| row.map_err(VaultError::from).and_then(|snippet| snippet))
            .collect()
    }

    fn snippet(&self, id: SnippetId) -> Result<Snippet, VaultError> {
        self.conn
            .lock()
            .query_row(
                "SELECT id, name, command, created_at, updated_at FROM snippets WHERE id = ?1",
                params![id.to_string()],
                row_to_snippet,
            )
            .optional()?
            .ok_or(VaultError::SnippetNotFound(id))?
    }

    /// Adds a snippet.
    pub fn create_snippet(&self, draft: SnippetDraft) -> Result<Snippet, VaultError> {
        let draft = draft.validate()?;
        let id = SnippetId(Uuid::now_v7());
        let now = now_millis();
        self.conn.lock().execute(
            "INSERT INTO snippets (id, name, command, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id.to_string(), draft.name, draft.command, now],
        )?;
        self.snippet(id)
    }

    /// Replaces a snippet's name and command.
    pub fn update_snippet(
        &self,
        id: SnippetId,
        draft: SnippetDraft,
    ) -> Result<Snippet, VaultError> {
        let draft = draft.validate()?;
        let updated = self.conn.lock().execute(
            "UPDATE snippets SET name = ?2, command = ?3, updated_at = ?4 WHERE id = ?1",
            params![id.to_string(), draft.name, draft.command, now_millis()],
        )?;
        if updated == 0 {
            return Err(VaultError::SnippetNotFound(id));
        }
        self.snippet(id)
    }

    /// Deletes a snippet.
    pub fn delete_snippet(&self, id: SnippetId) -> Result<(), VaultError> {
        let deleted = self.conn.lock().execute(
            "DELETE FROM snippets WHERE id = ?1",
            params![id.to_string()],
        )?;
        if deleted == 0 {
            return Err(VaultError::SnippetNotFound(id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_default_to_the_first_command_line() {
        let draft = SnippetDraft {
            name: "  ".into(),
            command: "\n  docker ps -a\nexit\n".into(),
        }
        .validate()
        .unwrap();
        assert_eq!(draft.name, "docker ps -a");
        assert!(
            SnippetDraft {
                name: String::new(),
                command: "  \n".into()
            }
            .validate()
            .is_err()
        );
    }
}
