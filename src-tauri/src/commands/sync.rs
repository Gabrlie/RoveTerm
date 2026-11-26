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
        SyncError::Forbidden(_) => "sync.forbidden",
        SyncError::Store(_) => "sync.store",
    };
    CommandError::new(code, error.to_string())
}

fn target_kind(target: &SyncTarget) -> &'static str {
    match target {
        SyncTarget::Folder { .. } => "folder",
        SyncTarget::WebDav { .. } => "webdav",
    }
}

/// Safe target description for logs (omits credentials, queries and fragments).
fn target_summary(target: &SyncTarget) -> String {
    match target {
        SyncTarget::Folder { path } => path.clone(),
        SyncTarget::WebDav { url, username } => {
            let trimmed = url.trim();
            let without_query = trimmed.split(['?', '#']).next().unwrap_or("");
            let safe_url = match without_query.split_once("://") {
                Some((scheme, rest)) => {
                    let (authority, path) = match rest.find('/') {
                        Some(idx) => (&rest[..idx], &rest[idx..]),
                        None => (rest, ""),
                    };
                    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
                    format!("{scheme}://{host}{path}")
                }
                None => "<invalid-url>".to_owned(),
            };
            format!("{username}@{safe_url}")
        }
    }
}

/// Picks the storage password for [`sync_connect`]: folders use none; WebDAV uses the newly
/// entered password when non-empty, otherwise reuses the existing stored WebDAV password so the
/// user can edit URL, folder or user name without retyping it.
fn resolve_connect_secret(
    target: &SyncTarget,
    provided: Option<Zeroizing<String>>,
    previous: Option<&Local>,
) -> Option<Zeroizing<String>> {
    match target {
        SyncTarget::Folder { .. } => None,
        SyncTarget::WebDav { .. } => provided.or_else(|| {
            previous.and_then(|local| match local.target {
                SyncTarget::WebDav { .. } => local.secret.clone(),
                SyncTarget::Folder { .. } => None,
            })
        }),
    }
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
    fn receipts(
        &self,
        key: &SyncKey,
    ) -> Result<std::collections::BTreeMap<String, rove_sync::Known>, SyncError> {
        let space = key.fingerprint(b"rove/sync/receipts/v1");
        let mut receipts: std::collections::BTreeMap<_, _> = serde_json::from_value(
            self.vault
                .0
                .sync_receipt(&space)
                .map_err(|e| SyncError::Store(e.to_string()))?,
        )
        .map_err(|e| SyncError::Format(e.to_string()))?;
        let journal = self
            .conversations
            .0
            .sync_receipt(&space)
            .map_err(journal_error)?;
        receipts.extend(
            serde_json::from_value::<std::collections::BTreeMap<String, rove_sync::Known>>(journal)
                .map_err(|e| SyncError::Format(e.to_string()))?,
        );
        Ok(receipts)
    }

    fn apply_tracked(&self, changes: &[Change], key: &SyncKey) -> Result<(), SyncError> {
        let space = key.fingerprint(b"rove/sync/receipts/v1");
        let (conversations, others): (Vec<_>, Vec<_>) = changes
            .iter()
            .cloned()
            .partition(|change| change.kind == CONVERSATION_KIND);
        if !others.is_empty() {
            self.vault
                .0
                .transaction(|| {
                    self.vault.0.apply_sync_rows(&vault_changes(&others))?;
                    let old = self.vault.0.sync_receipt(&space)?;
                    let rows = self
                        .vault
                        .records()
                        .map_err(|e| rove_vault::VaultError::Invalid(e.to_string()))?;
                    let value = imported_versions(key, old, &others, rows)?;
                    self.vault.0.set_sync_receipt(&space, &value)
                })
                .map_err(|e| SyncError::Store(e.to_string()))?;
        }
        if !conversations.is_empty() {
            let store = &self.conversations;
            store
                .0
                .transaction(&mut || {
                    let err = |e: SyncError| rove_journal::JournalError::Io(e.to_string());
                    store.apply(&conversations).map_err(err)?;
                    let value = imported_versions(
                        key,
                        store.0.sync_receipt(&space)?,
                        &conversations,
                        store.records().map_err(err)?,
                    )?;
                    store.0.set_sync_receipt(&space, &value)
                })
                .map_err(journal_error)?;
        }
        Ok(())
    }

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

fn imported_versions(
    key: &SyncKey,
    old: serde_json::Value,
    changes: &[Change],
    rows: Vec<Record>,
) -> Result<serde_json::Value, serde_json::Error> {
    use std::collections::BTreeMap;
    let mut receipts: BTreeMap<String, rove_sync::Known> = serde_json::from_value(old)?;
    let rows: BTreeMap<_, _> = rows.into_iter().map(|r| ((r.kind, r.id), r.body)).collect();
    for change in changes {
        let body = rows.get(&(change.kind.clone(), change.id.clone()));
        let fingerprint = match body {
            Some(body) => key.fingerprint(&serde_json::to_vec(body)?),
            None => String::new(),
        };
        receipts.insert(
            format!("{}:{}", change.kind, change.id),
            rove_sync::Known {
                hlc: change.hlc.clone(),
                fingerprint,
                deleted: body.is_none(),
            },
        );
    }
    serde_json::to_value(receipts)
}

/// Recent AI conversations as sync records; arriving ones are merged into the journal (records
/// it lacks are added with their own time, and deletion tombstones are applied).
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
                limit: Some(u32::MAX),
                newest_first: true,
                ..rove_journal::JournalQuery::default()
            })
            .map_err(journal_error)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        let cutoff = now - CONVERSATION_AGE_MS;
        let deleted =
            crate::commands::ai::deleted_conversations(self.0.as_ref()).map_err(journal_error)?;
        let mut seen = std::collections::HashSet::new();
        let mut ids = Vec::new();
        for start in starts {
            let Some(id) = start.session_id.filter(|_| start.ts >= cutoff) else {
                continue;
            };
            if !seen.insert(id.clone())
                || start.payload["parent"].as_str().is_some()
                || deleted.contains(&id)
            {
                continue;
            }
            ids.push(id);
            if ids.len() >= MAX_CONVERSATIONS {
                break;
            }
        }
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
        for id in ids {
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
            if deleted.contains(&change.id)
                || body["records"].as_array().is_some_and(|records| {
                    records.iter().any(|record| {
                        record["kind"] == rove_ai::audit::KIND_STARTED
                            && record["payload"]["parent"].as_str().is_some()
                    })
                })
            {
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

fn vault_changes(changes: &[Change]) -> Vec<SyncChange> {
    changes
        .iter()
        .map(|change| SyncChange {
            kind: change.kind.clone(),
            id: change.id.clone(),
            body: change.body.clone(),
        })
        .collect()
}

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
        let changes = vault_changes(changes);
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

#[cfg(desktop)]
pub(super) async fn reserve_schedule(
    vault: &Vault,
    task: &rove_vault::Schedule,
    owner: uuid::Uuid,
) -> Result<(), CommandError> {
    let Some(local) = read_local(vault)? else {
        return Ok(());
    };
    if let SyncTarget::Folder { path } = &local.target
        && !shared_claim_volume(Path::new(path))
    {
        return Err(CommandError::new(
            "schedule.claim_storage",
            "independent directory replicas cannot arbitrate a schedule claim",
        ));
    }
    let key = local
        .key
        .ok_or_else(|| CommandError::new("sync.needs_password", "unlock the sync space first"))?;
    let backend = local
        .target
        .backend(local.secret.as_deref().map(String::as_str))?;
    rove_sync::verify_space(backend.as_ref(), &key)
        .await
        .map_err(sync_error)?;
    reserve_in_space(backend.as_ref(), &key, &task.id, owner).await
}

/// An exclusive create arbitrates between devices only when they address the same live filesystem.
#[cfg(any(desktop, test))]
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn shared_claim_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut storage = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: statfs writes the complete result on success; the path stays alive for the call.
    let volume = unsafe {
        if libc::statfs(path.as_ptr(), storage.as_mut_ptr()) != 0 {
            return false;
        }
        storage.assume_init()
    };
    #[cfg(target_os = "macos")]
    {
        let name: Vec<_> = volume
            .f_fstypename
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        matches!(name.as_slice(), b"nfs" | b"smbfs" | b"afpfs" | b"webdav")
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // Linux statfs magic numbers for NFS, SMB and CIFS.
        matches!(
            volume.f_type as u32,
            0x6969 | 0x517b | 0xff534d42 | 0xfe534d42
        )
    }
}

#[cfg(windows)]
fn shared_claim_volume(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::UNC(..)|Prefix::VerbatimUNC(..)))
}

#[cfg(any(desktop, test))]
#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn shared_claim_volume(_: &Path) -> bool {
    false
}

#[cfg(any(desktop, test))]
async fn reserve_in_space(
    backend: &dyn SyncBackend,
    key: &SyncKey,
    id: &str,
    owner: uuid::Uuid,
) -> Result<(), CommandError> {
    let identity = format!("schedule:{id}");
    let path = format!("claims/{}", key.fingerprint(identity.as_bytes()));
    let sealed = key
        .seal(owner.as_bytes(), path.as_bytes())
        .map_err(sync_error)?;
    if !backend.put_new(&path, &sealed).await.map_err(sync_error)? {
        let existing = backend
            .get(&path)
            .await
            .map_err(sync_error)?
            .ok_or_else(|| CommandError::new("sync.format", "schedule claim unavailable"))?;
        if key
            .open(&existing, path.as_bytes())
            .map_err(sync_error)?
            .as_slice()
            != owner.as_bytes()
        {
            return Err(CommandError::new(
                "schedule.claimed",
                "another device owns this task",
            ));
        }
    }
    Ok(())
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
/// sync space exists there. Leaving `storage_password` empty while updating an existing WebDAV
/// configuration reuses the currently saved WebDAV password. If the connection check fails, the
/// previously saved configuration is left untouched. Sync starts with [`sync_setup`].
#[tauri::command]
pub async fn sync_connect(
    state: State<'_, AppState>,
    target: SyncTarget,
    storage_password: Option<String>,
) -> Result<SyncProbe, CommandError> {
    let provided = storage_password
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new);
    if provided
        .as_ref()
        .is_some_and(|value| value.len() > MAX_PASSWORD)
    {
        return Err(CommandError::new(
            "input.invalid",
            "storage password too long",
        ));
    }
    state.ensure_unlocked().await?;
    let previous = read_local(&state.vault)?;
    let reused_secret = provided.is_none()
        && matches!(target, SyncTarget::WebDav { .. })
        && previous
            .as_ref()
            .is_some_and(|local| matches!(local.target, SyncTarget::WebDav { .. }) && local.secret.is_some());
    let secret = resolve_connect_secret(&target, provided, previous.as_ref());
    let kind = target_kind(&target);
    let summary = target_summary(&target);
    tracing::info!(kind, target = %summary, reused_secret, "testing sync storage connection");
    let backend = match target.backend(secret.as_deref().map(String::as_str)) {
        Ok(backend) => backend,
        Err(error) => {
            tracing::warn!(
                kind,
                target = %summary,
                code = error.code,
                error = %error.message,
                "sync storage target invalid"
            );
            return Err(error);
        }
    };
    let exists = match rove_sync::space_exists(backend.as_ref()).await {
        Ok(exists) => exists,
        Err(error) => {
            let cmd_error = sync_error(error);
            tracing::warn!(
                kind,
                target = %summary,
                code = cmd_error.code,
                error = %cmd_error.message,
                "sync storage connection failed"
            );
            return Err(cmd_error);
        }
    };
    let _guard = state.sync_lock.lock().await;
    let (sync_state, key) = kept_state(read_local(&state.vault)?, &target);
    write_local(
        &state.vault,
        &target,
        &sync_state,
        key.as_ref(),
        secret.as_ref(),
    )?;
    tracing::info!(
        kind,
        target = %summary,
        space_exists = exists,
        kept_key = key.is_some(),
        "sync storage connected"
    );
    Ok(SyncProbe { exists })
}

/// Looks again whether a space exists at the connected storage.
#[tauri::command]
pub async fn sync_probe(state: State<'_, AppState>) -> Result<SyncProbe, CommandError> {
    state.ensure_unlocked().await?;
    let local = read_local(&state.vault)?
        .ok_or_else(|| CommandError::new("sync.not_configured", "sync is not set up"))?;
    let kind = target_kind(&local.target);
    let summary = target_summary(&local.target);
    let backend = local
        .target
        .backend(local.secret.as_deref().map(String::as_str))?;
    match rove_sync::space_exists(backend.as_ref()).await {
        Ok(exists) => {
            tracing::info!(kind, target = %summary, space_exists = exists, "sync storage probed");
            Ok(SyncProbe { exists })
        }
        Err(error) => {
            let cmd_error = sync_error(error);
            tracing::warn!(
                kind,
                target = %summary,
                code = cmd_error.code,
                error = %cmd_error.message,
                "sync storage probe failed"
            );
            Err(cmd_error)
        }
    }
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
    let kind = target_kind(&local.target);
    let summary = target_summary(&local.target);
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
    tracing::info!(kind, target = %summary, create, "starting sync space setup");
    // Another device may create the space at the same moment: then this one joins it.
    let (key, created) = match if create {
        rove_sync::create_or_join_space(backend.as_ref(), &password, KdfParams::default()).await
    } else {
        rove_sync::join_space(backend.as_ref(), &password)
            .await
            .map(|key| (key, false))
    } {
        Ok(pair) => pair,
        Err(error) => {
            let cmd_error = sync_error(error);
            tracing::warn!(
                kind,
                target = %summary,
                create,
                code = cmd_error.code,
                error = %cmd_error.message,
                "sync space setup failed"
            );
            return Err(cmd_error);
        }
    };
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
    let kind = target_kind(target);
    let summary = target_summary(target);
    if let Err(error) = rove_sync::verify_space(backend, key).await {
        if matches!(error, SyncError::SpaceChanged) {
            write_local(vault, target, &sync_state, None, secret)?;
        }
        let cmd_error = sync_error(error);
        tracing::warn!(
            kind,
            target = %summary,
            code = cmd_error.code,
            error = %cmd_error.message,
            "sync space verification failed"
        );
        return Err(cmd_error);
    }
    let store = SyncStore {
        vault: VaultStore(vault.clone()),
        conversations: ConversationStore(state.journal.clone()),
    };
    let result = rove_sync::sync(backend, key, &mut sync_state, &store).await;
    write_local(vault, target, &sync_state, Some(key), secret)?;
    match result {
        Ok(report) => {
            tracing::info!(
                kind,
                target = %summary,
                applied = report.applied,
                pushed = report.pushed,
                "sync round completed"
            );
            Ok(report)
        }
        Err(error) => {
            let cmd_error = sync_error(error);
            tracing::warn!(
                kind,
                target = %summary,
                code = cmd_error.code,
                error = %cmd_error.message,
                "sync round failed"
            );
            Err(cmd_error)
        }
    }
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

/// The stored WebDAV password when WebDAV sync is configured on this device.
fn revealed_webdav_secret(local: Option<Local>) -> Option<String> {
    let local = local?;
    match local.target {
        SyncTarget::WebDav { .. } => local.secret.map(|secret| secret.to_string()),
        SyncTarget::Folder { .. } => None,
    }
}

/// Returns the saved WebDAV password so the user can inspect or edit it on this device.
#[tauri::command]
pub async fn sync_reveal_secret(state: State<'_, AppState>) -> Result<Option<String>, CommandError> {
    state.ensure_unlocked().await?;
    Ok(revealed_webdav_secret(read_local(&state.vault)?))
}

/// Turns sync off on this device. The space and the data on other devices are left alone.
#[tauri::command]
pub async fn sync_disable(state: State<'_, AppState>) -> Result<(), CommandError> {
    let _guard = state.sync_lock.lock().await;
    state.vault.set_sync_local(None)?;
    tracing::info!("sync disabled on this device");
    Ok(())
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
    fn reuses_stored_webdav_secret_when_editing_without_retyping_password() {
        let dav = SyncTarget::WebDav {
            url: "https://dav.example/RoveTerm".into(),
            username: "me".into(),
        };
        let dav_edited = SyncTarget::WebDav {
            url: "https://dav.example/RoveTerm2".into(),
            username: "me".into(),
        };
        let mut previous = local(dav.clone(), "device-1");
        previous.secret = Some(Zeroizing::new("stored-pass".into()));

        let reused = resolve_connect_secret(&dav_edited, None, Some(&previous));
        assert_eq!(reused.as_deref().map(String::as_str), Some("stored-pass"));

        let replaced = resolve_connect_secret(
            &dav_edited,
            Some(Zeroizing::new("new-pass".into())),
            Some(&previous),
        );
        assert_eq!(replaced.as_deref().map(String::as_str), Some("new-pass"));

        let folder = SyncTarget::Folder {
            path: "/sync".into(),
        };
        assert!(resolve_connect_secret(&folder, None, Some(&previous)).is_none());
        assert!(resolve_connect_secret(&dav, None, None).is_none());
    }

    #[test]
    fn reveals_only_saved_webdav_secret() {
        let dav = SyncTarget::WebDav {
            url: "https://dav.example/RoveTerm".into(),
            username: "me".into(),
        };
        let mut with_secret = local(dav.clone(), "device-1");
        with_secret.secret = Some(Zeroizing::new("stored-pass".into()));
        assert_eq!(
            revealed_webdav_secret(Some(with_secret)).as_deref(),
            Some("stored-pass")
        );

        let without_secret = local(dav, "device-1");
        assert!(revealed_webdav_secret(Some(without_secret)).is_none());

        let mut folder = local(
            SyncTarget::Folder {
                path: "/sync".into(),
            },
            "device-1",
        );
        folder.secret = Some(Zeroizing::new("ignored".into()));
        assert!(revealed_webdav_secret(Some(folder)).is_none());
        assert!(revealed_webdav_secret(None).is_none());
    }

    #[test]
    fn maps_forbidden_and_unauthorized_to_distinct_codes() {
        assert_eq!(sync_error(SyncError::Unauthorized).code, "sync.unauthorized");
        assert_eq!(
            sync_error(SyncError::Forbidden("HTTP 403".into())).code,
            "sync.forbidden"
        );
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

    #[test]
    fn workers_do_not_consume_the_synced_conversation_limit() {
        use rove_journal::{JournalStore, NewRecord, SqliteJournal};

        let journal = Arc::new(SqliteJournal::open_in_memory().unwrap());
        let start = |id, parent| {
            journal
                .append(NewRecord {
                    kind: rove_ai::audit::KIND_STARTED.into(),
                    session_id: Some(id),
                    payload: serde_json::json!({ "parent": parent }),
                })
                .unwrap();
        };
        for index in 0..=MAX_CONVERSATIONS {
            start(format!("ai-lead-{index}"), None);
        }
        for index in 0..1024 {
            start(format!("ai-worker-{index}"), Some("ai-lead-1"));
        }
        let store = ConversationStore(journal);
        let records = store.records().unwrap();
        assert_eq!(records.len(), MAX_CONVERSATIONS);
        assert!(
            records
                .iter()
                .all(|record| record.id.starts_with("ai-lead-"))
        );
        assert!(!records.iter().any(|record| record.id == "ai-lead-0"));
    }

    #[test]
    fn legacy_worker_uploads_are_ignored_and_their_tombstones_apply() {
        use rove_journal::{JournalStore, NewRecord, SqliteJournal};
        use serde_json::json;

        let journal = Arc::new(SqliteJournal::open_in_memory().unwrap());
        let store = ConversationStore(journal.clone());
        let id = "ai-worker";
        let payload = json!({ "parent": "ai-lead" });
        let mut change = Change {
            kind: CONVERSATION_KIND.into(),
            id: id.into(),
            hlc: rove_sync::Hlc {
                wall: 1,
                counter: 0,
                device: "other".into(),
            },
            body: Some(json!({ "records": [{
                "ts": 1, "kind": rove_ai::audit::KIND_STARTED, "payload": payload,
            }] })),
        };
        store.apply(std::slice::from_ref(&change)).unwrap();
        assert!(store.session(id).unwrap().is_empty());

        journal
            .append(NewRecord {
                kind: rove_ai::audit::KIND_STARTED.into(),
                session_id: Some(id.into()),
                payload,
            })
            .unwrap();
        change.body = Some(json!({ "deleted": true }));
        store.apply(&[change]).unwrap();
        assert!(store.session(id).unwrap().is_empty());
        assert!(
            store
                .records()
                .unwrap()
                .iter()
                .any(|record| record.id == id && record.body["deleted"] == true)
        );
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
    #[tokio::test]
    async fn only_one_device_can_claim_a_shared_task() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FolderBackend::new(dir.path());
        let key = SyncKey::from_bytes([4; 32]);
        let a = uuid::Uuid::now_v7();
        let b = uuid::Uuid::now_v7();
        let (left, right) = tokio::join!(
            reserve_in_space(&backend, &key, "legacy", a),
            reserve_in_space(&backend, &key, "legacy", b)
        );
        assert_ne!(left.is_ok(), right.is_ok());
        let winner = if left.is_ok() { a } else { b };
        assert!(
            reserve_in_space(&backend, &key, "legacy", winner)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn recovery_after_a_journal_failure_respects_a_third_devices_update() {
        use rove_journal::{NewRecord, SqliteJournal};
        use rove_vault::{MemoryKeyCache, SnippetDraft};
        let root = tempfile::tempdir().unwrap();
        let disk = tempfile::tempdir().unwrap();
        let space = FolderBackend::new(root.path());
        let key = SyncKey::from_bytes([8; 32]);
        let cache = Arc::new(MemoryKeyCache::default());
        let open = |device: &str| {
            let vault = Arc::new(
                Vault::open(&disk.path().join(format!("{device}.db")), cache.clone()).unwrap(),
            );
            vault.try_unlock();
            let journal = Arc::new(
                SqliteJournal::open(&disk.path().join(format!("{device}-journal.db"))).unwrap(),
            );
            SyncStore {
                vault: VaultStore(vault),
                conversations: ConversationStore(journal),
            }
        };
        let a = open("a");
        let b = open("b");
        let c = open("c");
        let mut sa = SyncState::new("a");
        let mut sb = SyncState::new("b");
        let mut sc = SyncState::new("c");
        for (store, state) in [(&a, &mut sa), (&b, &mut sb), (&c, &mut sc)] {
            rove_sync::sync(&space, &key, state, store).await.unwrap();
        }
        let draft = |name: &str| SnippetDraft {
            name: name.into(),
            command: "pwd".into(),
        };
        let initial = a.vault.0.create_snippet(draft("imported")).unwrap();
        a.conversations
            .0
            .append(NewRecord {
                kind: rove_ai::audit::KIND_STARTED.into(),
                session_id: Some("ai-import".into()),
                payload: serde_json::json!({}),
            })
            .unwrap();
        rove_sync::sync(&space, &key, &mut sa, &a).await.unwrap();
        // A journal transaction fails after the Vault transaction has committed.
        let path = disk.path().join("b-journal.db");
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        assert!(rove_sync::sync(&space, &key, &mut sb, &b).await.is_err());
        assert_eq!(b.vault.0.list_snippets().unwrap()[0].name, "imported");
        drop(b);
        rove_sync::sync(&space, &key, &mut sc, &c).await.unwrap();
        c.vault
            .0
            .update_snippet(initial.id, draft("newer"))
            .unwrap();
        rove_sync::sync(&space, &key, &mut sc, &c).await.unwrap();
        sql.execute_batch("DROP TRIGGER fail_import").unwrap();
        let recovered = open("b");
        let result = rove_sync::sync(&space, &key, &mut sb, &recovered)
            .await
            .unwrap();
        assert_eq!(result.pushed, 0);
        assert_eq!(recovered.vault.0.list_snippets().unwrap()[0].name, "newer");
        rove_sync::sync(&space, &key, &mut sc, &c).await.unwrap();
        assert_eq!(c.vault.0.list_snippets().unwrap()[0].name, "newer");
    }
    #[test]
    fn an_independent_directory_cannot_arbitrate_between_devices() {
        let local = tempfile::tempdir().unwrap();
        assert!(!shared_claim_volume(local.path()));
    }
}
