//! Multi-device sync: set up a space, run a round, report status, turn sync off. The space is
//! described by a [`SyncTarget`]; the vault stores it with the sync state and the sealed key.

use std::path::Path;
use std::sync::Arc;

use rove_sync::{
    Change, FolderBackend, KdfParams, Record, RecordStore, Report, SyncBackend, SyncError, SyncKey,
    SyncState,
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
}

impl SyncTarget {
    /// Checks the target from the WebView and opens its backend.
    fn backend(&self) -> Result<Box<dyn SyncBackend>, CommandError> {
        match self {
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
    /// Sync is set up on this device.
    pub configured: bool,
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
        SyncError::Crypto(_) => "sync.crypto",
        SyncError::Store(_) => "sync.store",
    };
    CommandError::new(code, error.to_string())
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

fn read_local(
    vault: &Vault,
) -> Result<Option<(SyncTarget, SyncState, Option<SyncKey>)>, CommandError> {
    let Some(local) = vault.sync_local()? else {
        return Ok(None);
    };
    let target: SyncTarget = serde_json::from_value(local.target)
        .map_err(|error| CommandError::new("sync.format", error.to_string()))?;
    let state: SyncState = serde_json::from_value(local.state)
        .map_err(|error| CommandError::new("sync.format", error.to_string()))?;
    let key = local.key.map(|bytes| SyncKey::from_bytes(*bytes));
    Ok(Some((target, state, key)))
}

fn write_local(
    vault: &Vault,
    target: &SyncTarget,
    state: &SyncState,
    key: &SyncKey,
) -> Result<(), CommandError> {
    let local = SyncLocal {
        target: serde_json::to_value(target)
            .map_err(|error| CommandError::new("sync.format", error.to_string()))?,
        state: serde_json::to_value(state)
            .map_err(|error| CommandError::new("sync.format", error.to_string()))?,
        key: Some(key.to_bytes()),
    };
    Ok(vault.set_sync_local(Some(&local))?)
}

/// Reports whether and how sync is set up on this device.
#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> Result<SyncStatus, CommandError> {
    Ok(match read_local(&state.vault)? {
        Some((target, sync_state, key)) => SyncStatus {
            configured: true,
            target: Some(target),
            ready: key.is_some(),
            device: Some(sync_state.device),
            records: sync_state
                .known
                .values()
                .filter(|known| !known.deleted)
                .count(),
        },
        None => SyncStatus {
            configured: false,
            target: None,
            ready: false,
            device: None,
            records: 0,
        },
    })
}

/// Sets up sync on this device: creates a new space at `target` protected by `password`
/// (`create`), or joins the existing one there. Then runs a first round.
#[tauri::command]
pub async fn sync_setup(
    state: State<'_, AppState>,
    target: SyncTarget,
    password: String,
    create: bool,
) -> Result<Report, CommandError> {
    let password = Zeroizing::new(password);
    if password.len() > MAX_PASSWORD || (create && password.chars().count() < MIN_PASSWORD) {
        return Err(CommandError::new(
            "sync.weak_password",
            "the sync password must have at least 8 characters",
        ));
    }
    state.ensure_unlocked().await?;
    let backend = target.backend()?;
    let key = if create {
        rove_sync::create_space(backend.as_ref(), &password, KdfParams::default()).await
    } else {
        rove_sync::join_space(backend.as_ref(), &password).await
    }
    .map_err(sync_error)?;
    let mut sync_state = SyncState::new(&uuid::Uuid::now_v7().to_string());
    let _guard = state.sync_lock.lock().await;
    let report = rove_sync::sync(
        backend.as_ref(),
        &key,
        &mut sync_state,
        &VaultStore(state.vault.clone()),
    )
    .await
    .map_err(sync_error)?;
    write_local(&state.vault, &target, &sync_state, &key)?;
    Ok(report)
}

/// Runs one sync round. Fails with `sync.not_configured` when sync is not set up and with
/// `sync.needs_password` when this device does not hold the key.
#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> Result<Report, CommandError> {
    state.ensure_unlocked().await?;
    let _guard = state.sync_lock.lock().await;
    let (target, mut sync_state, key) = read_local(&state.vault)?
        .ok_or_else(|| CommandError::new("sync.not_configured", "sync is not set up"))?;
    let key = key.ok_or_else(|| {
        CommandError::new(
            "sync.needs_password",
            "enter the sync password on this device",
        )
    })?;
    let backend = target.backend()?;
    let report = rove_sync::sync(
        backend.as_ref(),
        &key,
        &mut sync_state,
        &VaultStore(state.vault.clone()),
    )
    .await
    .map_err(sync_error)?;
    write_local(&state.vault, &target, &sync_state, &key)?;
    Ok(report)
}

/// Turns sync off on this device. The space and the data on other devices are left alone.
#[tauri::command]
pub async fn sync_disable(state: State<'_, AppState>) -> Result<(), CommandError> {
    let _guard = state.sync_lock.lock().await;
    Ok(state.vault.set_sync_local(None)?)
}
