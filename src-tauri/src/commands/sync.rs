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
        SyncError::Unauthorized => "sync.unauthorized",
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
    key: &SyncKey,
    secret: Option<&Zeroizing<String>>,
) -> Result<(), CommandError> {
    let local = SyncLocal {
        target: serde_json::to_value(target)
            .map_err(|error| CommandError::new("sync.format", error.to_string()))?,
        state: serde_json::to_value(state)
            .map_err(|error| CommandError::new("sync.format", error.to_string()))?,
        key: Some(key.to_bytes()),
        secret: secret.cloned(),
    };
    Ok(vault.set_sync_local(Some(&local))?)
}

/// The state a set-up round starts from. Re-entering the password for the space this device
/// already syncs with keeps its device id and history; otherwise every local record would look
/// new, win over newer edits from other devices, and the old segments would be orphaned.
fn initial_state(previous: Option<Local>, target: &SyncTarget, create: bool) -> SyncState {
    match previous {
        Some(local) if !create && local.target == *target => local.state,
        _ => SyncState::new(&uuid::Uuid::now_v7().to_string()),
    }
}

/// Reports whether and how sync is set up on this device.
#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> Result<SyncStatus, CommandError> {
    Ok(match read_local(&state.vault)? {
        Some(local) => SyncStatus {
            configured: true,
            ready: local.key.is_some()
                && (local.secret.is_some() || matches!(local.target, SyncTarget::Folder { .. })),
            target: Some(local.target),
            device: Some(local.state.device),
            records: local
                .state
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
/// (`create`), or joins the existing one there. Then runs a first round. `storage_password` is
/// the WebDAV password; it is stored sealed with the device key.
#[tauri::command]
pub async fn sync_setup(
    state: State<'_, AppState>,
    target: SyncTarget,
    password: String,
    create: bool,
    storage_password: Option<String>,
) -> Result<Report, CommandError> {
    let password = Zeroizing::new(password);
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
    if password.len() > MAX_PASSWORD || (create && password.chars().count() < MIN_PASSWORD) {
        return Err(CommandError::new(
            "sync.weak_password",
            "the sync password must have at least 8 characters",
        ));
    }
    state.ensure_unlocked().await?;
    let backend = target.backend(secret.as_deref().map(String::as_str))?;
    let key = if create {
        rove_sync::create_space(backend.as_ref(), &password, KdfParams::default()).await
    } else {
        rove_sync::join_space(backend.as_ref(), &password).await
    }
    .map_err(sync_error)?;
    let _guard = state.sync_lock.lock().await;
    let mut sync_state = initial_state(read_local(&state.vault)?, &target, create);
    let report = rove_sync::sync(
        backend.as_ref(),
        &key,
        &mut sync_state,
        &VaultStore(state.vault.clone()),
    )
    .await
    .map_err(sync_error)?;
    write_local(&state.vault, &target, &sync_state, &key, secret.as_ref())?;
    Ok(report)
}

/// Runs one sync round. Fails with `sync.not_configured` when sync is not set up and with
/// `sync.needs_password` when this device does not hold the key.
#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> Result<Report, CommandError> {
    state.ensure_unlocked().await?;
    let _guard = state.sync_lock.lock().await;
    let Local {
        target,
        state: mut sync_state,
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
    let report = rove_sync::sync(
        backend.as_ref(),
        &key,
        &mut sync_state,
        &VaultStore(state.vault.clone()),
    )
    .await
    .map_err(sync_error)?;
    write_local(&state.vault, &target, &sync_state, &key, secret.as_ref())?;
    Ok(report)
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
            key: None,
            secret: None,
        }
    }

    #[test]
    fn rejoining_the_same_space_keeps_the_device_state() {
        let folder = SyncTarget::Folder {
            path: "/sync".into(),
        };
        let other = SyncTarget::WebDav {
            url: "https://dav.example/sync".into(),
            username: "me".into(),
        };
        let kept = initial_state(Some(local(folder.clone(), "device-1")), &folder, false);
        assert_eq!(kept.device, "device-1");
        // Another target, a new space, or no previous setup each start fresh.
        for (previous, target, create) in [
            (Some(local(folder.clone(), "device-1")), &other, false),
            (Some(local(folder.clone(), "device-1")), &folder, true),
            (None, &folder, false),
        ] {
            assert_ne!(initial_state(previous, target, create).device, "device-1");
        }
    }
}
