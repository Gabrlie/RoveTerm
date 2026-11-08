//! The vault: SQLite storage of hosts, encrypted host secrets and known hosts.

use std::path::Path;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::crypto::FieldCipher;
use crate::model::now_millis;
use crate::{
    Host, HostDraft, HostId, HostKey, Interaction, KeyCache, KeyCacheError, KnownHostStatus,
    MasterKey, Secret, VaultError,
};

/// Version of the on-disk schema, stored in `PRAGMA user_version`. Only ever increases.
pub const SCHEMA_VERSION: i32 = 1;

const KEY_CHECK_PLAINTEXT: &[u8] = b"rove-vault-key-check";
const KEY_CHECK_AAD: &[u8] = b"rove/vault/key-check/v1";

/// Lock state of the vault's secret store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LockState {
    /// Secrets can be read and written.
    Unlocked,
    /// Secrets are unavailable; hosts remain readable.
    Locked {
        /// Why the vault is locked.
        reason: LockReason,
    },
}

/// Why the secret store is locked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum LockReason {
    /// No unlock has been attempted yet.
    NotAttempted,
    /// The platform key store could not be reached.
    StoreUnavailable(String),
    /// The user denied access to the platform key store.
    Denied,
    /// The vault holds secrets but the key store has no key, or a different one. Saved secrets
    /// can only be discarded with [`Vault::reset_secrets`].
    KeyLost,
}

/// Host, secret and known-host storage. Host metadata is always readable; secrets require the
/// vault to be unlocked with the device master key.
pub struct Vault {
    conn: Mutex<Connection>,
    cipher: RwLock<Option<FieldCipher>>,
    state: RwLock<LockState>,
    key_cache: Arc<dyn KeyCache>,
}

impl Vault {
    /// Opens or creates the database at `path` and applies schema migrations. The vault starts
    /// locked; call [`Vault::try_unlock`].
    pub fn open(path: &Path, key_cache: Arc<dyn KeyCache>) -> Result<Self, VaultError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| VaultError::Io(error.to_string()))?;
        }
        Self::with_connection(Connection::open(path)?, key_cache)
    }

    /// Opens a vault in memory, for tests.
    pub fn open_in_memory(key_cache: Arc<dyn KeyCache>) -> Result<Self, VaultError> {
        Self::with_connection(Connection::open_in_memory()?, key_cache)
    }

    fn with_connection(conn: Connection, key_cache: Arc<dyn KeyCache>) -> Result<Self, VaultError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            cipher: RwLock::new(None),
            state: RwLock::new(LockState::Locked {
                reason: LockReason::NotAttempted,
            }),
            key_cache,
        })
    }

    /// Returns the current lock state.
    pub fn lock_state(&self) -> LockState {
        self.state.read().clone()
    }

    /// Attempts to unlock without showing system authorization UI. Used at startup; failure leaves
    /// the vault locked and is not an error.
    pub fn try_unlock(&self) -> LockState {
        self.unlock_with(Interaction::Silent)
    }

    /// Attempts to unlock on behalf of a user action; the platform store may prompt.
    pub fn unlock(&self) -> LockState {
        self.unlock_with(Interaction::Allowed)
    }

    fn unlock_with(&self, interaction: Interaction) -> LockState {
        let state = match self.load_or_create_key(interaction) {
            Ok(cipher) => {
                *self.cipher.write() = Some(cipher);
                LockState::Unlocked
            }
            Err(reason) => LockState::Locked { reason },
        };
        tracing::info!(?state, "vault unlock attempted");
        *self.state.write() = state.clone();
        state
    }

    fn load_or_create_key(&self, interaction: Interaction) -> Result<FieldCipher, LockReason> {
        let key_check: Option<Vec<u8>> = self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM meta WHERE key = 'key_check'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| LockReason::StoreUnavailable(error.to_string()))?;
        let loaded = self.key_cache.load(interaction).map_err(lock_reason)?;
        match (loaded, key_check) {
            (Some(key), Some(check)) => {
                let cipher = FieldCipher::derive(&key);
                match cipher.open(&check, KEY_CHECK_AAD) {
                    Ok(plain) if plain.as_slice() == KEY_CHECK_PLAINTEXT => Ok(cipher),
                    _ => Err(LockReason::KeyLost),
                }
            }
            (None, Some(_)) => Err(LockReason::KeyLost),
            (Some(key), None) => self.adopt_key(&key),
            (None, None) => {
                let key = MasterKey::generate()
                    .map_err(|error| LockReason::StoreUnavailable(error.to_string()))?;
                self.key_cache.store(&key).map_err(lock_reason)?;
                self.adopt_key(&key)
            }
        }
    }

    /// Records the key check for a key that has no secrets encrypted under it yet.
    fn adopt_key(&self, key: &MasterKey) -> Result<FieldCipher, LockReason> {
        let cipher = FieldCipher::derive(key);
        let check = cipher
            .seal(KEY_CHECK_PLAINTEXT, KEY_CHECK_AAD)
            .map_err(|error| LockReason::StoreUnavailable(error.to_string()))?;
        self.conn
            .lock()
            .execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('key_check', ?1)",
                params![check],
            )
            .map_err(|error| LockReason::StoreUnavailable(error.to_string()))?;
        Ok(cipher)
    }

    /// Deletes every stored secret and unlocks with a fresh master key. The only way out of
    /// [`LockReason::KeyLost`]. Hosts and known hosts are kept.
    pub fn reset_secrets(&self) -> Result<LockState, VaultError> {
        let key = MasterKey::generate()?;
        self.key_cache.store(&key)?;
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM host_secrets", [])?;
            tx.execute("DELETE FROM meta WHERE key = 'key_check'", [])?;
            tx.commit()?;
        }
        let cipher = self
            .adopt_key(&key)
            .map_err(|reason| VaultError::Locked(format!("{reason:?}")))?;
        *self.cipher.write() = Some(cipher);
        *self.state.write() = LockState::Unlocked;
        Ok(LockState::Unlocked)
    }

    /// Lists hosts ordered by group, then name.
    pub fn list_hosts(&self) -> Result<Vec<Host>, VaultError> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(
            "SELECT id, name, group_name, hostname, port, username, auth, tags, created_at, updated_at
             FROM hosts ORDER BY group_name IS NOT NULL, group_name, name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], row_to_host)?;
        rows.map(|row| row.map_err(VaultError::from).and_then(|host| host))
            .collect()
    }

    /// Returns one host.
    pub fn host(&self, id: HostId) -> Result<Host, VaultError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, name, group_name, hostname, port, username, auth, tags, created_at, updated_at
             FROM hosts WHERE id = ?1",
            params![id.to_string()],
            row_to_host,
        )
        .optional()?
        .ok_or(VaultError::HostNotFound(id))?
    }

    /// Validates `draft` and stores it as a new host.
    pub fn create_host(&self, draft: HostDraft) -> Result<Host, VaultError> {
        let draft = draft.validate()?;
        let now = now_millis();
        let host = Host {
            id: HostId::new(),
            name: draft.name,
            group: draft.group,
            hostname: draft.hostname,
            port: draft.port,
            username: draft.username,
            auth: draft.auth,
            tags: draft.tags,
            created_at: now,
            updated_at: now,
        };
        self.conn.lock().execute(
            "INSERT INTO hosts (id, name, group_name, hostname, port, username, auth, tags, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                host.id.to_string(),
                host.name,
                host.group,
                host.hostname,
                host.port,
                host.username,
                serde_json::to_string(&host.auth)?,
                serde_json::to_string(&host.tags)?,
                host.created_at,
                host.updated_at
            ],
        )?;
        Ok(host)
    }

    /// Validates `draft` and replaces the host's fields. Changing the authentication method to one
    /// that stores a different secret kind deletes the stored secret.
    pub fn update_host(&self, id: HostId, draft: HostDraft) -> Result<Host, VaultError> {
        let draft = draft.validate()?;
        let previous = self.host(id)?;
        let now = now_millis();
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE hosts SET name = ?2, group_name = ?3, hostname = ?4, port = ?5, username = ?6,
                 auth = ?7, tags = ?8, updated_at = ?9 WHERE id = ?1",
                params![
                    id.to_string(),
                    draft.name,
                    draft.group,
                    draft.hostname,
                    draft.port,
                    draft.username,
                    serde_json::to_string(&draft.auth)?,
                    serde_json::to_string(&draft.tags)?,
                    now
                ],
            )?;
            if std::mem::discriminant(&previous.auth) != std::mem::discriminant(&draft.auth) {
                tx.execute(
                    "DELETE FROM host_secrets WHERE host_id = ?1",
                    params![id.to_string()],
                )?;
            }
            tx.commit()?;
        }
        self.host(id)
    }

    /// Deletes a host together with its stored secret. Works while locked.
    pub fn delete_host(&self, id: HostId) -> Result<(), VaultError> {
        let deleted = self
            .conn
            .lock()
            .execute("DELETE FROM hosts WHERE id = ?1", params![id.to_string()])?;
        if deleted == 0 {
            return Err(VaultError::HostNotFound(id));
        }
        Ok(())
    }

    fn with_cipher<T>(
        &self,
        f: impl FnOnce(&FieldCipher) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let cipher = self.cipher.read();
        match cipher.as_ref() {
            Some(cipher) => f(cipher),
            None => Err(VaultError::Locked(format!("{:?}", self.lock_state()))),
        }
    }

    /// Encrypts and stores `secret` for host `id`, replacing any existing secret. Requires the
    /// vault to be unlocked.
    pub fn set_secret(&self, id: HostId, secret: &Secret) -> Result<(), VaultError> {
        self.host(id)?;
        let plaintext = Zeroizing::new(serde_json::to_vec(secret)?);
        let sealed = self.with_cipher(|cipher| cipher.seal(&plaintext, &secret_aad(id)))?;
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO host_secrets (host_id, sealed, updated_at) VALUES (?1, ?2, ?3)",
            params![id.to_string(), sealed, now_millis()],
        )?;
        Ok(())
    }

    /// Returns the decrypted secret of host `id`, or `None` when none is stored. Requires the vault
    /// to be unlocked.
    pub fn secret(&self, id: HostId) -> Result<Option<Secret>, VaultError> {
        let sealed: Option<Vec<u8>> = self
            .conn
            .lock()
            .query_row(
                "SELECT sealed FROM host_secrets WHERE host_id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(sealed) = sealed else {
            return Ok(None);
        };
        let plaintext = self.with_cipher(|cipher| cipher.open(&sealed, &secret_aad(id)))?;
        Ok(Some(serde_json::from_slice(&plaintext)?))
    }

    /// Returns `true` when host `id` has a stored secret. Works while locked.
    pub fn has_secret(&self, id: HostId) -> Result<bool, VaultError> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT 1 FROM host_secrets WHERE host_id = ?1",
                params![id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Deletes the stored secret of host `id`, if any.
    pub fn delete_secret(&self, id: HostId) -> Result<(), VaultError> {
        self.conn.lock().execute(
            "DELETE FROM host_secrets WHERE host_id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Compares `key` with the trusted key recorded for `hostname:port`.
    pub fn check_host_key(
        &self,
        hostname: &str,
        port: u16,
        key: &HostKey,
    ) -> Result<KnownHostStatus, VaultError> {
        let recorded: Option<(String, String, String)> = self
            .conn
            .lock()
            .query_row(
                "SELECT algorithm, public_key, fingerprint FROM known_hosts WHERE hostname = ?1 AND port = ?2",
                params![hostname.to_ascii_lowercase(), port],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(match recorded {
            None => KnownHostStatus::Unknown,
            Some((_, public_key, _)) if public_key == key.public_key => KnownHostStatus::Trusted,
            Some((algorithm, _, fingerprint)) => KnownHostStatus::Mismatch {
                expected_algorithm: algorithm,
                expected_fingerprint: fingerprint,
            },
        })
    }

    /// Records `key` as the trusted key of `hostname:port`, replacing any previous key.
    pub fn trust_host_key(
        &self,
        hostname: &str,
        port: u16,
        key: &HostKey,
    ) -> Result<(), VaultError> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO known_hosts (hostname, port, algorithm, public_key, fingerprint, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                hostname.to_ascii_lowercase(),
                port,
                key.algorithm,
                key.public_key,
                key.fingerprint,
                now_millis()
            ],
        )?;
        Ok(())
    }
}

fn secret_aad(id: HostId) -> Vec<u8> {
    format!("rove/vault/host-secret/v1:{id}").into_bytes()
}

fn lock_reason(error: KeyCacheError) -> LockReason {
    match error {
        KeyCacheError::Denied => LockReason::Denied,
        KeyCacheError::Malformed => LockReason::KeyLost,
        KeyCacheError::Unavailable(message) => LockReason::StoreUnavailable(message),
    }
}

type HostRow = Result<Host, VaultError>;

fn row_to_host(row: &rusqlite::Row<'_>) -> rusqlite::Result<HostRow> {
    let id: String = row.get(0)?;
    let auth: String = row.get(6)?;
    let tags: String = row.get(7)?;
    Ok((|| {
        Ok(Host {
            id: id.parse()?,
            name: row.get(1)?,
            group: row.get(2)?,
            hostname: row.get(3)?,
            port: row.get(4)?,
            username: row.get(5)?,
            auth: serde_json::from_str(&auth)?,
            tags: serde_json::from_str(&tags)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    })())
}

fn migrate(conn: &Connection) -> Result<(), VaultError> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(VaultError::FutureSchema(version));
    }
    if version < 1 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
             CREATE TABLE hosts (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 group_name TEXT,
                 hostname TEXT NOT NULL,
                 port INTEGER NOT NULL,
                 username TEXT NOT NULL,
                 auth TEXT NOT NULL,
                 tags TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE host_secrets (
                 host_id TEXT PRIMARY KEY REFERENCES hosts(id) ON DELETE CASCADE,
                 sealed BLOB NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE known_hosts (
                 hostname TEXT NOT NULL,
                 port INTEGER NOT NULL,
                 algorithm TEXT NOT NULL,
                 public_key TEXT NOT NULL,
                 fingerprint TEXT NOT NULL,
                 added_at INTEGER NOT NULL,
                 PRIMARY KEY (hostname, port)
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    Ok(())
}
