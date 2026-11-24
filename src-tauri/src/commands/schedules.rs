//! The fleet lead's scheduled tasks: how each last ran, and running one now.

use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
#[cfg(desktop)]
use crate::schedule;
use crate::state::AppState;
#[cfg(desktop)]
use tauri::AppHandle;

/// How a scheduled task last ran.
#[derive(Debug, Serialize)]
pub struct ScheduleStatus {
    pub schedule: String,
    /// The scheduled time handled last (Unix milliseconds).
    pub due: i64,
    /// `started`, `done`, `failed` or `missed`.
    pub status: String,
    /// The fleet lead conversation of that run.
    pub conversation: Option<String>,
    /// When it was recorded (Unix milliseconds).
    pub at: i64,
    pub running: bool,
}

/// How each scheduled task last ran.
#[cfg(desktop)]
#[tauri::command]
pub fn schedule_status(state: State<'_, AppState>) -> Vec<ScheduleStatus> {
    schedule::handled(state.journal.as_ref())
        .into_iter()
        .map(|(id, (due, status, conversation, at))| ScheduleStatus {
            running: schedule::is_running(&id),
            schedule: id,
            due,
            status,
            conversation,
            at,
        })
        .collect()
}

#[cfg(mobile)]
#[tauri::command]
pub fn schedule_status() -> Vec<ScheduleStatus> {
    Vec::new()
}

#[tauri::command]
pub fn schedule_device(state: State<'_, AppState>) -> Result<uuid::Uuid, CommandError> {
    Ok(state.vault.installation_id()?)
}

#[cfg(desktop)]
#[tauri::command]
pub async fn schedule_claim(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _lock = state.sync_lock.lock().await;
    let device = state.vault.installation_id()?;
    let existing = state
        .vault
        .preferences()?
        .ai
        .schedules
        .into_iter()
        .find(|t| t.id == id && t.owner.is_none())
        .ok_or_else(|| CommandError::new("schedule.claimed", "task unavailable for claiming"))?;
    super::sync::reserve_schedule(&state.vault, &existing, device).await?;
    state.vault.transaction(|| {
        let mut prefs = state.vault.preferences()?;
        let task = prefs
            .ai
            .schedules
            .iter_mut()
            .find(|t| **t == existing)
            .ok_or_else(|| rove_vault::VaultError::Invalid("task changed while claiming".into()))?;
        task.owner = Some(device);
        task.created_at = jiff::Timestamp::now().as_millisecond();
        state.vault.set_preferences(prefs)?;
        Ok(())
    })?;
    Ok(())
}

#[cfg(mobile)]
#[tauri::command]
pub fn schedule_claim() -> Result<(), CommandError> {
    Err(unsupported())
}

/// Runs scheduled task `id` now, whatever its time; `false` when it is running already.
#[cfg(desktop)]
#[tauri::command]
pub fn schedule_run_now(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    let prefs = state.vault.preferences()?;
    let task = prefs
        .ai
        .schedules
        .into_iter()
        .find(|task| task.id == id)
        .ok_or_else(|| CommandError::new("input.invalid", "no such scheduled task"))?;
    if task.owner != Some(state.vault.installation_id()?) {
        return Err(CommandError::new(
            "schedule.otherDevice",
            "run this task on its assigned device",
        ));
    }
    Ok(schedule::spawn_run(
        &app,
        task,
        jiff::Timestamp::now().as_millisecond(),
    ))
}

#[cfg(mobile)]
#[tauri::command]
pub fn schedule_run_now() -> Result<bool, CommandError> {
    Err(unsupported())
}

#[cfg(mobile)]
fn unsupported() -> CommandError {
    CommandError::new(
        "schedule.unsupported",
        "scheduled tasks are available on desktop",
    )
}
