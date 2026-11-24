//! Vault lock state commands.

use rove_vault::{DevicePrefs, LockState};
use serde::Serialize;
use tauri::{AppHandle, State};

#[cfg(mobile)]
use {
    crate::mobile_key_cache::MobileKeyCache,
    rove_vault::{KeyCacheError, LockReason, Vault, VaultError},
    std::sync::Arc,
    tauri::{Emitter, Manager},
    tauri_plugin_rove_mobile::{BiometricStatus, MobileBridgeExt, MobileError},
};

use crate::error::CommandError;
use crate::state::AppState;

/// Returns the current lock state.
#[tauri::command]
pub fn vault_status(state: State<'_, AppState>) -> LockState {
    state.vault.lock_state()
}

#[derive(Serialize)]
pub struct VaultBiometricStatus {
    pub supported: bool,
    pub enrolled: bool,
    pub enabled: bool,
}

/// Returns available device authentication and the saved vault policy.
#[tauri::command]
pub async fn vault_biometric_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<VaultBiometricStatus, CommandError> {
    #[cfg(desktop)]
    {
        let _ = (app, state);
        Ok(VaultBiometricStatus {
            supported: false,
            enrolled: false,
            enabled: false,
        })
    }
    #[cfg(mobile)]
    {
        let vault = state.vault.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let status = app
                .rove_mobile()
                .biometric_status()
                .map_err(biometric_error)?;
            Ok(VaultBiometricStatus {
                supported: status.supported,
                enrolled: status.enrolled,
                enabled: vault.device_preferences()?.vault_biometric,
            })
        })
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))?
    }
}

/// Rewraps the existing vault key before saving the device's authentication policy.
#[tauri::command]
pub async fn vault_set_biometric(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<DevicePrefs, CommandError> {
    #[cfg(desktop)]
    {
        let _ = (app, state, enabled);
        Err(biometric_unsupported())
    }
    #[cfg(mobile)]
    {
        let vault = state.vault.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let cache = app.state::<Arc<MobileKeyCache>>();
            let _change = cache.policy_change.lock();
            if enabled {
                let BiometricStatus {
                    supported,
                    enrolled,
                } = app
                    .rove_mobile()
                    .biometric_status()
                    .map_err(biometric_error)?;
                if !supported {
                    return Err(biometric_unsupported());
                }
                if !enrolled {
                    return Err(CommandError::new(
                        "vault.biometric_unavailable",
                        "set up device authentication before enabling vault protection",
                    ));
                }
            }

            let previous = cache.biometric();
            let lock = vault
                .rewrap_master_key(
                    || {
                        cache.set_biometric(enabled);
                        Ok(())
                    },
                    || cache.set_biometric(previous),
                )
                .map_err(|error| rewrap_error(&vault, error))?;
            let prefs = vault.transaction(|| {
                let mut prefs = vault.device_preferences()?;
                prefs.vault_biometric = enabled;
                vault.set_device_preferences(prefs)
            })?;
            if let Err(error) = app.emit(crate::VAULT_STATE_EVENT, lock) {
                tracing::warn!(%error, "emitting vault state failed");
            }
            Ok(prefs)
        })
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))?
    }
}

fn biometric_unsupported() -> CommandError {
    CommandError::new(
        "vault.biometric_unsupported",
        "biometric vault protection is available on mobile",
    )
}

#[cfg(mobile)]
fn biometric_error(error: MobileError) -> CommandError {
    match error {
        MobileError::Unsupported => biometric_unsupported(),
        MobileError::Denied => CommandError::new("vault.denied", error.to_string()),
        _ => CommandError::new("vault.biometric_unavailable", error.to_string()),
    }
}

#[cfg(mobile)]
fn rewrap_error(vault: &Vault, error: VaultError) -> CommandError {
    match error {
        VaultError::KeyCache(KeyCacheError::Denied) => {
            CommandError::new("vault.denied", error.to_string())
        }
        VaultError::KeyCache(KeyCacheError::Unavailable(message)) => {
            CommandError::new("vault.biometric_unavailable", message)
        }
        VaultError::Locked(_)
            if matches!(
                vault.lock_state(),
                LockState::Locked {
                    reason: LockReason::KeyLost
                }
            ) =>
        {
            CommandError::new("vault.key_lost", error.to_string())
        }
        _ => error.into(),
    }
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
