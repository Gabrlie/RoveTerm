//! The fleet lead's scheduled tasks: how each last ran, and running one now.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::schedule;
use crate::state::AppState;

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

/// Runs scheduled task `id` now, whatever its time; `false` when it is running already.
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
    Ok(schedule::spawn_run(
        &app,
        task,
        jiff::Timestamp::now().as_millisecond(),
    ))
}
