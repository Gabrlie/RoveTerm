//! The AI agent's API key, kept on this device only and sealed with the device master key.

use rusqlite::{OptionalExtension, params};
use zeroize::Zeroizing;

use crate::model::now_millis;
use crate::sync::{hex, unhex};
use crate::{Vault, VaultError};

const AI_KEY_AAD: &[u8] = b"rove/vault/ai-key/v1";
/// Longest key accepted.
const MAX_KEY: usize = 4096;

impl Vault {
    /// Whether an API key is stored. Readable while locked.
    pub fn has_ai_key(&self) -> Result<bool, VaultError> {
        Ok(self.stored_ai_key()?.is_some())
    }

    fn stored_ai_key(&self) -> Result<Option<String>, VaultError> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM preferences WHERE key = 'ai_key'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The stored API key. Requires the vault to be unlocked; `None` when none is stored.
    pub fn ai_key(&self) -> Result<Option<Zeroizing<String>>, VaultError> {
        let Some(stored) = self.stored_ai_key()? else {
            return Ok(None);
        };
        let sealed =
            unhex(&stored).ok_or_else(|| VaultError::Invalid("stored AI key is corrupt".into()))?;
        let plain = self.with_cipher(|cipher| cipher.open(&sealed, AI_KEY_AAD))?;
        let key = String::from_utf8(plain.to_vec())
            .map_err(|_| VaultError::Invalid("stored AI key is corrupt".into()))?;
        Ok(Some(Zeroizing::new(key)))
    }

    /// Stores `key` sealed with the device key, or removes it with `None`. The key is trimmed and
    /// must be non-empty, at most 4096 characters, without whitespace or control characters.
    /// Storing requires the vault to be unlocked.
    pub fn set_ai_key(&self, key: Option<&str>) -> Result<(), VaultError> {
        let Some(key) = key else {
            self.conn
                .lock()
                .execute("DELETE FROM preferences WHERE key = 'ai_key'", [])?;
            return Ok(());
        };
        let key = key.trim();
        if key.is_empty()
            || key.len() > MAX_KEY
            || key.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(VaultError::Invalid("the API key is not valid".into()));
        }
        let sealed = hex(&self.with_cipher(|cipher| cipher.seal(key.as_bytes(), AI_KEY_AAD))?);
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES ('ai_key', ?1, ?2)",
            params![sealed, now_millis()],
        )?;
        Ok(())
    }
}
