//! User preferences and the fonts they can refer to.

use rove_vault::{DevicePrefs, Preferences};
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;

/// Returns the stored preferences, or the defaults.
#[tauri::command]
pub fn prefs_get(state: State<'_, AppState>) -> Result<Preferences, CommandError> {
    Ok(state.vault.preferences()?)
}

/// Validates and stores `prefs`; returns the normalized preferences.
#[tauri::command]
pub fn prefs_set(
    state: State<'_, AppState>,
    prefs: Preferences,
) -> Result<Preferences, CommandError> {
    Ok(state.vault.set_preferences(prefs)?)
}

/// Returns this device's preferences, or the defaults.
#[tauri::command]
pub fn device_prefs_get(state: State<'_, AppState>) -> Result<DevicePrefs, CommandError> {
    Ok(state.vault.device_preferences()?)
}

/// Validates and stores this device's preferences and applies the transfer limit; returns the
/// normalized preferences. Async: lowering the transfer limit spawns onto the runtime.
#[tauri::command]
pub async fn device_prefs_set(
    state: State<'_, AppState>,
    prefs: DevicePrefs,
) -> Result<DevicePrefs, CommandError> {
    let prefs = state.vault.set_device_preferences(prefs)?;
    state
        .transfers
        .set_max_concurrent(usize::from(prefs.max_transfers));
    Ok(prefs)
}

/// Lists the families of the monospaced fonts installed on this device, sorted and without
/// duplicates. Empty where fonts cannot be enumerated (mobile).
#[tauri::command]
pub async fn fonts_monospace() -> Result<Vec<String>, CommandError> {
    tauri::async_runtime::spawn_blocking(monospace_families)
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))
}

#[cfg(desktop)]
fn monospace_families() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut families: Vec<String> = db
        .faces()
        .filter(|face| face.monospaced)
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    families
}

#[cfg(mobile)]
fn monospace_families() -> Vec<String> {
    Vec::new()
}
