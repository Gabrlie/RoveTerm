//! The vault: SQLite storage of hosts, encrypted host secrets and known hosts.

use std::path::Path;
use std::sync::Arc;

use parking_lot::{ReentrantMutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::crypto::FieldCipher;
use crate::model::now_millis;
use crate::{
    AuthMethod, DevicePrefs, ForwardDraft, ForwardId, ForwardRule, Host, HostDraft, HostId,
    HostKey, HostOs, HostRoute, Interaction, KeyCache, KeyCacheError, KnownHostStatus, MasterKey,
    Preferences, Secret, VaultError,
};

/// Version of the on-disk schema, stored in `PRAGMA user_version`. Only ever increases.
pub const SCHEMA_VERSION: i32 = 10;

/// Longest chain of jump hosts a route may form.
pub const MAX_JUMP_DEPTH: usize = 4;

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
    pub(crate) conn: ReentrantMutex<Connection>,
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
            conn: ReentrantMutex::new(conn),
            cipher: RwLock::new(None),
            state: RwLock::new(LockState::Locked {
                reason: LockReason::NotAttempted,
            }),
            key_cache,
        })
    }

    pub fn seal_audit(
        &self,
        session: &str,
        reference: uuid::Uuid,
        body: &serde_json::Value,
    ) -> Result<Vec<u8>, VaultError> {
        let data = Zeroizing::new(serde_json::to_vec(body)?);
        self.with_cipher(|cipher| {
            cipher.seal(
                &data,
                format!("rove/audit/v1:{session}:{reference}").as_bytes(),
            )
        })
    }

    pub fn open_audit(
        &self,
        session: &str,
        reference: uuid::Uuid,
        sealed: &[u8],
    ) -> Result<serde_json::Value, VaultError> {
        self.with_cipher(|cipher| {
            let data = cipher.open(
                sealed,
                format!("rove/audit/v1:{session}:{reference}").as_bytes(),
            )?;
            Ok(serde_json::from_slice(&data)?)
        })
    }

    /// Identity of this installation, independent of its current sync space.
    pub fn installation_id(&self) -> Result<uuid::Uuid, VaultError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO meta(key,value) VALUES ('installation', ?1)",
            [uuid::Uuid::now_v7().as_bytes().as_slice()],
        )?;
        let bytes: Vec<u8> =
            conn.query_row("SELECT value FROM meta WHERE key='installation'", [], |r| {
                r.get(0)
            })?;
        uuid::Uuid::from_slice(&bytes).map_err(|e| VaultError::Invalid(e.to_string()))
    }

    /// Holds the connection across nested store methods until the complete batch commits.
    pub fn transaction<T>(
        &self,
        action: impl FnOnce() -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        let value = action()?;
        tx.commit()?;
        Ok(value)
    }

    pub fn sync_receipt(&self, space: &str) -> Result<serde_json::Value, VaultError> {
        let data: Option<Vec<u8>> = self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM meta WHERE key=?1",
                [format!("receipts:{space}")],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match data {
            Some(data) => serde_json::from_slice(&data)?,
            None => serde_json::json!({}),
        })
    }

    pub fn set_sync_receipt(
        &self,
        space: &str,
        value: &serde_json::Value,
    ) -> Result<(), VaultError> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES (?1,?2)",
            params![format!("receipts:{space}"), serde_json::to_vec(value)?],
        )?;
        Ok(())
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

    /// Re-stores the device master key under a different key-cache policy and unlocks the vault.
    /// The current policy may prompt for authorization. Existing credentials keep the same key.
    ///
    /// `switch_policy` configures the cache for the next write and leaves the old policy in place
    /// on error. `rollback_policy` restores access under the old policy if storing the key or
    /// recording its check fails. Both callbacks run while the vault holds its database lock.
    pub fn rewrap_master_key<F, R>(
        &self,
        switch_policy: F,
        rollback_policy: R,
    ) -> Result<LockState, VaultError>
    where
        F: FnOnce() -> Result<(), VaultError>,
        R: FnOnce(),
    {
        let conn = self.conn.lock();
        let key_check = read_key_check(&conn)?;
        let loaded = match self.key_cache.load(Interaction::Allowed) {
            Ok(loaded) => loaded,
            Err(error) => {
                if let LockState::Locked { reason } = &mut *self.state.write() {
                    *reason = lock_reason(&error);
                }
                return Err(error.into());
            }
        };
        let key = match loaded {
            Some(key) => key,
            None if key_check.is_some() => return Err(self.mark_key_lost()),
            None => MasterKey::generate()?,
        };
        let cipher = FieldCipher::derive(&key);
        if let Some(check) = &key_check
            && !matches_key_check(&cipher, check)
        {
            return Err(self.mark_key_lost());
        }

        switch_policy()?;
        if let Err(error) = self.key_cache.store(&key) {
            rollback_policy();
            return Err(error.into());
        }
        if key_check.is_none()
            && let Err(error) = write_key_check(&conn, &cipher)
        {
            rollback_policy();
            return Err(error);
        }

        *self.cipher.write() = Some(cipher);
        *self.state.write() = LockState::Unlocked;
        Ok(LockState::Unlocked)
    }

    fn mark_key_lost(&self) -> VaultError {
        let mut cipher = self.cipher.write();
        *cipher = None;
        *self.state.write() = LockState::Locked {
            reason: LockReason::KeyLost,
        };
        VaultError::Locked("device key lost".into())
    }

    fn unlock_with(&self, interaction: Interaction) -> LockState {
        let conn = self.conn.lock();
        let state = match self.load_or_create_key(&conn, interaction) {
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

    fn load_or_create_key(
        &self,
        conn: &Connection,
        interaction: Interaction,
    ) -> Result<FieldCipher, LockReason> {
        let key_check = read_key_check(conn)
            .map_err(|error| LockReason::StoreUnavailable(error.to_string()))?;
        let key = match self
            .key_cache
            .load(interaction)
            .map_err(|error| lock_reason(&error))?
        {
            Some(key) => key,
            None if key_check.is_some() => return Err(LockReason::KeyLost),
            None => {
                let key = MasterKey::generate()
                    .map_err(|error| LockReason::StoreUnavailable(error.to_string()))?;
                self.key_cache
                    .store(&key)
                    .map_err(|error| lock_reason(&error))?;
                key
            }
        };
        let cipher = FieldCipher::derive(&key);
        if let Some(check) = key_check {
            if !matches_key_check(&cipher, &check) {
                return Err(LockReason::KeyLost);
            }
        } else {
            write_key_check(conn, &cipher)
                .map_err(|error| LockReason::StoreUnavailable(error.to_string()))?;
        }
        Ok(cipher)
    }

    /// Deletes every stored secret and unlocks with a fresh master key. The only way out of
    /// [`LockReason::KeyLost`]. Hosts and known hosts are kept.
    pub fn reset_secrets(&self) -> Result<LockState, VaultError> {
        tracing::warn!(
            "resetting the vault: stored passwords, private keys, AI keys and Tailscale state are removed"
        );
        let conn = self.conn.lock();
        let key = MasterKey::generate()?;
        self.key_cache.store(&key)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM host_secrets", [])?;
        tx.execute("UPDATE ssh_keys SET sealed = NULL", [])?;
        tx.execute(
            "DELETE FROM preferences WHERE key LIKE 'ai_key:%' OR key = 'tailscale_state'",
            [],
        )?;
        let mut device = self.device_preferences()?;
        if device.tailscale.configured {
            device.tailscale.configured = false;
            self.set_device_preferences(device)?;
        }
        tx.execute("DELETE FROM meta WHERE key = 'key_check'", [])?;
        tx.commit()?;
        let cipher = FieldCipher::derive(&key);
        write_key_check(&conn, &cipher).map_err(|error| {
            let reason = LockReason::StoreUnavailable(error.to_string());
            VaultError::Locked(format!("{reason:?}"))
        })?;
        *self.cipher.write() = Some(cipher);
        *self.state.write() = LockState::Unlocked;
        Ok(LockState::Unlocked)
    }

    /// Lists hosts ordered by group, then name.
    pub fn list_hosts(&self) -> Result<Vec<Host>, VaultError> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(
            "SELECT id, name, group_name, hostname, port, username, auth, tags, created_at, updated_at, route, last_connected_at, os
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
            "SELECT id, name, group_name, hostname, port, username, auth, tags, created_at, updated_at, route, last_connected_at, os
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
        let id = HostId::new();
        self.check_route(id, &draft.route)?;
        self.check_auth(&draft.auth)?;
        let now = now_millis();
        let host = Host {
            id,
            name: draft.name,
            group: draft.group,
            hostname: draft.hostname,
            port: draft.port,
            username: draft.username,
            auth: draft.auth,
            route: draft.route,
            tags: draft.tags,
            created_at: now,
            updated_at: now,
            last_connected_at: None,
            os: None,
        };
        self.conn.lock().execute(
            "INSERT INTO hosts (id, name, group_name, hostname, port, username, auth, tags, created_at, updated_at, route)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                host.updated_at,
                serde_json::to_string(&host.route)?
            ],
        )?;
        Ok(host)
    }

    /// Validates `draft` and replaces the host's fields. Changing the authentication method to one
    /// that stores a different secret kind deletes the stored secret.
    pub fn update_host(&self, id: HostId, draft: HostDraft) -> Result<Host, VaultError> {
        let draft = draft.validate()?;
        let previous = self.host(id)?;
        self.check_route(id, &draft.route)?;
        self.check_auth(&draft.auth)?;
        let now = now_millis();
        {
            let conn = self.conn.lock();
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE hosts SET name = ?2, group_name = ?3, hostname = ?4, port = ?5, username = ?6,
                 auth = ?7, tags = ?8, updated_at = ?9, route = ?10 WHERE id = ?1",
                params![
                    id.to_string(),
                    draft.name,
                    draft.group,
                    draft.hostname,
                    draft.port,
                    draft.username,
                    serde_json::to_string(&draft.auth)?,
                    serde_json::to_string(&draft.tags)?,
                    now,
                    serde_json::to_string(&draft.route)?
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
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        self.delete_host_rows(&tx, id)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn delete_host_rows(&self, tx: &Connection, id: HostId) -> Result<(), VaultError> {
        let deleted = tx.execute("DELETE FROM hosts WHERE id = ?1", params![id.to_string()])?;
        if deleted == 0 {
            return Err(VaultError::HostNotFound(id));
        }
        // Hosts that jumped through the deleted host connect directly from now on.
        tx.execute(
            "UPDATE hosts SET route = ?2, updated_at = ?3
             WHERE json_extract(route, '$.kind') = 'jump' AND json_extract(route, '$.host') = ?1",
            params![
                id.to_string(),
                serde_json::to_string(&HostRoute::Direct)?,
                now_millis()
            ],
        )?;
        Ok(())
    }

    /// Records that host `id` was just connected to.
    pub fn touch_host(&self, id: HostId) -> Result<(), VaultError> {
        self.conn.lock().execute(
            "UPDATE hosts SET last_connected_at = ?2 WHERE id = ?1",
            params![id.to_string(), now_millis()],
        )?;
        Ok(())
    }

    /// Records the operating system detected on host `id`, or forgets it with `None`.
    pub fn set_host_os(&self, id: HostId, os: Option<HostOs>) -> Result<(), VaultError> {
        let json = match os {
            Some(os) => Some(serde_json::to_string(&os.validate()?)?),
            None => None,
        };
        let updated = self.conn.lock().execute(
            "UPDATE hosts SET os = ?2 WHERE id = ?1",
            params![id.to_string(), json],
        )?;
        if updated == 0 {
            return Err(VaultError::HostNotFound(id));
        }
        Ok(())
    }

    /// Checks that a library key named by `auth` exists.
    fn check_auth(&self, auth: &AuthMethod) -> Result<(), VaultError> {
        if let AuthMethod::Key { key } = auth {
            self.ssh_key(*key)?;
        }
        Ok(())
    }

    /// Checks that a jump route of `id` names another existing host and that following jump
    /// hosts from there neither returns to `id` nor exceeds [`MAX_JUMP_DEPTH`].
    fn check_route(&self, id: HostId, route: &HostRoute) -> Result<(), VaultError> {
        let mut next = route.clone();
        let mut depth = 0;
        while let HostRoute::Jump { host } = next {
            depth += 1;
            if host == id {
                return Err(VaultError::Invalid(
                    "a host cannot jump through itself".into(),
                ));
            }
            if depth > MAX_JUMP_DEPTH {
                return Err(VaultError::Invalid(format!(
                    "at most {MAX_JUMP_DEPTH} jump hosts in a chain"
                )));
            }
            next = self.host(host)?.route;
        }
        Ok(())
    }

    /// Lists forwarding rules of `host`, or of every host, oldest first.
    pub fn list_forwards(&self, host: Option<HostId>) -> Result<Vec<ForwardRule>, VaultError> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(
            "SELECT id, host_id, label, spec, auto_start, created_at, updated_at FROM forwards
             WHERE ?1 IS NULL OR host_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = statement.query_map(params![host.map(|id| id.to_string())], row_to_forward)?;
        rows.map(|row| row.map_err(VaultError::from).and_then(|rule| rule))
            .collect()
    }

    /// Returns one forwarding rule.
    pub fn forward(&self, id: ForwardId) -> Result<ForwardRule, VaultError> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT id, host_id, label, spec, auto_start, created_at, updated_at FROM forwards
             WHERE id = ?1",
            params![id.to_string()],
            row_to_forward,
        )
        .optional()?
        .ok_or(VaultError::ForwardNotFound(id))?
    }

    /// Validates `draft` and stores it as a new rule of `host_id`.
    pub fn create_forward(
        &self,
        host_id: HostId,
        draft: ForwardDraft,
    ) -> Result<ForwardRule, VaultError> {
        let draft = draft.validate()?;
        self.host(host_id)?;
        let now = now_millis();
        let rule = ForwardRule {
            id: ForwardId::new(),
            host_id,
            label: draft.label,
            spec: draft.spec,
            auto_start: draft.auto_start,
            created_at: now,
            updated_at: now,
        };
        self.conn.lock().execute(
            "INSERT INTO forwards (id, host_id, label, spec, auto_start, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rule.id.to_string(),
                host_id.to_string(),
                rule.label,
                serde_json::to_string(&rule.spec)?,
                rule.auto_start,
                rule.created_at,
                rule.updated_at
            ],
        )?;
        Ok(rule)
    }

    /// Validates `draft` and replaces the rule's fields.
    pub fn update_forward(
        &self,
        id: ForwardId,
        draft: ForwardDraft,
    ) -> Result<ForwardRule, VaultError> {
        let draft = draft.validate()?;
        let updated = self.conn.lock().execute(
            "UPDATE forwards SET label = ?2, spec = ?3, auto_start = ?4, updated_at = ?5
             WHERE id = ?1",
            params![
                id.to_string(),
                draft.label,
                serde_json::to_string(&draft.spec)?,
                draft.auto_start,
                now_millis()
            ],
        )?;
        if updated == 0 {
            return Err(VaultError::ForwardNotFound(id));
        }
        self.forward(id)
    }

    /// Deletes a forwarding rule.
    pub fn delete_forward(&self, id: ForwardId) -> Result<(), VaultError> {
        let deleted = self.conn.lock().execute(
            "DELETE FROM forwards WHERE id = ?1",
            params![id.to_string()],
        )?;
        if deleted == 0 {
            return Err(VaultError::ForwardNotFound(id));
        }
        Ok(())
    }

    /// Returns the stored preferences, or the defaults when none are stored. Readable while
    /// locked.
    pub fn preferences(&self) -> Result<Preferences, VaultError> {
        let stored: Option<String> = self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM preferences WHERE key = 'app'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match stored {
            Some(json) => Ok(serde_json::from_str(&json)?),
            None => Ok(Preferences::default()),
        }
    }

    /// Validates and stores `prefs`, replacing the previous record. Returns what was stored.
    pub fn set_preferences(&self, prefs: Preferences) -> Result<Preferences, VaultError> {
        let prefs = prefs.validate()?;
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES ('app', ?1, ?2)",
            params![serde_json::to_string(&prefs)?, now_millis()],
        )?;
        Ok(prefs)
    }

    /// Returns this device's preferences, or the defaults when none are stored. Readable while
    /// locked.
    pub fn device_preferences(&self) -> Result<DevicePrefs, VaultError> {
        let stored: Option<String> = self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM preferences WHERE key = 'device'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match stored {
            Some(json) => Ok(serde_json::from_str::<DevicePrefs>(&json)?.upgrade()),
            None => Ok(DevicePrefs::default()),
        }
    }

    /// Validates and stores this device's preferences. Returns what was stored.
    pub fn set_device_preferences(&self, prefs: DevicePrefs) -> Result<DevicePrefs, VaultError> {
        let prefs = prefs.validate()?;
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES ('device', ?1, ?2)",
            params![serde_json::to_string(&prefs)?, now_millis()],
        )?;
        Ok(prefs)
    }

    pub(crate) fn with_cipher<T>(
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

    /// Compares `key` with the trusted key recorded for `hostname:port` reached along `route`
    /// (empty for direct connections). The same address behind different jump hosts or proxies
    /// is a different server and has its own record.
    pub fn check_host_key(
        &self,
        hostname: &str,
        port: u16,
        route: &str,
        key: &HostKey,
    ) -> Result<KnownHostStatus, VaultError> {
        let recorded: Option<(String, String, String)> = self
            .conn
            .lock()
            .query_row(
                "SELECT algorithm, public_key, fingerprint FROM known_hosts
                 WHERE hostname = ?1 AND port = ?2 AND route = ?3",
                params![hostname.to_ascii_lowercase(), port, route],
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

    /// Records `key` as the trusted key of `hostname:port` along `route`, replacing any previous
    /// key of that triple.
    pub fn trust_host_key(
        &self,
        hostname: &str,
        port: u16,
        route: &str,
        key: &HostKey,
    ) -> Result<(), VaultError> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO known_hosts
             (hostname, port, route, algorithm, public_key, fingerprint, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                hostname.to_ascii_lowercase(),
                port,
                route,
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

fn read_key_check(conn: &Connection) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'key_check'",
        [],
        |row| row.get(0),
    )
    .optional()
}

fn matches_key_check(cipher: &FieldCipher, check: &[u8]) -> bool {
    cipher
        .open(check, KEY_CHECK_AAD)
        .is_ok_and(|plain| plain.as_slice() == KEY_CHECK_PLAINTEXT)
}

fn write_key_check(conn: &Connection, cipher: &FieldCipher) -> Result<(), VaultError> {
    let check = cipher.seal(KEY_CHECK_PLAINTEXT, KEY_CHECK_AAD)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('key_check', ?1)",
        params![check],
    )?;
    Ok(())
}

fn lock_reason(error: &KeyCacheError) -> LockReason {
    match error {
        KeyCacheError::Denied => LockReason::Denied,
        KeyCacheError::Malformed => LockReason::KeyLost,
        KeyCacheError::Unavailable(message) => LockReason::StoreUnavailable(message.clone()),
    }
}

type HostRow = Result<Host, VaultError>;
type ForwardRow = Result<ForwardRule, VaultError>;

fn row_to_forward(row: &rusqlite::Row<'_>) -> rusqlite::Result<ForwardRow> {
    let id: String = row.get(0)?;
    let host_id: String = row.get(1)?;
    let spec: String = row.get(3)?;
    Ok((|| {
        Ok(ForwardRule {
            id: id.parse()?,
            host_id: host_id.parse()?,
            label: row.get(2)?,
            spec: serde_json::from_str(&spec)?,
            auto_start: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })())
}

fn row_to_host(row: &rusqlite::Row<'_>) -> rusqlite::Result<HostRow> {
    let id: String = row.get(0)?;
    let auth: String = row.get(6)?;
    let tags: String = row.get(7)?;
    let route: String = row.get(10)?;
    Ok((|| {
        Ok(Host {
            id: id.parse()?,
            name: row.get(1)?,
            group: row.get(2)?,
            hostname: row.get(3)?,
            port: row.get(4)?,
            username: row.get(5)?,
            auth: serde_json::from_str(&auth)?,
            route: serde_json::from_str(&route)?,
            tags: serde_json::from_str(&tags)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
            last_connected_at: row.get(11)?,
            // Display-only and written by this crate; a record that no longer parses is dropped
            // rather than hiding the host.
            os: row
                .get::<_, Option<String>>(12)?
                .and_then(|json| serde_json::from_str(&json).ok()),
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
    if version < 2 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE forwards (
                 id TEXT PRIMARY KEY,
                 host_id TEXT NOT NULL REFERENCES hosts(id) ON DELETE CASCADE,
                 label TEXT NOT NULL,
                 spec TEXT NOT NULL,
                 auto_start INTEGER NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE INDEX forwards_by_host ON forwards (host_id);
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
    }
    if version < 3 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE preferences (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
    }
    if version < 4 {
        conn.execute_batch(
            r#"BEGIN;
             ALTER TABLE hosts ADD COLUMN route TEXT NOT NULL DEFAULT '{"kind":"direct"}';
             PRAGMA user_version = 4;
             COMMIT;"#,
        )?;
    }
    if version < 5 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE ssh_keys (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 algorithm TEXT NOT NULL,
                 public_key TEXT NOT NULL,
                 fingerprint TEXT NOT NULL,
                 sealed BLOB,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             ALTER TABLE hosts ADD COLUMN last_connected_at INTEGER;
             PRAGMA user_version = 5;
             COMMIT;",
        )?;
    }
    if version < 6 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE snippets (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 command TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             PRAGMA user_version = 6;
             COMMIT;",
        )?;
    }
    if version < 7 {
        // SQLite cannot change a primary key in place: rebuild the table with `route` in it.
        // Existing records were all direct connections.
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE known_hosts_v7 (
                 hostname TEXT NOT NULL,
                 port INTEGER NOT NULL,
                 route TEXT NOT NULL DEFAULT '',
                 algorithm TEXT NOT NULL,
                 public_key TEXT NOT NULL,
                 fingerprint TEXT NOT NULL,
                 added_at INTEGER NOT NULL,
                 PRIMARY KEY (hostname, port, route)
             );
             INSERT INTO known_hosts_v7 (hostname, port, route, algorithm, public_key, fingerprint, added_at)
                 SELECT hostname, port, '', algorithm, public_key, fingerprint, added_at FROM known_hosts;
             DROP TABLE known_hosts;
             ALTER TABLE known_hosts_v7 RENAME TO known_hosts;
             PRAGMA user_version = 7;
             COMMIT;",
        )?;
    }
    if version < 8 {
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE hosts ADD COLUMN os TEXT;
             PRAGMA user_version = 8;
             COMMIT;",
        )?;
    }
    if version < 9 {
        // API keys became per agent; the single key served Codex.
        conn.execute_batch(
            "BEGIN;
             UPDATE preferences SET key = 'ai_key:acp:codex' WHERE key = 'ai_key';
             PRAGMA user_version = 9;
             COMMIT;",
        )?;
    }
    if version < 10 {
        // Notes about each host, kept by the user and the AI agents.
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE host_notes (
                 host_id TEXT PRIMARY KEY REFERENCES hosts(id) ON DELETE CASCADE,
                 text TEXT NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             PRAGMA user_version = 10;
             COMMIT;",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use parking_lot::Mutex;

    use super::*;

    #[derive(Default)]
    struct BiometricKeyCache {
        biometric: AtomicBool,
        deny_load: AtomicBool,
        fail_biometric_store: AtomicBool,
        stored: Mutex<Option<(MasterKey, bool)>>,
    }

    impl BiometricKeyCache {
        fn enable_biometrics(&self) -> Result<(), VaultError> {
            self.biometric.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn disable_biometrics(&self) {
            self.biometric.store(false, Ordering::Relaxed);
        }
    }

    impl KeyCache for BiometricKeyCache {
        fn load(&self, interaction: Interaction) -> Result<Option<MasterKey>, KeyCacheError> {
            let biometric = self.biometric.load(Ordering::Relaxed);
            if self.deny_load.load(Ordering::Relaxed)
                || (biometric && interaction == Interaction::Silent)
            {
                return Err(KeyCacheError::Denied);
            }
            Ok(self
                .stored
                .lock()
                .as_ref()
                .filter(|(_, protected)| *protected == biometric)
                .map(|(key, _)| key.clone()))
        }

        fn store(&self, key: &MasterKey) -> Result<(), KeyCacheError> {
            let biometric = self.biometric.load(Ordering::Relaxed);
            if biometric && self.fail_biometric_store.load(Ordering::Relaxed) {
                return Err(KeyCacheError::Unavailable("test store failure".into()));
            }
            *self.stored.lock() = Some((key.clone(), biometric));
            Ok(())
        }
    }

    fn save_host_secret(vault: &Vault) -> (HostId, Secret) {
        let host = vault
            .create_host(HostDraft {
                name: String::new(),
                group: None,
                hostname: "host.example".into(),
                port: 22,
                username: "test".into(),
                auth: AuthMethod::Password,
                route: HostRoute::Direct,
                tags: Vec::new(),
            })
            .unwrap();
        let secret = Secret::Password {
            password: Zeroizing::new(uuid::Uuid::now_v7().to_string()),
        };
        vault.set_secret(host.id, &secret).unwrap();
        (host.id, secret)
    }

    #[test]
    fn biometric_cache_requires_interaction_to_unlock_saved_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let cache = Arc::new(BiometricKeyCache::default());
        cache.enable_biometrics().unwrap();
        let (id, secret) = {
            let vault = Vault::open(&path, cache.clone()).unwrap();
            assert_eq!(vault.unlock(), LockState::Unlocked);
            save_host_secret(&vault)
        };

        let vault = Vault::open(&path, cache).unwrap();
        let denied = LockState::Locked {
            reason: LockReason::Denied,
        };
        assert_eq!(vault.try_unlock(), denied);
        assert_eq!(vault.lock_state(), denied);
        assert!(matches!(vault.secret(id), Err(VaultError::Locked(_))));
        assert_eq!(vault.unlock(), LockState::Unlocked);
        assert_eq!(vault.secret(id).unwrap(), Some(secret));
    }

    #[test]
    fn rewrap_keeps_saved_secrets_under_biometric_policy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let cache = Arc::new(BiometricKeyCache::default());
        let vault = Vault::open(&path, cache.clone()).unwrap();
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        let (id, secret) = save_host_secret(&vault);
        let check = read_key_check(&vault.conn.lock()).unwrap().unwrap();

        let state = vault
            .rewrap_master_key(
                || cache.enable_biometrics(),
                || panic!("successful rewrap must not roll back"),
            )
            .unwrap();
        assert_eq!(state, LockState::Unlocked);
        assert_eq!(vault.secret(id).unwrap().as_ref(), Some(&secret));
        assert_eq!(read_key_check(&vault.conn.lock()).unwrap(), Some(check));

        drop(vault);
        let vault = Vault::open(&path, cache).unwrap();
        assert_eq!(
            vault.try_unlock(),
            LockState::Locked {
                reason: LockReason::Denied,
            }
        );
        assert_eq!(vault.unlock(), LockState::Unlocked);
        assert_eq!(vault.secret(id).unwrap(), Some(secret));
    }

    #[test]
    fn rewrap_rolls_back_after_key_store_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let cache = Arc::new(BiometricKeyCache::default());
        let vault = Vault::open(&path, cache.clone()).unwrap();
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        let (id, secret) = save_host_secret(&vault);
        let check = read_key_check(&vault.conn.lock()).unwrap().unwrap();
        cache.fail_biometric_store.store(true, Ordering::Relaxed);

        let result =
            vault.rewrap_master_key(|| cache.enable_biometrics(), || cache.disable_biometrics());
        assert!(matches!(
            result,
            Err(VaultError::KeyCache(KeyCacheError::Unavailable(_)))
        ));
        assert!(!cache.biometric.load(Ordering::Relaxed));
        assert_eq!(vault.lock_state(), LockState::Unlocked);
        assert_eq!(vault.secret(id).unwrap().as_ref(), Some(&secret));
        assert_eq!(read_key_check(&vault.conn.lock()).unwrap(), Some(check));

        drop(vault);
        let vault = Vault::open(&path, cache).unwrap();
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        assert_eq!(vault.secret(id).unwrap(), Some(secret));
    }

    #[test]
    fn rewrap_adopts_a_generated_or_cached_key_in_a_new_vault() {
        for initial_key in [None, Some(MasterKey::generate().unwrap())] {
            let cache = Arc::new(BiometricKeyCache::default());
            if let Some(key) = &initial_key {
                cache.store(key).unwrap();
            }
            let vault = Vault::open_in_memory(cache.clone()).unwrap();
            assert!(read_key_check(&vault.conn.lock()).unwrap().is_none());

            let state = vault
                .rewrap_master_key(
                    || cache.enable_biometrics(),
                    || panic!("successful rewrap must not roll back"),
                )
                .unwrap();
            assert_eq!(state, LockState::Unlocked);
            assert_eq!(vault.lock_state(), LockState::Unlocked);
            let key = cache.load(Interaction::Allowed).unwrap().unwrap();
            let check = read_key_check(&vault.conn.lock()).unwrap().unwrap();
            assert!(matches_key_check(&FieldCipher::derive(&key), &check));
            if let Some(initial_key) = &initial_key {
                assert!(matches_key_check(&FieldCipher::derive(initial_key), &check));
            }
            assert!(matches!(
                cache.load(Interaction::Silent),
                Err(KeyCacheError::Denied)
            ));
        }
    }

    #[test]
    fn rewrap_rolls_back_when_writing_key_check_fails() {
        let cache = Arc::new(BiometricKeyCache::default());
        let vault = Vault::open_in_memory(cache.clone()).unwrap();
        vault
            .conn
            .lock()
            .execute_batch(
                "CREATE TRIGGER reject_key_check BEFORE INSERT ON meta
                 WHEN NEW.key = 'key_check' BEGIN
                    SELECT RAISE(ABORT, 'test key check write failure');
                 END;",
            )
            .unwrap();

        let result =
            vault.rewrap_master_key(|| cache.enable_biometrics(), || cache.disable_biometrics());
        assert!(matches!(result, Err(VaultError::Database(_))));
        assert!(!cache.biometric.load(Ordering::Relaxed));
        assert!(read_key_check(&vault.conn.lock()).unwrap().is_none());
        assert_eq!(
            vault.lock_state(),
            LockState::Locked {
                reason: LockReason::NotAttempted,
            }
        );
        assert!(vault.cipher.read().is_none());
    }

    #[test]
    fn rewrap_load_error_updates_only_the_locked_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let cache = Arc::new(BiometricKeyCache::default());
        let unlocked = Vault::open(&path, cache.clone()).unwrap();
        assert_eq!(unlocked.try_unlock(), LockState::Unlocked);
        let (id, secret) = save_host_secret(&unlocked);
        let locked = Vault::open(&path, cache.clone()).unwrap();
        cache.deny_load.store(true, Ordering::Relaxed);

        for vault in [&locked, &unlocked] {
            let result = vault.rewrap_master_key(
                || panic!("load failure must not switch policy"),
                || panic!("load failure must not roll back policy"),
            );
            assert!(matches!(
                result,
                Err(VaultError::KeyCache(KeyCacheError::Denied))
            ));
        }
        assert_eq!(
            locked.lock_state(),
            LockState::Locked {
                reason: LockReason::Denied,
            }
        );
        assert_eq!(unlocked.lock_state(), LockState::Unlocked);
        assert_eq!(unlocked.secret(id).unwrap(), Some(secret));
    }

    #[test]
    fn rewrap_rejects_a_missing_or_mismatched_key() {
        for replacement in [None, Some(MasterKey::generate().unwrap())] {
            let cache = Arc::new(BiometricKeyCache::default());
            let vault = Vault::open_in_memory(cache.clone()).unwrap();
            assert_eq!(vault.try_unlock(), LockState::Unlocked);
            let (id, _) = save_host_secret(&vault);
            let check = read_key_check(&vault.conn.lock()).unwrap().unwrap();
            *cache.stored.lock() = replacement.map(|key| (key, false));

            let result = vault.rewrap_master_key(
                || panic!("a lost key must not switch policy"),
                || panic!("a lost key must not roll back policy"),
            );
            assert!(
                matches!(result, Err(VaultError::Locked(reason)) if reason == "device key lost")
            );
            assert_eq!(
                vault.lock_state(),
                LockState::Locked {
                    reason: LockReason::KeyLost,
                }
            );
            assert_eq!(read_key_check(&vault.conn.lock()).unwrap(), Some(check));
            assert!(vault.has_secret(id).unwrap());
            assert!(matches!(vault.secret(id), Err(VaultError::Locked(_))));
        }
    }
}
