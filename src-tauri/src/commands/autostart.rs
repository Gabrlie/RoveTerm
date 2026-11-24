//! Starting RoveTerm when the user logs in, so scheduled tasks run without opening it by hand.
//! The operating system's login items are the record: the setting is per device by nature.

use tauri::AppHandle;

use crate::error::CommandError;

/// The argument a login start passes: the window starts minimized.
#[cfg(desktop)]
pub const MINIMIZED_ARG: &str = "--minimized";

/// Whether RoveTerm starts when the user logs in; `None` where it cannot (mobile).
#[tauri::command]
pub fn autostart_get(app: AppHandle) -> Result<Option<bool>, CommandError> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch()
            .is_enabled()
            .map(Some)
            .map_err(|error| CommandError::new("app.autostart", error.to_string()))
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(None)
    }
}

/// Starts RoveTerm (minimized) when the user logs in, or stops doing so.
#[tauri::command]
pub fn autostart_set(app: AppHandle, enabled: bool) -> Result<(), CommandError> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        let result = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
        result.map_err(|error| CommandError::new("app.autostart", error.to_string()))
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, enabled);
        Err(CommandError::new(
            "app.autostart",
            "starting at login is not available on this device",
        ))
    }
}

/// Minimizes the main window when this start came from logging in.
pub fn apply(app: &tauri::App) {
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if !std::env::args().any(|arg| arg == MINIMIZED_ARG) {
            return;
        }
        if let Some(window) = app.get_webview_window("main")
            && let Err(error) = window.minimize()
        {
            tracing::warn!(%error, "minimizing the window at login failed");
        }
    }
    #[cfg(not(desktop))]
    let _ = app;
}
