//! The vault's side of multi-device sync: every syncable record as JSON, applying changes that
//! arrive from other devices, and this device's sync settings. The vault knows nothing of the
//! sync protocol; it only reads and writes records. See docs/decisions/0005-sync.md.
//!
//! Synced: hosts (without connection times), their secrets, library keys with their private
//! halves, snippets, forward rules, the user preferences and the AI agents' API keys. Never synced: known hosts, device
//! preferences, the device key, and the sync settings themselves.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::model::now_millis;
use crate::{
    AuthMethod, ForwardId, ForwardSpec, HostDraft, HostId, HostOs, HostRoute, KeyId, KeySecret,
    Preferences, ProviderPrefs, Secret, SnippetDraft, SnippetId, Vault, VaultError,
};

/// Record kinds, in the order changes are applied (parents first).
pub const SYNC_KINDS: [&str; 8] = [
    "pref",
    "ai_provider",
    "host",
    "key",
    "host_secret",
    "forward",
    "snippet",
    "ai_key",
];

/// Sections of the preferences, each synced as its own `pref` record so that a change to one
/// never overwrites another. The model providers leave the `ai` section and sync one by one as
/// `ai_provider` records. (Older versions synced the whole as one `prefs` record, which new
/// versions ignore: last writer wins on a whole record lost one device's providers to another
/// device's other settings.)
pub const PREF_SECTIONS: [&str; 5] = ["appearance", "terminal", "files", "status", "ai"];
/// Kinds that carry secrets, which [`Vault::reset_secrets`] discards on this device: after a
/// reset the sync space has to restore them rather than learn that they are gone.
pub const SYNC_SECRET_KINDS: [&str; 3] = ["host_secret", "key", "ai_key"];

/// One syncable record.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncRecord {
    pub kind: &'static str,
    pub id: String,
    pub body: Value,
}

/// A change to apply: `body: None` deletes the record.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncChange {
    pub kind: String,
    pub id: String,
    pub body: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HostBody {
    name: String,
    group: Option<String>,
    hostname: String,
    port: u16,
    username: String,
    auth: AuthMethod,
    route: HostRoute,
    tags: Vec<String>,
    os: Option<HostOs>,
    created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct KeyBody {
    name: String,
    algorithm: String,
    public_key: String,
    fingerprint: String,
    created_at: i64,
    /// Absent when this device lost the private half (after a reset); the receiving device then
    /// keeps its own.
    secret: Option<KeySecret>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SnippetBody {
    name: String,
    command: String,
    created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ForwardBody {
    host_id: HostId,
    label: String,
    spec: ForwardSpec,
    auto_start: bool,
    created_at: i64,
}

/// This device's sync settings: where the space is and the sync state, plus the sync key sealed
/// with the device key. Stored apart from everything synced.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncLocal {
    /// The backend and how to reach it, as the application describes it.
    pub target: Value,
    /// The engine's bookkeeping between rounds.
    pub state: Value,
    /// The sync key; `None` until the user enters the sync password on this device.
    pub key: Option<Zeroizing<[u8; 32]>>,
    /// The storage's own password (WebDAV), if it needs one.
    pub secret: Option<Zeroizing<String>>,
}

#[derive(Serialize, Deserialize)]
struct StoredSyncLocal {
    target: Value,
    state: Value,
    /// Hex of the sealed key.
    key: Option<String>,
    /// Hex of the sealed storage password.
    #[serde(default)]
    secret: Option<String>,
}

const SYNC_KEY_AAD: &[u8] = b"rove/vault/sync-key/v1";
const SYNC_SECRET_AAD: &[u8] = b"rove/vault/sync-secret/v1";

fn json<T: Serialize>(value: &T) -> Result<Value, VaultError> {
    Ok(serde_json::to_value(value)?)
}

fn body<T: for<'de> Deserialize<'de>>(value: &Value) -> Result<T, VaultError> {
    serde_json::from_value(value.clone())
        .map_err(|error| VaultError::Invalid(format!("sync record: {error}")))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

impl Vault {
    /// Every syncable record. Requires the vault to be unlocked: without it secrets would look
    /// deleted and the deletion would spread to other devices.
    pub fn sync_records(&self) -> Result<Vec<SyncRecord>, VaultError> {
        self.with_cipher(|_| Ok(()))?;
        let prefs = self.preferences()?;
        let mut whole = json(&prefs)?;
        if let Some(ai) = whole.get_mut("ai").and_then(Value::as_object_mut) {
            ai.remove("providers");
        }
        let mut records: Vec<SyncRecord> = PREF_SECTIONS
            .iter()
            .map(|section| SyncRecord {
                kind: "pref",
                id: (*section).to_owned(),
                body: whole.get(*section).cloned().unwrap_or(Value::Null),
            })
            .collect();
        for provider in &prefs.ai.providers {
            records.push(SyncRecord {
                kind: "ai_provider",
                id: provider.id.clone(),
                body: json(provider)?,
            });
        }
        for host in self.list_hosts()? {
            let id = host.id.to_string();
            if let Some(secret) = self.secret(host.id)? {
                records.push(SyncRecord {
                    kind: "host_secret",
                    id: id.clone(),
                    body: json(&secret)?,
                });
            }
            let body = HostBody {
                name: host.name,
                group: host.group,
                hostname: host.hostname,
                port: host.port,
                username: host.username,
                auth: host.auth,
                route: host.route,
                tags: host.tags,
                os: host.os,
                created_at: host.created_at,
            };
            records.push(SyncRecord {
                kind: "host",
                id,
                body: json(&body)?,
            });
        }
        for key in self.list_ssh_keys()? {
            let secret = if key.has_private {
                Some(self.ssh_key_secret(key.id)?)
            } else {
                None
            };
            let body = KeyBody {
                name: key.name,
                algorithm: key.algorithm,
                public_key: key.public_key,
                fingerprint: key.fingerprint,
                created_at: key.created_at,
                secret,
            };
            records.push(SyncRecord {
                kind: "key",
                id: key.id.to_string(),
                body: json(&body)?,
            });
        }
        for snippet in self.list_snippets()? {
            let body = SnippetBody {
                name: snippet.name,
                command: snippet.command,
                created_at: snippet.created_at,
            };
            records.push(SyncRecord {
                kind: "snippet",
                id: snippet.id.to_string(),
                body: json(&body)?,
            });
        }
        for rule in self.list_forwards(None)? {
            let body = ForwardBody {
                host_id: rule.host_id,
                label: rule.label,
                spec: rule.spec,
                auto_start: rule.auto_start,
                created_at: rule.created_at,
            };
            records.push(SyncRecord {
                kind: "forward",
                id: rule.id.to_string(),
                body: json(&body)?,
            });
        }
        for agent in self.ai_key_ids()? {
            if let Some(key) = self.ai_key(&agent)? {
                records.push(SyncRecord {
                    kind: "ai_key",
                    id: agent,
                    body: serde_json::json!({ "key": key.as_str() }),
                });
            }
        }
        Ok(records)
    }

    /// Applies changes from other devices: upserts parents before children and deletes children
    /// before parents. Bodies are validated like local edits. Requires the vault to be unlocked.
    /// A child whose parent is missing (a forward of a host deleted meanwhile) is skipped.
    pub fn apply_sync(&self, changes: &[SyncChange]) -> Result<(), VaultError> {
        self.with_cipher(|_| Ok(()))?;
        for kind in SYNC_KINDS {
            for change in changes.iter().filter(|change| change.kind == kind) {
                if let Some(body) = &change.body {
                    self.upsert_synced(kind, &change.id, body)?;
                }
            }
        }
        for kind in SYNC_KINDS.iter().rev() {
            for change in changes
                .iter()
                .filter(|change| change.kind == *kind && change.body.is_none())
            {
                self.delete_synced(kind, &change.id)?;
            }
        }
        Ok(())
    }

    fn upsert_synced(&self, kind: &str, id: &str, value: &Value) -> Result<(), VaultError> {
        let now = now_millis();
        match kind {
            // The whole preferences as older versions synced them: see `PREF_SECTIONS`.
            "prefs" => {}
            "pref" => {
                if !PREF_SECTIONS.contains(&id) {
                    return Ok(());
                }
                let mut whole = json(&self.preferences()?)?;
                let mut section = value.clone();
                if id == "ai"
                    && let Some(fields) = section.as_object_mut()
                {
                    // Providers sync on their own; this device keeps the ones it has.
                    fields.insert("providers".into(), whole["ai"]["providers"].clone());
                }
                whole[id] = section;
                self.set_preferences(body::<Preferences>(&whole)?)?;
            }
            "ai_provider" => {
                let provider: ProviderPrefs = body(value)?;
                if provider.id != id {
                    return Err(VaultError::Invalid("provider record under another id".into()));
                }
                let mut prefs = self.preferences()?;
                match prefs.ai.providers.iter_mut().find(|entry| entry.id == id) {
                    Some(entry) => *entry = provider,
                    None => prefs.ai.providers.push(provider),
                }
                self.set_preferences(prefs)?;
            }
            "host" => {
                let host_id: HostId = id.parse()?;
                let host: HostBody = body(value)?;
                let draft = HostDraft {
                    name: host.name,
                    group: host.group,
                    hostname: host.hostname,
                    port: host.port,
                    username: host.username,
                    auth: host.auth,
                    route: host.route,
                    tags: host.tags,
                }
                .validate()?;
                let os = host.os.map(HostOs::validate).transpose()?;
                self.conn.lock().execute(
                    "INSERT INTO hosts (id, name, group_name, hostname, port, username, auth, tags, route, os, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(id) DO UPDATE SET name = excluded.name, group_name = excluded.group_name,
                         hostname = excluded.hostname, port = excluded.port, username = excluded.username,
                         auth = excluded.auth, tags = excluded.tags, route = excluded.route, os = excluded.os,
                         created_at = excluded.created_at, updated_at = excluded.updated_at",
                    params![
                        host_id.to_string(),
                        draft.name,
                        draft.group,
                        draft.hostname,
                        draft.port,
                        draft.username,
                        serde_json::to_string(&draft.auth)?,
                        serde_json::to_string(&draft.tags)?,
                        serde_json::to_string(&draft.route)?,
                        os.as_ref().map(serde_json::to_string).transpose()?,
                        host.created_at,
                        now
                    ],
                )?;
            }
            "ai_key" => {
                let key = value
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| VaultError::Invalid("AI key record without a key".into()))?;
                self.set_ai_key(id, Some(key))?;
            }
            "host_secret" => {
                let host_id: HostId = id.parse()?;
                if self.host(host_id).is_ok() {
                    self.set_secret(host_id, &body::<Secret>(value)?)?;
                }
            }
            "key" => {
                let key_id: KeyId = id.parse()?;
                let key: KeyBody = body(value)?;
                let sealed = key
                    .secret
                    .as_ref()
                    .map(|secret| self.seal_key(key_id, secret))
                    .transpose()?;
                self.conn.lock().execute(
                    "INSERT INTO ssh_keys (id, name, algorithm, public_key, fingerprint, sealed, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET name = excluded.name, algorithm = excluded.algorithm,
                         public_key = excluded.public_key, fingerprint = excluded.fingerprint,
                         sealed = COALESCE(excluded.sealed, ssh_keys.sealed),
                         created_at = excluded.created_at, updated_at = excluded.updated_at",
                    params![
                        key_id.to_string(),
                        key.name.trim(),
                        key.algorithm,
                        key.public_key,
                        key.fingerprint,
                        sealed,
                        key.created_at,
                        now
                    ],
                )?;
            }
            "forward" => {
                let forward_id: ForwardId = id.parse()?;
                let rule: ForwardBody = body(value)?;
                if self.host(rule.host_id).is_err() {
                    return Ok(());
                }
                let spec = rule.spec.validate()?;
                self.conn.lock().execute(
                    "INSERT INTO forwards (id, host_id, label, spec, auto_start, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET host_id = excluded.host_id, label = excluded.label,
                         spec = excluded.spec, auto_start = excluded.auto_start,
                         created_at = excluded.created_at, updated_at = excluded.updated_at",
                    params![
                        forward_id.to_string(),
                        rule.host_id.to_string(),
                        rule.label.chars().take(100).collect::<String>(),
                        serde_json::to_string(&spec)?,
                        rule.auto_start,
                        rule.created_at,
                        now
                    ],
                )?;
            }
            "snippet" => {
                let snippet_id: SnippetId = id.parse()?;
                let snippet: SnippetBody = body(value)?;
                let draft = SnippetDraft {
                    name: snippet.name,
                    command: snippet.command,
                }
                .validate()?;
                self.conn.lock().execute(
                    "INSERT INTO snippets (id, name, command, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(id) DO UPDATE SET name = excluded.name, command = excluded.command,
                         created_at = excluded.created_at, updated_at = excluded.updated_at",
                    params![snippet_id.to_string(), draft.name, draft.command, snippet.created_at, now],
                )?;
            }
            // Kinds from a newer version of the app are ignored.
            _ => {}
        }
        Ok(())
    }

    fn delete_synced(&self, kind: &str, id: &str) -> Result<(), VaultError> {
        match kind {
            "ai_provider" => {
                let mut prefs = self.preferences()?;
                let before = prefs.ai.providers.len();
                prefs.ai.providers.retain(|entry| entry.id != id);
                if prefs.ai.providers.len() != before {
                    self.set_preferences(prefs)?;
                }
            }
            "host" => match self.delete_host(id.parse()?) {
                Ok(()) | Err(VaultError::HostNotFound(_)) => {}
                Err(error) => return Err(error),
            },
            "host_secret" => self.delete_secret(id.parse()?)?,
            "ai_key" => {
                // Keys do not leave by themselves: which device removed one is worth knowing.
                tracing::info!(agent = %id, "AI key removed by another device through sync");
                self.set_ai_key(id, None)?;
            }
            "key" => {
                let key_id: KeyId = id.parse()?;
                self.conn.lock().execute(
                    "DELETE FROM ssh_keys WHERE id = ?1",
                    params![key_id.to_string()],
                )?;
            }
            "forward" => {
                let forward_id: ForwardId = id.parse()?;
                self.conn.lock().execute(
                    "DELETE FROM forwards WHERE id = ?1",
                    params![forward_id.to_string()],
                )?;
            }
            "snippet" => {
                let snippet_id: SnippetId = id.parse()?;
                self.conn.lock().execute(
                    "DELETE FROM snippets WHERE id = ?1",
                    params![snippet_id.to_string()],
                )?;
            }
            // The preferences record always exists; other kinds are from a newer version.
            _ => {}
        }
        Ok(())
    }

    /// This device's sync settings, or `None` when sync is not set up. The sealed key and storage
    /// password are opened only when the vault is unlocked; while locked they read as `None`.
    pub fn sync_local(&self) -> Result<Option<SyncLocal>, VaultError> {
        let stored: Option<String> = self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM preferences WHERE key = 'sync'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        let stored: StoredSyncLocal = serde_json::from_str(&stored)?;
        let open = |hex: Option<&str>, aad: &[u8]| {
            let sealed = hex.and_then(unhex)?;
            self.with_cipher(|cipher| cipher.open(&sealed, aad)).ok()
        };
        let key = open(stored.key.as_deref(), SYNC_KEY_AAD)
            .and_then(|plain| <[u8; 32]>::try_from(plain.as_slice()).ok())
            .map(Zeroizing::new);
        let secret = open(stored.secret.as_deref(), SYNC_SECRET_AAD)
            .and_then(|plain| String::from_utf8(plain.to_vec()).ok())
            .map(Zeroizing::new);
        Ok(Some(SyncLocal {
            target: stored.target,
            state: stored.state,
            key,
            secret,
        }))
    }

    /// Stores this device's sync settings (sealing the key with the device key), or removes them
    /// with `None`. Storing a key requires the vault to be unlocked.
    pub fn set_sync_local(&self, local: Option<&SyncLocal>) -> Result<(), VaultError> {
        let Some(local) = local else {
            self.conn
                .lock()
                .execute("DELETE FROM preferences WHERE key = 'sync'", [])?;
            return Ok(());
        };
        let key = match &local.key {
            Some(key) => {
                Some(hex(&self.with_cipher(|cipher| {
                    cipher.seal(key.as_slice(), SYNC_KEY_AAD)
                })?))
            }
            None => None,
        };
        let secret = match &local.secret {
            Some(secret) => {
                Some(hex(&self.with_cipher(|cipher| {
                    cipher.seal(secret.as_bytes(), SYNC_SECRET_AAD)
                })?))
            }
            None => None,
        };
        let stored = StoredSyncLocal {
            target: local.target.clone(),
            state: local.state.clone(),
            key,
            secret,
        };
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO preferences (key, value, updated_at) VALUES ('sync', ?1, ?2)",
            params![serde_json::to_string(&stored)?, now_millis()],
        )?;
        Ok(())
    }
}
