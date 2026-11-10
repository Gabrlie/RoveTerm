//! API keys of AI providers and agents, kept on this device only, each sealed with the device
//! master key. A key is stored under an agent reference: `builtin:<provider id>` or `acp:<agent id>`.

use rusqlite::{OptionalExtension, params};
use zeroize::Zeroizing;

use crate::model::now_millis;
use crate::sync::{hex, unhex};
use crate::{Vault, VaultError};

/// AAD of keys stored before keys were per agent (schema 8); such a key became `acp:codex`'s.
const LEGACY_AAD: &[u8] = b"rove/vault/ai-key/v1";
/// Where migration 9 put the single key of schema 8.
const LEGACY_ID: &str = "acp:codex";
/// Longest key accepted.
const MAX_KEY: usize = 4096;
/// Longest agent reference.
const MAX_ID: usize = 80;

fn aad(id: &str) -> Vec<u8> {
    format!("rove/vault/ai-key/v1:{id}").into_bytes()
}

fn row(id: &str) -> Result<String, VaultError> {
    let valid = !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_:.".contains(c));
    if valid {
        Ok(format!("ai_key:{id}"))
    } else {
        Err(VaultError::Invalid("not an agent reference".into()))
    }
}

impl Vault {
    /// The agent references that have a key stored. Readable while locked.
    pub fn ai_key_ids(&self) -> Result<Vec<String>, VaultError> {
        let conn = self.conn.lock();
        let mut statement =
            conn.prepare("SELECT key FROM preferences WHERE key LIKE 'ai_key:%' ORDER BY key")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .filter_map(|key| key.strip_prefix("ai_key:").map(str::to_owned))
            .collect();
        Ok(ids)
    }

    /// The key of agent `id`. Requires the vault to be unlocked; `None` when none is stored.
    pub fn ai_key(&self, id: &str) -> Result<Option<Zeroizing<String>>, VaultError> {
        let key = row(id)?;
        let stored: Option<String> = self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM preferences WHERE key = ?1",
                [&key],
                |row| row.get(0),
            )
            .optional()?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let corrupt = || VaultError::Invalid("stored AI key is corrupt".into());
        let sealed = unhex(&stored).ok_or_else(corrupt)?;
        let plain = match self.with_cipher(|cipher| cipher.open(&sealed, &aad(id))) {
            Ok(plain) => plain,
            // The key migration 9 moved to Codex keeps the AAD it was sealed with; no other row
            // may open with it.
            Err(VaultError::Crypto) if id == LEGACY_ID => {
                self.with_cipher(|cipher| cipher.open(&sealed, LEGACY_AAD))?
            }
            Err(other) => return Err(other),
        };
        Ok(Some(Zeroizing::new(
            String::from_utf8(plain.to_vec()).map_err(|_| corrupt())?,
        )))
    }

    /// Stores `key` for agent `id` sealed with the device key, or removes it with `None`. The key
    /// is trimmed and must be non-empty, at most 4096 characters, without whitespace or control
    /// characters. Storing requires the vault to be unlocked.
    pub fn set_ai_key(&self, id: &str, key: Option<&str>) -> Result<(), VaultError> {
        let row = row(id)?;
        let Some(key) = key else {
            self.conn
                .lock()
                .execute("DELETE FROM preferences WHERE key = ?1", [&row])?;
            return Ok(());
        };
        let key = key.trim();
        if key.is_empty()
            || key.len() > MAX_KEY
            || key.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(VaultError::Invalid("the API key is not valid".into()));
        }
        let sealed = hex(&self.with_cipher(|cipher| cipher.seal(key.as_bytes(), &aad(id)))?);
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![row, sealed, now_millis()],
        )?;
        Ok(())
    }
}

#[cfg(all(test, feature = "memory-key-cache"))]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{LockState, MemoryKeyCache};

    #[test]
    fn a_key_from_before_per_agent_keys_still_opens() {
        let vault = Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap();
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        // As schema 8 sealed it, under the name migration 9 gave it.
        let sealed = hex(&vault
            .with_cipher(|cipher| cipher.seal(b"sk-old", LEGACY_AAD))
            .unwrap());
        vault
            .conn
            .lock()
            .execute("INSERT INTO preferences (key, value, updated_at) VALUES ('ai_key:acp:codex', ?1, 0)", [sealed])
            .unwrap();
        assert_eq!(
            vault.ai_key("acp:codex").unwrap().unwrap().as_str(),
            "sk-old"
        );
        // The legacy binding holds for Codex's row only.
        let moved = vault
            .conn
            .lock()
            .query_row(
                "SELECT value FROM preferences WHERE key = 'ai_key:acp:codex'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        vault.set_ai_key("acp:claude", Some("ant")).unwrap();
        vault
            .conn
            .lock()
            .execute(
                "UPDATE preferences SET value = ?1 WHERE key = 'ai_key:acp:claude'",
                [moved],
            )
            .unwrap();
        assert!(
            vault.ai_key("acp:claude").is_err(),
            "a key moved to another agent does not open"
        );
    }
}
