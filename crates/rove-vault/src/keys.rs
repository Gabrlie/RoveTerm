//! The key library: named SSH keys that hosts refer to by id. Names and public halves are readable
//! while locked; private keys and passphrases are sealed like host secrets.
//!
//! The vault does not parse keys. Callers derive `algorithm`, `public_key` and `fingerprint` from
//! the private key (rove-ssh) and pass them in [`KeyDraft`].

use std::fmt;
use std::str::FromStr;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::model::now_millis;
use crate::{AuthMethod, Vault, VaultError};

/// Identifier of a library key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyId(Uuid);

impl KeyId {
    /// Generates a new time-ordered identifier.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for KeyId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for KeyId {
    type Err = VaultError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self).map_err(|_| {
            VaultError::Invalid(format!(
                "invalid key id `{}`",
                s.chars().take(64).collect::<String>()
            ))
        })
    }
}

/// A library key as listed. Contains no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshKey {
    /// Identifier.
    pub id: KeyId,
    /// Display name, unique in the library.
    pub name: String,
    /// Key algorithm, for example `ssh-ed25519`.
    pub algorithm: String,
    /// Public key in OpenSSH single-line form.
    pub public_key: String,
    /// `SHA256:` fingerprint.
    pub fingerprint: String,
    /// `false` after the secrets were reset: the private key must be imported again.
    pub has_private: bool,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
    /// Last modification time, Unix milliseconds.
    pub updated_at: i64,
}

/// The private half of a library key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySecret {
    /// Private key in OpenSSH or PEM text form.
    pub private_key: Zeroizing<String>,
    /// Passphrase of an encrypted private key.
    pub passphrase: Option<Zeroizing<String>>,
}

impl fmt::Debug for KeySecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeySecret(..)")
    }
}

/// Input for adding a key or replacing its private half.
#[derive(Debug, Clone)]
pub struct KeyDraft {
    /// Display name.
    pub name: String,
    /// Algorithm derived from the private key.
    pub algorithm: String,
    /// Public key derived from the private key.
    pub public_key: String,
    /// Fingerprint derived from the private key.
    pub fingerprint: String,
    /// The private half.
    pub secret: KeySecret,
}

/// Trims `name` and checks its length.
fn valid_name(name: &str) -> Result<String, VaultError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(VaultError::Invalid(
            "key name must be 1-64 characters".into(),
        ));
    }
    Ok(name.to_owned())
}

fn key_aad(id: KeyId) -> Vec<u8> {
    format!("rove/vault/ssh-key/v1:{id}").into_bytes()
}

type KeyRow = Result<SshKey, VaultError>;

fn row_to_key(row: &rusqlite::Row<'_>) -> rusqlite::Result<KeyRow> {
    let id: String = row.get(0)?;
    Ok((|| {
        Ok(SshKey {
            id: id.parse()?,
            name: row.get(1)?,
            algorithm: row.get(2)?,
            public_key: row.get(3)?,
            fingerprint: row.get(4)?,
            has_private: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })())
}

const KEY_COLUMNS: &str =
    "id, name, algorithm, public_key, fingerprint, sealed IS NOT NULL, created_at, updated_at";

impl Vault {
    /// Lists library keys by name.
    pub fn list_ssh_keys(&self) -> Result<Vec<SshKey>, VaultError> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(&format!(
            "SELECT {KEY_COLUMNS} FROM ssh_keys ORDER BY name COLLATE NOCASE"
        ))?;
        let rows = statement.query_map([], row_to_key)?;
        rows.map(|row| row.map_err(VaultError::from).and_then(|key| key))
            .collect()
    }

    /// Returns one library key.
    pub fn ssh_key(&self, id: KeyId) -> Result<SshKey, VaultError> {
        self.conn
            .lock()
            .query_row(
                &format!("SELECT {KEY_COLUMNS} FROM ssh_keys WHERE id = ?1"),
                params![id.to_string()],
                row_to_key,
            )
            .optional()?
            .ok_or(VaultError::KeyNotFound(id))?
    }

    /// Adds a key. Requires the vault to be unlocked.
    pub fn create_ssh_key(&self, draft: KeyDraft) -> Result<SshKey, VaultError> {
        let name = valid_name(&draft.name)?;
        self.check_unique_name(&name, None)?;
        let id = KeyId::new();
        let sealed = self.seal_key(id, &draft.secret)?;
        let now = now_millis();
        self.conn.lock().execute(
            "INSERT INTO ssh_keys (id, name, algorithm, public_key, fingerprint, sealed, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                id.to_string(),
                name,
                draft.algorithm,
                draft.public_key,
                draft.fingerprint,
                sealed,
                now
            ],
        )?;
        self.ssh_key(id)
    }

    /// Renames a key. Works while locked.
    pub fn rename_ssh_key(&self, id: KeyId, name: &str) -> Result<SshKey, VaultError> {
        let name = valid_name(name)?;
        self.check_unique_name(&name, Some(id))?;
        let updated = self.conn.lock().execute(
            "UPDATE ssh_keys SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), name, now_millis()],
        )?;
        if updated == 0 {
            return Err(VaultError::KeyNotFound(id));
        }
        self.ssh_key(id)
    }

    /// Replaces the name and both halves of a key, for example after importing it again. Hosts
    /// using the key keep using it. Requires the vault to be unlocked.
    pub fn replace_ssh_key(&self, id: KeyId, draft: KeyDraft) -> Result<SshKey, VaultError> {
        let name = valid_name(&draft.name)?;
        self.ssh_key(id)?;
        self.check_unique_name(&name, Some(id))?;
        let sealed = self.seal_key(id, &draft.secret)?;
        self.conn.lock().execute(
            "UPDATE ssh_keys SET name = ?2, algorithm = ?3, public_key = ?4, fingerprint = ?5,
             sealed = ?6, updated_at = ?7 WHERE id = ?1",
            params![
                id.to_string(),
                name,
                draft.algorithm,
                draft.public_key,
                draft.fingerprint,
                sealed,
                now_millis()
            ],
        )?;
        self.ssh_key(id)
    }

    /// Deletes a key. Fails with [`VaultError::KeyInUse`] naming the hosts that still use it.
    pub fn delete_ssh_key(&self, id: KeyId) -> Result<(), VaultError> {
        self.ssh_key(id)?;
        let users: Vec<String> = self
            .list_hosts()?
            .into_iter()
            .filter(|host| host.auth == AuthMethod::Key { key: id })
            .map(|host| host.name)
            .collect();
        if !users.is_empty() {
            return Err(VaultError::KeyInUse(users));
        }
        self.conn.lock().execute(
            "DELETE FROM ssh_keys WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Returns the private half of a key. Requires the vault to be unlocked. Fails with
    /// [`VaultError::KeyNotFound`] for an unknown key and with [`VaultError::Invalid`] when its
    /// private half was discarded by a reset.
    pub fn ssh_key_secret(&self, id: KeyId) -> Result<KeySecret, VaultError> {
        let sealed: Option<Option<Vec<u8>>> = self
            .conn
            .lock()
            .query_row(
                "SELECT sealed FROM ssh_keys WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let sealed = sealed.ok_or(VaultError::KeyNotFound(id))?.ok_or_else(|| {
            VaultError::Invalid("the private key was discarded; import it again".into())
        })?;
        let plaintext = self.with_cipher(|cipher| cipher.open(&sealed, &key_aad(id)))?;
        Ok(serde_json::from_slice(&plaintext)?)
    }

    fn seal_key(&self, id: KeyId, secret: &KeySecret) -> Result<Vec<u8>, VaultError> {
        let plaintext = Zeroizing::new(serde_json::to_vec(secret)?);
        self.with_cipher(|cipher| cipher.seal(&plaintext, &key_aad(id)))
    }

    fn check_unique_name(&self, name: &str, except: Option<KeyId>) -> Result<(), VaultError> {
        let clash: Option<String> = self
            .conn
            .lock()
            .query_row(
                "SELECT id FROM ssh_keys WHERE name = ?1 COLLATE NOCASE AND id IS NOT ?2",
                params![name, except.map(|id| id.to_string())],
                |row| row.get(0),
            )
            .optional()?;
        match clash {
            Some(_) => Err(VaultError::Invalid(format!(
                "a key named `{name}` already exists"
            ))),
            None => Ok(()),
        }
    }
}
