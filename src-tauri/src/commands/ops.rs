//! IPC for host-scoped durable operations tasks.

use rove_ops::{OpsTask, OpsTaskId, PrivateRef};
use rove_vault::HostId;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::{error::CommandError, ops_runtime, state::AppState};

#[tauri::command]
pub fn ops_task_list(
    state: State<'_, AppState>,
    conversation: Option<String>,
) -> Result<Vec<OpsTask>, CommandError> {
    Ok(state
        .ops
        .list()
        .map_err(ops_runtime::error)?
        .into_iter()
        .filter(|task| {
            conversation
                .as_ref()
                .is_none_or(|id| id == &task.conversation)
        })
        .collect())
}

#[tauri::command]
pub fn ops_task_get(state: State<'_, AppState>, id: OpsTaskId) -> Result<OpsTask, CommandError> {
    state.ops.get(id).map_err(ops_runtime::error)
}

#[tauri::command]
pub async fn ops_task_input(
    app: AppHandle,
    id: OpsTaskId,
    text: String,
    host_ids: Vec<HostId>,
    resolve_uncertain: bool,
) -> Result<OpsTask, CommandError> {
    ops_runtime::append_input(&app, id, &text, host_ids, resolve_uncertain).await
}

#[tauri::command]
pub fn ops_task_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: OpsTaskId,
) -> Result<OpsTask, CommandError> {
    let task = state.ops.cancel(id).map_err(ops_runtime::error)?;
    use tauri::Emitter;
    if let Err(error) = app.emit("ops-update", &task) {
        tracing::debug!(%error, "operations cancellation update unavailable");
    }
    Ok(task)
}

#[tauri::command]
pub fn ops_task_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    id: OpsTaskId,
) -> Result<OpsTask, CommandError> {
    let task = state.ops.retry_failed(id).map_err(ops_runtime::error)?;
    ops_runtime::kick(&app, id);
    Ok(task)
}

#[tauri::command]
pub async fn ops_task_evidence(
    app: AppHandle,
    id: OpsTaskId,
    reference: PrivateRef,
) -> Result<Value, CommandError> {
    let state = app.state::<AppState>();
    let task = state.ops.get(id).map_err(ops_runtime::error)?;
    if !ops_runtime::reference_belongs(&task, &reference) {
        return Err(CommandError::new(
            "ops.scope",
            "the reference does not belong to this task",
        ));
    }
    state.ensure_unlocked().await?;
    ops_runtime::read_private(&state, &reference)
}
