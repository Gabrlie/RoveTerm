//! IPC for host-scoped durable operations tasks.

use rove_ops::{OpsSubtaskId, OpsTaskId, PrivateRef};
use rove_vault::HostId;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::{error::CommandError, ops_runtime, state::AppState};
use ops_runtime::OpsTaskView;

#[tauri::command]
pub fn ops_task_list(
    state: State<'_, AppState>,
    conversation: Option<String>,
) -> Result<Vec<OpsTaskView>, CommandError> {
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
        .map(|task| ops_runtime::task_view(&state, &task))
        .collect())
}

#[tauri::command]
pub fn ops_task_get(
    state: State<'_, AppState>,
    id: OpsTaskId,
) -> Result<OpsTaskView, CommandError> {
    let task = state.ops.get(id).map_err(ops_runtime::error)?;
    Ok(ops_runtime::task_view(&state, &task))
}

#[tauri::command]
pub async fn ops_task_input(
    app: AppHandle,
    id: OpsTaskId,
    text: String,
    host_ids: Vec<HostId>,
    resolve_uncertain: bool,
) -> Result<OpsTaskView, CommandError> {
    let task = ops_runtime::append_input(&app, id, &text, host_ids, resolve_uncertain).await?;
    Ok(ops_runtime::task_view(&app.state::<AppState>(), &task))
}

#[tauri::command]
pub fn ops_task_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: OpsTaskId,
) -> Result<OpsTaskView, CommandError> {
    let task = state.ops.cancel(id).map_err(ops_runtime::error)?;
    ops_runtime::emit(&app, &task);
    Ok(ops_runtime::task_view(&state, &task))
}

#[tauri::command]
pub fn ops_subtask_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: OpsTaskId,
    subtask_id: OpsSubtaskId,
) -> Result<OpsTaskView, CommandError> {
    let task = state
        .ops
        .cancel_subtask(id, subtask_id)
        .map_err(ops_runtime::error)?;
    ops_runtime::emit(&app, &task);
    Ok(ops_runtime::task_view(&state, &task))
}

#[tauri::command]
pub async fn ops_task_retry(
    app: AppHandle,
    id: OpsTaskId,
    host_id: Option<HostId>,
    confirm_uncertain: Option<bool>,
) -> Result<OpsTaskView, CommandError> {
    ops_runtime::retry_task(&app, id, host_id, confirm_uncertain.unwrap_or_default()).await
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
