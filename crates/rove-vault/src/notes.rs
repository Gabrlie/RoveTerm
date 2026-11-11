//! Notes about a saved host: what runs there, what to watch for, the changes worth remembering.
//! The user and the AI agents keep them; agents get them at the start of a conversation. They
//! follow the host (removed with it) and sync as `host_notes` records apart from the host, so
//! an agent updating them never overwrites the user's edit of the host.

use rusqlite::{OptionalExtension, params};

use crate::model::now_millis;
use crate::{HostId, Vault, VaultError};

/// Longest notes kept for one host, in bytes.
pub const MAX_NOTES: usize = 16 * 1024;

impl Vault {
    /// The notes of host `id`; empty when there are none.
    pub fn host_notes(&self, id: HostId) -> Result<String, VaultError> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT text FROM host_notes WHERE host_id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Replaces the notes of host `id`; empty text removes them. At most [`MAX_NOTES`] bytes,
    /// without control characters other than line breaks and tabs.
    pub fn set_host_notes(&self, id: HostId, text: &str) -> Result<(), VaultError> {
        self.host(id)?;
        let text = text.trim();
        if text.len() > MAX_NOTES {
            return Err(VaultError::Invalid(format!(
                "host notes are limited to {MAX_NOTES} bytes"
            )));
        }
        if text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(VaultError::Invalid(
                "host notes contain control characters".into(),
            ));
        }
        let conn = self.conn.lock();
        if text.is_empty() {
            conn.execute(
                "DELETE FROM host_notes WHERE host_id = ?1",
                params![id.to_string()],
            )?;
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO host_notes (host_id, text, updated_at) VALUES (?1, ?2, ?3)",
                params![id.to_string(), text, now_millis()],
            )?;
        }
        Ok(())
    }

    /// Hosts with notes and the notes, for sync.
    pub(crate) fn all_host_notes(&self) -> Result<Vec<(String, String)>, VaultError> {
        let conn = self.conn.lock();
        let mut statement =
            conn.prepare("SELECT host_id, text FROM host_notes ORDER BY host_id")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
