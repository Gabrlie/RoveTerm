//! External editor IPC and application-exit protection.

use std::sync::{Arc, atomic::Ordering};

use rove_fs::{FileSystem, FsId};
use rove_vault::ExternalEditorPrefs;
use tauri::{AppHandle, Emitter, State};

use crate::error::CommandError;
use crate::external_edit::{EditError, EditId, ExternalEditUpdate, UpdateSink};
use crate::state::AppState;

impl From<EditError> for CommandError {
    fn from(error: EditError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

fn events(app: AppHandle) -> UpdateSink {
    Arc::new(move |update| {
        if let Err(error) = app.emit("external-edit-update", update) {
            tracing::debug!(%error, "external edit update event unavailable");
        }
    })
}

fn editor(state: &AppState) -> Result<ExternalEditorPrefs, CommandError> {
    if !cfg!(desktop) {
        return Err(CommandError::new(
            "fs.unsupported",
            "external editing requires a desktop",
        ));
    }
    Ok(state
        .vault
        .device_preferences()?
        .external_editor
        .validate()?)
}

fn reopened(
    state: &AppState,
    id: Option<FsId>,
) -> Result<Option<Arc<dyn FileSystem>>, CommandError> {
    id.map(|id| {
        state
            .fs_sessions
            .get(id)
            .map_err(|_| EditError::Reconnect.into())
    })
    .transpose()
}

#[tauri::command]
pub async fn external_edit_start(
    app: AppHandle,
    state: State<'_, AppState>,
    id: FsId,
    path: String,
) -> Result<ExternalEditUpdate, CommandError> {
    let editor = editor(&state)?;
    let fs = state.fs_sessions.get(id)?;
    Ok(state
        .external_edits
        .start(events(app), fs, &path, editor)
        .await?)
}

#[tauri::command]
pub fn external_edit_list(state: State<'_, AppState>) -> Vec<ExternalEditUpdate> {
    state.external_edits.list()
}

#[tauri::command]
pub async fn external_edit_open_copy(
    app: AppHandle,
    state: State<'_, AppState>,
    edit_id: EditId,
) -> Result<(), CommandError> {
    Ok(state
        .external_edits
        .reopen(edit_id, editor(&state)?, events(app))
        .await?)
}

#[tauri::command]
pub async fn external_edit_remote_version(
    state: State<'_, AppState>,
    edit_id: EditId,
    fs_id: Option<FsId>,
) -> Result<String, CommandError> {
    Ok(state
        .external_edits
        .remote_version(edit_id, reopened(&state, fs_id)?)
        .await?)
}

#[tauri::command]
pub async fn external_edit_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    edit_id: EditId,
    fs_id: Option<FsId>,
    approved_version: Option<String>,
) -> Result<(), CommandError> {
    Ok(state
        .external_edits
        .retry(
            edit_id,
            reopened(&state, fs_id)?,
            approved_version,
            events(app),
        )
        .await?)
}

#[tauri::command]
pub async fn external_edit_finish(
    app: AppHandle,
    state: State<'_, AppState>,
    edit_id: EditId,
    fs_id: Option<FsId>,
) -> Result<(), CommandError> {
    Ok(state
        .external_edits
        .finish(edit_id, reopened(&state, fs_id)?, events(app))
        .await?)
}

#[tauri::command]
pub async fn external_edit_discard(
    app: AppHandle,
    state: State<'_, AppState>,
    edit_id: EditId,
) -> Result<(), CommandError> {
    Ok(state.external_edits.discard(edit_id, events(app)).await?)
}

#[tauri::command]
pub fn external_edit_reveal(
    state: State<'_, AppState>,
    edit_id: EditId,
) -> Result<(), CommandError> {
    Ok(state.external_edits.reveal(edit_id)?)
}

/// Marks unsaved in-memory drafts for the native quit handler; no text crosses this IPC.
#[tauri::command]
pub fn external_edit_set_dirty(state: State<'_, AppState>, dirty: bool) {
    state.external_edit_dirty.store(dirty, Ordering::Release);
}

#[tauri::command]
pub async fn external_edit_shutdown(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state.external_edits.shutdown().await;
    state.external_edit_dirty.store(false, Ordering::Release);
    app.exit(0);
    Ok(())
}
