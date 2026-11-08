//! Vault lock state commands.

use rove_vault::LockState;
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;

/// Returns the current lock state.
#[tauri::command]
pub fn vault_status(state: State<'_, AppState>) -> LockState {
    state.vault.lock_state()
}

/// Unlocks on behalf of the user; the platform store may prompt.
#[tauri::command]
pub async fn vault_unlock(state: State<'_, AppState>) -> Result<LockState, CommandError> {
    let vault = state.vault.clone();
    tauri::async_runtime::spawn_blocking(move || vault.unlock())
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))
}

/// Discards every stored secret and unlocks with a new device key.
#[tauri::command]
pub fn vault_reset_secrets(state: State<'_, AppState>) -> Result<LockState, CommandError> {
    Ok(state.vault.reset_secrets()?)
}
