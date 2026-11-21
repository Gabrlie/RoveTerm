//! Read-only discovery of devices visible to the installed desktop Tailscale client.

use tauri::State;

use crate::{error::CommandError, state::AppState};

#[cfg(desktop)]
#[tauri::command]
pub async fn tailscale_snapshot(
    state: State<'_, AppState>,
) -> Result<rove_tailscale::TailnetSnapshot, CommandError> {
    state
        .tailscale
        .snapshot()
        .await
        .map_err(|error| CommandError::new("tailscale.discovery", error.to_string()))
}

#[cfg(mobile)]
#[tauri::command]
pub async fn tailscale_snapshot(
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, CommandError> {
    Err(CommandError::new(
        "tailscale.unsupported",
        "desktop discovery is required",
    ))
}
