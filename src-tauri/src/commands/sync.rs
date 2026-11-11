//! Multi-device sync: set up a space, run a round, report status, turn sync off. The space is
//! described by a [`SyncTarget`]; the vault stores it with the sync state and the sealed key.

use std::path::Path;
use std::sync::Arc;

use rove_sync::{
    Change, FolderBackend, KdfParams, Record, RecordStore, Report, SyncBackend, SyncError, SyncKey,
    SyncState, WebDavBackend,
};
use rove_vault::{SyncChange, SyncLocal, Vault};
use serde::{Deserialize, Serialize};
use tauri::State;
use zeroize::Zeroizing;

use crate::error::CommandError;
use crate::state::AppState;

/// Shortest sync password accepted when creating a space.
const MIN_PASSWORD: usize = 8;
/// Longest sync password accepted.
const MAX_PASSWORD: usize = 1024;

/// Where the sync space lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncTarget {
    /// A directory on this device, typically inside a synced folder.
    Folder { path: String },
    /// A directory on a WebDAV server; its password is stored sealed, apart from the target.
    #[serde(rename = "webdav")]
    WebDav { url: String, username: String },
}

impl SyncTarget {
    /// Checks the target from the WebView and opens its backend; `secret` is the storage's
    /// password where it needs one.
    fn backend(&self, secret: Option<&str>) -> Result<Box<dyn SyncBackend>, CommandError> {
        match self {
            Self::WebDav { url, username } => {
                if url.len() > 2048 || username.len() > 256 {
                    return Err(CommandError::new(
                        "input.invalid",
                        "WebDAV address or user name too long",
                    ));
                }
                let password = secret.ok_or_else(|| {
                    CommandError::new(
                        "sync.needs_password",
                        "enter the WebDAV password on this device",
                    )
                })?;
                Ok(Box::new(
                    WebDavBackend::new(url, username, password).map_err(sync_error)?,
                ))
            }
            Self::Folder { path } => {
                let dir = Path::new(path);
                if path.len() > 4096 || !dir.is_absolute() {
                    return Err(CommandError::new(
                        "input.invalid",
                        "the sync folder must be an absolute path",
                    ));
                }
                Ok(Box::new(FolderBackend::new(dir)))
            }
        }
    }
}

/// Sync as the settings page shows it.
#[derive(Debug, Serialize)]
pub struct SyncStatus {
    /// Storage is chosen on this device.
    pub configured: bool,
    /// The storage can be reached: a folder, or WebDAV with its password on this device.
    pub connected: bool,
    pub target: Option<SyncTarget>,
    /// The sync key is available (the vault is unlocked and holds it).
    pub ready: bool,
    /// This device's id in the space.
    pub device: Option<String>,
    /// Records this device has synced.
    pub records: usize,
}

fn sync_error(error: SyncError) -> CommandError {
    let code = match &error {
        SyncError::Backend(_) => "sync.backend",
        SyncError::Format(_) => "sync.format",
        SyncError::Rejected => "sync.rejected",
        SyncError::WrongPassword => "sync.wrong_password",
        SyncError::NoSpace => "sync.no_space",
        SyncError::SpaceExists => "sync.space_exists",
        SyncError::SpaceChanged => "sync.space_changed",
        SyncError::Crypto(_) => "sync.crypto",
        SyncError::Unauthorized => "sync.unauthorized",
        SyncError::Store(_) => "sync.store",
    };
    CommandError::new(code, error.to_string())
}

/// Record kind of a synced AI conversation (its journal records).
const CONVERSATION_KIND: &str = "ai_conversation";
/// Most conversations synced: the most recently started.
const MAX_CONVERSATIONS: usize = 100;
/// Conversations started longer ago than this are not synced.
const CONVERSATION_AGE_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// What a device syncs: the vault's records, and its recent AI conversations from the journal.
struct SyncStore {
    vault: VaultStore,
    conversations: ConversationStore,
}

impl RecordStore for SyncStore {
    fn records(&self) -> Result<Vec<Record>, SyncError> {
        let mut records = self.vault.records()?;
        records.extend(self.conversations.records()?);
        Ok(records)
    }

    fn apply(&self, changes: &[Change]) -> Result<(), SyncError> {
        let (conversations, others): (Vec<Change>, Vec<Change>) = changes
            .iter()
            .cloned()
            .partition(|change| change.kind == CONVERSATION_KIND);
        self.vault.apply(&others)?;
        self.conversations.apply(&conversations)
    }
}

/// Recent AI conversations as sync records; arriving ones are merged into the journal (records
/// it lacks are added with their own time). The journal only appends, so deletions are ignored.
struct ConversationStore(Arc<dyn rove_journal::JournalStore>);

fn journal_error(error: rove_journal::JournalError) -> SyncError {
    SyncError::Store(error.to_string())
}

impl ConversationStore {
    fn session(&self, id: &str) -> Result<Vec<rove_journal::JournalRecord>, SyncError> {
        self.0
            .read(&rove_journal::JournalQuery {
                session_id: Some(id.to_owned()),
                limit: Some(5000),
                ..rove_journal::JournalQuery::default()
            })
            .map_err(journal_error)
    }

    fn records(&self) -> Result<Vec<Record>, SyncError> {
        let starts = self
            .0
            .read(&rove_journal::JournalQuery {
                kind: Some(rove_ai::audit::KIND_STARTED.to_owned()),
                limit: Some(1000),
                newest_first: true,
                ..rove_journal::JournalQuery::default()
            })
            .map_err(journal_error)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        let cutoff = now - CONVERSATION_AGE_MS;
        let mut ids: Vec<String> = Vec::new();
        for start in starts {
            let Some(id) = start.session_id.filter(|_| start.ts >= cutoff) else {
                continue;
            };
            if !ids.contains(&id) {
                ids.push(id);
            }
            if ids.len() >= MAX_CONVERSATIONS {
                break;
            }
        }
        let deleted =
            crate::commands::ai::deleted_conversations(self.0.as_ref()).map_err(journal_error)?;
        let mut records = Vec::new();
        // A deleted conversation is passed on as deleted (a removed record is not: conversations
        // also leave the synced set by age, and that must not delete them elsewhere).
        let tombstones = self
            .0
            .read(&rove_journal::JournalQuery {
                kind: Some(rove_ai::audit::KIND_DELETED.to_owned()),
                limit: Some(10_000),
                newest_first: true,
                ..rove_journal::JournalQuery::default()
            })
            .map_err(journal_error)?;
        for tombstone in tombstones {
            if tombstone.ts < cutoff {
                break;
            }
            let Some(id) = tombstone.payload.get("session").and_then(|id| id.as_str()) else {
                continue;
            };
            if !records.iter().any(|record: &Record| record.id == id) {
                records.push(Record {
                    kind: CONVERSATION_KIND.to_owned(),
                    id: id.to_owned(),
                    body: serde_json::json!({ "deleted": true }),
                });
            }
        }
        for id in ids.into_iter().filter(|id| !deleted.contains(id)) {
            let entries: Vec<(i64, String, serde_json::Value)> = self
                .session(&id)?
                .into_iter()
                .map(|record| (record.ts, record.kind, record.payload))
                .collect();
            if let Some(body) = rove_ai::audit::conversation_body(&entries) {
                records.push(Record {
                    kind: CONVERSATION_KIND.to_owned(),
                    id,
                    body,
                });
            }
        }
        Ok(records)
    }

    fn apply(&self, changes: &[Change]) -> Result<(), SyncError> {
        let deleted =
            crate::commands::ai::deleted_conversations(self.0.as_ref()).map_err(journal_error)?;
        for change in changes {
            // Ids and sizes from another device are checked like any outside input.
            let Some(body) = &change.body else { continue };
            if !change.id.starts_with("ai-")
                || change.id.len() > 64
                || body.to_string().len() > rove_ai::audit::SYNC_MAX * 2
            {
                continue;
            }
            if body.get("deleted") == Some(&serde_json::Value::Bool(true)) {
                crate::commands::ai::delete_conversation(self.0.as_ref(), &change.id)
                    .map_err(journal_error)?;
                continue;
            }
            // Deleted here: what another device still has does not bring it back.
            if deleted.contains(&change.id) {
                continue;
            }
            let local: Vec<(i64, String)> = self
                .session(&change.id)?
                .into_iter()
                .map(|record| (record.ts, record.kind))
                .collect();
            for (ts, kind, payload) in rove_ai::audit::missing_records(&local, body) {
                self.0
                    .import(
                        ts,
                        rove_journal::NewRecord {
                            kind,
                            session_id: Some(change.id.clone()),
                            payload,
                        },
                    )
                    .map_err(journal_error)?;
            }
        }
        Ok(())
    }
}

impl RecordStore for ConversationStore {
    fn records(&self) -> Result<Vec<Record>, SyncError> {
        ConversationStore::records(self)
    }

    fn apply(&self, changes: &[Change]) -> Result<(), SyncError> {
        ConversationStore::apply(self, changes)
    }
}

/// The vault as the sync engine's record store.
struct VaultStore(Arc<Vault>);

impl RecordStore for VaultStore {
    fn records(&self) -> Result<Vec<Record>, SyncError> {
        let records = self
            .0
            .sync_records()
            .map_err(|error| SyncError::Store(error.to_string()))?;
        Ok(records
            .into_iter()
            .map(|record| Record {
                kind: record.kind.to_owned(),
                id: record.id,
                body: record.body,
            })
            .collect())
    }

    fn apply(&self, changes: &[Change]) -> Result<(), SyncError> {
        let changes: Vec<SyncChange> = changes
            .iter()
            .map(|change| SyncChange {
                kind: change.kind.clone(),
                id: change.id.clone(),
                body: change.body.clone(),
            })
            .collect();
        self.0
            .apply_sync(&changes)
            .map_err(|error| SyncError::Store(error.to_string()))
    }
}

/// This device's sync settings as stored in the vault.
struct Local {
    target: SyncTarget,
    state: SyncState,
    key: Option<SyncKey>,
    secret: Option<Zeroizing<String>>,
}

fn read_local(vault: &Vault) -> Result<Option<Local>, CommandError> {
    let Some(local) = vault.sync_local()? else {
        return Ok(None);
    };
    let target: SyncTarget = serde_json::from_value(local.target)
        .map_err(|error| CommandError::new("sync.format", error.to_string()))?;
    let state: SyncState = serde_json::from_value(local.state)
        .map_err(|error| CommandError::new("sync.format", error.to_string()))?;
    let key = local.key.map(|bytes| SyncKey::from_bytes(*bytes));
    Ok(Some(Local {
        target,
        state,
        key,
        secret: local.secret,
    }))
}

fn write_local(
    vault: &Vault,
    target: &SyncTarget,
    state: &SyncState,
    key: Option<&SyncKey>,
    secret: Option<&Zeroizing<String>>,
) -> Result<(), CommandError> {
    let local = SyncLocal {
        target: serde_json::to_value(target)
            .map_err(|error| CommandError::new("sync.format", error.to_string()))?,
        state: serde_json::to_value(state)
            .map_err(|error| CommandError::new("sync.format", error.to_string()))?,
        key: key.map(SyncKey::to_bytes),
        secret: secret.cloned(),
    };
    Ok(vault.set_sync_local(Some(&local))?)
}

/// What connecting to the storage found.
#[derive(Debug, Serialize)]
pub struct SyncProbe {
    /// A sync space already exists there: the next step joins it rather than creating one.
    pub exists: bool,
}

/// The state and key kept when connecting to `target`. Reconnecting to the storage this device
/// already syncs with keeps its device id, history and key; otherwise every local record would
/// look new and win over newer edits from other devices, and the old segments would be orphaned.
fn kept_state(previous: Option<Local>, target: &SyncTarget) -> (SyncState, Option<SyncKey>) {
    match previous {
        Some(local) if local.target == *target => (local.state, local.key),
        _ => (SyncState::new(&uuid::Uuid::now_v7().to_string()), None),
    }
}

/// Reports whether and how sync is set up on this device.
#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> Result<SyncStatus, CommandError> {
    Ok(match read_local(&state.vault)? {
        Some(local) => {
            let connected =
                local.secret.is_some() || matches!(local.target, SyncTarget::Folder { .. });
            SyncStatus {
                configured: true,
                connected,
                ready: connected && local.key.is_some(),
                target: Some(local.target),
                device: Some(local.state.device),
                records: local
                    .state
                    .known
                    .values()
                    .filter(|known| !known.deleted)
                    .count(),
            }
        }
        None => SyncStatus {
            configured: false,
            connected: false,
            target: None,
            ready: false,
            device: None,
            records: 0,
        },
    })
}

/// Connects to the storage at `target` (checking the address and, for WebDAV,
/// `storage_password`), stores both on this device (the password sealed) and reports whether a
/// sync space exists there. Sync starts with [`sync_setup`].
#[tauri::command]
pub async fn sync_connect(
    state: State<'_, AppState>,
    target: SyncTarget,
    storage_password: Option<String>,
) -> Result<SyncProbe, CommandError> {
    let secret = storage_password
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new);
    if secret
        .as_ref()
        .is_some_and(|value| value.len() > MAX_PASSWORD)
    {
        return Err(CommandError::new(
            "input.invalid",
            "storage password too long",
        ));
    }
    state.ensure_unlocked().await?;
    let backend = target.backend(secret.as_deref().map(String::as_str))?;
    let exists = rove_sync::space_exists(backend.as_ref())
        .await
        .map_err(sync_error)?;
    let _guard = state.sync_lock.lock().await;
    let (sync_state, key) = kept_state(read_local(&state.vault)?, &target);
    write_local(
        &state.vault,
        &target,
        &sync_state,
        key.as_ref(),
        secret.as_ref(),
    )?;
    Ok(SyncProbe { exists })
}

/// Looks again whether a space exists at the connected storage.
#[tauri::command]
pub async fn sync_probe(state: State<'_, AppState>) -> Result<SyncProbe, CommandError> {
    state.ensure_unlocked().await?;
    let local = read_local(&state.vault)?
        .ok_or_else(|| CommandError::new("sync.not_configured", "sync is not set up"))?;
    let backend = local
        .target
        .backend(local.secret.as_deref().map(String::as_str))?;
    let exists = rove_sync::space_exists(backend.as_ref())
        .await
        .map_err(sync_error)?;
    Ok(SyncProbe { exists })
}

/// Starts syncing with the connected storage: joins the space there with `password`, or creates
/// one protected by it when there is none (at least 8 characters). Then runs a first round.
#[tauri::command]
pub async fn sync_setup(
    state: State<'_, AppState>,
    password: String,
) -> Result<Report, CommandError> {
    let password = Zeroizing::new(password);
    if password.len() > MAX_PASSWORD {
        return Err(CommandError::new("input.invalid", "sync password too long"));
    }
    state.ensure_unlocked().await?;
    let _guard = state.sync_lock.lock().await;
    let local = read_local(&state.vault)?
        .ok_or_else(|| CommandError::new("sync.not_configured", "sync is not set up"))?;
    let backend = local
        .target
        .backend(local.secret.as_deref().map(String::as_str))?;
    let create = !rove_sync::space_exists(backend.as_ref())
        .await
        .map_err(sync_error)?;
    if create && password.chars().count() < MIN_PASSWORD {
        return Err(CommandError::new(
            "sync.weak_password",
            "the sync password must have at least 8 characters",
        ));
    }
    // Another device may create the space at the same moment: then this one joins it.
    let (key, created) = if create {
        rove_sync::create_or_join_space(backend.as_ref(), &password, KdfParams::default()).await
    } else {
        rove_sync::join_space(backend.as_ref(), &password)
            .await
            .map(|key| (key, false))
    }
    .map_err(sync_error)?;
    // A new space starts a new history.
    let sync_state = if created {
        SyncState::new(&uuid::Uuid::now_v7().to_string())
    } else {
        local.state
    };
    round(
        state.inner(),
        backend.as_ref(),
        &local.target,
        &key,
        sync_state,
        local.secret.as_ref(),
    )
    .await
}

/// Runs one round with `key` and saves the state afterwards, also when the round failed (it
/// then holds the remote changes already written to the vault). A space that no longer opens
/// with `key` stops syncing until the password is entered again.
async fn round(
    state: &AppState,
    backend: &dyn SyncBackend,
    target: &SyncTarget,
    key: &SyncKey,
    mut sync_state: SyncState,
    secret: Option<&Zeroizing<String>>,
) -> Result<Report, CommandError> {
    let vault = &state.vault;
    if let Err(error) = rove_sync::verify_space(backend, key).await {
        if matches!(error, SyncError::SpaceChanged) {
            write_local(vault, target, &sync_state, None, secret)?;
        }
        return Err(sync_error(error));
    }
    let store = SyncStore {
        vault: VaultStore(vault.clone()),
        conversations: ConversationStore(state.journal.clone()),
    };
    let result = rove_sync::sync(backend, key, &mut sync_state, &store).await;
    write_local(vault, target, &sync_state, Some(key), secret)?;
    result.map_err(sync_error)
}

/// After the vault's secrets were reset: the sync state forgets them, so the next round
/// restores them from the space instead of sending their absence to the other devices. The
/// sync key went with the device key; the password has to be entered again.
pub(crate) async fn after_secrets_reset(state: &AppState) -> Result<(), CommandError> {
    let _guard = state.sync_lock.lock().await;
    let Some(mut local) = read_local(&state.vault)? else {
        return Ok(());
    };
    local.state.defer_to_space(&rove_vault::SYNC_SECRET_KINDS);
    write_local(&state.vault, &local.target, &local.state, None, None)
}

/// Runs one sync round. Fails with `sync.not_configured` when sync is not set up and with
/// `sync.needs_password` when this device does not hold the key.
#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> Result<Report, CommandError> {
    state.ensure_unlocked().await?;
    let _guard = state.sync_lock.lock().await;
    let Local {
        target,
        state: sync_state,
        key,
        secret,
    } = read_local(&state.vault)?
        .ok_or_else(|| CommandError::new("sync.not_configured", "sync is not set up"))?;
    let key = key.ok_or_else(|| {
        CommandError::new(
            "sync.needs_password",
            "enter the sync password on this device",
        )
    })?;
    let backend = target.backend(secret.as_deref().map(String::as_str))?;
    round(
        state.inner(),
        backend.as_ref(),
        &target,
        &key,
        sync_state,
        secret.as_ref(),
    )
    .await
}

/// Turns sync off on this device. The space and the data on other devices are left alone.
#[tauri::command]
pub async fn sync_disable(state: State<'_, AppState>) -> Result<(), CommandError> {
    let _guard = state.sync_lock.lock().await;
    Ok(state.vault.set_sync_local(None)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(target: SyncTarget, device: &str) -> Local {
        Local {
            target,
            state: SyncState::new(device),
            key: Some(SyncKey::from_bytes([7; 32])),
            secret: None,
        }
    }

    #[test]
    fn reconnecting_to_the_same_storage_keeps_the_device_state() {
        let folder = SyncTarget::Folder {
            path: "/sync".into(),
        };
        let other = SyncTarget::WebDav {
            url: "https://dav.example/sync".into(),
            username: "me".into(),
        };
        let (kept, key) = kept_state(Some(local(folder.clone(), "device-1")), &folder);
        assert_eq!(kept.device, "device-1");
        assert!(key.is_some());
        // Other storage, or no previous setup, start fresh and without a key.
        for previous in [Some(local(folder.clone(), "device-1")), None] {
            let (fresh, key) = kept_state(previous, &other);
            assert_ne!(fresh.device, "device-1");
            assert!(key.is_none());
        }
    }

    /// A conversation recorded on one device shows up on another, continues there, and the
    /// continuation comes back without repeats.
    #[test]
    fn conversations_sync_both_ways() {
        use rove_journal::{JournalQuery, JournalStore, NewRecord, SqliteJournal};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let space = tempfile::tempdir().unwrap();
        let backend = FolderBackend::new(space.path());
        let key = SyncKey::from_bytes([3; 32]);
        let device = |name: &str| {
            let journal: Arc<dyn JournalStore> = Arc::new(SqliteJournal::open_in_memory().unwrap());
            (
                journal.clone(),
                ConversationStore(journal),
                SyncState::new(name),
            )
        };
        let (journal_a, store_a, mut state_a) = device("a");
        let (journal_b, store_b, mut state_b) = device("b");
        let add = |journal: &Arc<dyn JournalStore>, kind: &str, text: &str| {
            journal
                .append(NewRecord {
                    kind: kind.into(),
                    session_id: Some("ai-1".into()),
                    payload: serde_json::json!({ "text": text }),
                })
                .unwrap();
        };
        add(&journal_a, rove_ai::audit::KIND_STARTED, "");
        add(&journal_a, rove_ai::audit::KIND_PROMPT, "disk?");
        let texts = |journal: &Arc<dyn JournalStore>| -> Vec<String> {
            journal
                .read(&JournalQuery {
                    session_id: Some("ai-1".into()),
                    ..JournalQuery::default()
                })
                .unwrap()
                .into_iter()
                .map(|record| {
                    format!(
                        "{}:{}",
                        record.kind,
                        record.payload["text"].as_str().unwrap_or("")
                    )
                })
                .collect()
        };
        runtime.block_on(async {
            rove_sync::sync(&backend, &key, &mut state_a, &store_a)
                .await
                .unwrap();
            rove_sync::sync(&backend, &key, &mut state_b, &store_b)
                .await
                .unwrap();
        });
        assert_eq!(texts(&journal_b).len(), 2);

        add(&journal_b, rove_ai::audit::KIND_PROMPT, "thanks");
        runtime.block_on(async {
            rove_sync::sync(&backend, &key, &mut state_b, &store_b)
                .await
                .unwrap();
            rove_sync::sync(&backend, &key, &mut state_a, &store_a)
                .await
                .unwrap();
            // Nothing new either way: the stores settle.
            let settled = rove_sync::sync(&backend, &key, &mut state_a, &store_a)
                .await
                .unwrap();
            assert_eq!(settled.pushed, 0);
        });
        let mut a = texts(&journal_a);
        let mut b = texts(&journal_b);
        a.sort();
        b.sort();
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);

        // Deleted on `a`: gone on `b` too, and `b`'s copy does not bring it back to `a`.
        crate::commands::ai::delete_conversation(journal_a.as_ref(), "ai-1").unwrap();
        runtime.block_on(async {
            rove_sync::sync(&backend, &key, &mut state_a, &store_a)
                .await
                .unwrap();
            rove_sync::sync(&backend, &key, &mut state_b, &store_b)
                .await
                .unwrap();
            rove_sync::sync(&backend, &key, &mut state_a, &store_a)
                .await
                .unwrap();
        });
        assert!(texts(&journal_b).is_empty());
        assert!(texts(&journal_a).is_empty());
        assert!(
            crate::commands::ai::deleted_conversations(journal_b.as_ref())
                .unwrap()
                .contains("ai-1")
        );
    }
}
