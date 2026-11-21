//! Encrypted Tailscale identity scoped to this installation.

use rusqlite::{OptionalExtension, params};
use zeroize::Zeroizing;

use crate::model::now_millis;
use crate::sync::{hex, unhex};
use crate::{Vault, VaultError};

const STATE_KEY: &str = "tailscale_state";
const STATE_AAD: &[u8] = b"rove/vault/tailscale-state/v1";

impl Vault {
    /// Reports whether an identity is stored, including while the vault is locked.
    pub fn has_tailscale_state(&self) -> Result<bool, VaultError> {
        Ok(self.conn.lock().query_row(
            "SELECT EXISTS(SELECT 1 FROM preferences WHERE key = ?1)",
            [STATE_KEY],
            |row| row.get(0),
        )?)
    }

    /// Opens the device-local identity. Requires an unlocked vault.
    pub fn tailscale_state(&self) -> Result<Option<Zeroizing<Vec<u8>>>, VaultError> {
        let conn = self.conn.lock();
        self.with_cipher(|cipher| {
            let stored: Option<String> = conn
                .query_row(
                    "SELECT value FROM preferences WHERE key = ?1",
                    [STATE_KEY],
                    |row| row.get(0),
                )
                .optional()?;
            stored
                .map(|value| {
                    let sealed = unhex(&value).ok_or(VaultError::Crypto)?;
                    cipher.open(&sealed, STATE_AAD)
                })
                .transpose()
        })
    }

    /// Seals the identity, or deletes it with `None`. Both operations require an unlocked vault.
    pub fn set_tailscale_state(&self, state: Option<&[u8]>) -> Result<(), VaultError> {
        let conn = self.conn.lock();
        self.with_cipher(|cipher| {
            if let Some(state) = state {
                let sealed = hex(&cipher.seal(state, STATE_AAD)?);
                conn.execute(
                    "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES (?1, ?2, ?3)",
                    params![STATE_KEY, sealed, now_millis()],
                )?;
            } else {
                conn.execute("DELETE FROM preferences WHERE key = ?1", [STATE_KEY])?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{LockState, MemoryKeyCache};

    #[test]
    fn identity_is_sealed_device_local_and_removed_by_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.sqlite");
        let cache = Arc::new(MemoryKeyCache::default());
        let payload = crate::MasterKey::generate().unwrap();
        let bytes: &[u8] = payload.as_bytes();
        {
            let vault = Vault::open(&path, cache.clone()).unwrap();
            assert!(!vault.has_tailscale_state().unwrap());
            assert!(matches!(
                vault.tailscale_state(),
                Err(VaultError::Locked(_))
            ));
            assert!(matches!(
                vault.set_tailscale_state(None),
                Err(VaultError::Locked(_))
            ));
            assert_eq!(vault.try_unlock(), LockState::Unlocked);
            let before = vault.sync_records().unwrap();
            vault.set_tailscale_state(Some(bytes)).unwrap();
            assert_eq!(vault.sync_records().unwrap(), before);
            let stored: String = vault
                .conn
                .lock()
                .query_row(
                    "SELECT value FROM preferences WHERE key = ?1",
                    [STATE_KEY],
                    |row| row.get(0),
                )
                .unwrap();
            let sealed = unhex(&stored).unwrap();
            assert!(!sealed.windows(bytes.len()).any(|window| window == bytes));
            assert!(matches!(
                vault.with_cipher(|cipher| cipher.open(&sealed, b"other")),
                Err(VaultError::Crypto)
            ));
        }
        let vault = Vault::open(&path, cache).unwrap();
        assert!(vault.has_tailscale_state().unwrap());
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        assert!(
            vault
                .tailscale_state()
                .unwrap()
                .is_some_and(|opened| opened.as_slice() == bytes)
        );
        assert_eq!(vault.reset_secrets().unwrap(), LockState::Unlocked);
        assert!(!vault.has_tailscale_state().unwrap());
        assert!(vault.tailscale_state().unwrap().is_none());
    }
}
