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
pub async fn vault_unlock(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<LockState, CommandError> {
    #[cfg(desktop)]
    let was_unlocked = state.vault.lock_state() == LockState::Unlocked;
    let vault = state.vault.clone();
    let lock = tauri::async_runtime::spawn_blocking(move || vault.unlock())
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))?;
    #[cfg(desktop)]
    if !was_unlocked && lock == LockState::Unlocked {
        tauri::async_runtime::spawn(async move {
            crate::commands::tailscale::auto_connect(&app).await;
        });
    }
    #[cfg(mobile)]
    drop(app);
    Ok(lock)
}

/// Discards every stored secret and unlocks with a new device key. Sync then restores the
/// secrets from the space rather than deleting them on the other devices.
#[tauri::command]
pub async fn vault_reset_secrets(state: State<'_, AppState>) -> Result<LockState, CommandError> {
    let lock = {
        #[cfg(desktop)]
        let _guard = state.tailscale_lock.lock().await;
        let lock = state.vault.reset_secrets()?;
        #[cfg(desktop)]
        {
            let mut prefs = state.vault.device_preferences()?;
            if prefs.tailscale.configured {
                prefs.tailscale.configured = false;
                state.vault.set_device_preferences(prefs)?;
            }
            state.tailscale.disconnect(false).await?;
        }
        lock
    };
    crate::commands::sync::after_secrets_reset(state.inner()).await?;
    Ok(lock)
}
