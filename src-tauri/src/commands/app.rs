//! Application log and mobile window commands.

use crate::error::CommandError;
use tauri_plugin_rove_mobile::{MobileBridgeExt, MobileInsets};

/// Returns the latest system bar and keyboard insets.
#[tauri::command]
pub fn mobile_insets(app: tauri::AppHandle) -> Result<MobileInsets, CommandError> {
    app.rove_mobile()
        .get_insets()
        .map_err(|error| CommandError::new("app.io", error.to_string()))
}

/// Performs the native back action when the app has no screen to pop.
#[tauri::command]
pub fn mobile_back_default(app: tauri::AppHandle) -> Result<(), CommandError> {
    app.rove_mobile()
        .move_to_background()
        .map_err(|error| CommandError::new("app.io", error.to_string()))
}

/// Longest message taken from the interface.
const MAX_MESSAGE: usize = 4096;
/// Most messages taken from the interface per minute; the rest are dropped.
const PER_MINUTE: u32 = 60;

static WINDOW: std::sync::Mutex<(Option<std::time::Instant>, u32)> =
    std::sync::Mutex::new((None, 0));

/// Writes a warning or error of the interface (`level` `warn` or `error`) to the log, at most
/// [`MAX_MESSAGE`] bytes and [`PER_MINUTE`] a minute.
#[tauri::command]
pub fn log_write(level: String, message: String) -> Result<(), CommandError> {
    {
        let mut window = WINDOW
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = std::time::Instant::now();
        if window
            .0
            .is_none_or(|start| now.duration_since(start).as_secs() >= 60)
        {
            *window = (Some(now), 0);
        }
        if window.1 >= PER_MINUTE {
            return Ok(());
        }
        window.1 += 1;
    }
    let mut end = message.len().min(MAX_MESSAGE);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    let message = &message[..end];
    match level.as_str() {
        "error" => tracing::error!(target: "ui", "{message}"),
        "warn" => tracing::warn!(target: "ui", "{message}"),
        _ => {
            return Err(CommandError::new(
                "input.invalid",
                "the level must be warn or error",
            ));
        }
    }
    Ok(())
}

/// Opens the log folder on desktop or shares the latest log on mobile.
#[tauri::command(async)]
pub fn logs_reveal(app: tauri::AppHandle) -> Result<(), CommandError> {
    let dir = crate::logs::dir()
        .ok_or_else(|| CommandError::new("app.io", "the log is not written to files"))?;
    #[cfg(desktop)]
    {
        let _ = app;
        crate::reveal::reveal(dir).map_err(|error| {
            CommandError::new(
                "app.io",
                format!("{error}. The log files are in {}", dir.display()),
            )
        })
    }
    #[cfg(mobile)]
    {
        crate::logs::flush();
        let newest = crate::logs::list_files_in(dir)
            .map_err(|error| CommandError::new("app.io", error.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| CommandError::new("app.io", "no log files found"))?;
        let path = dir.join(&newest.name);
        let path = path.as_path();
        #[cfg(target_os = "android")]
        let staged = {
            use tauri::Manager;

            app.state::<crate::state::AppState>()
                .mobile_files
                .stage_share_source(path, dir)
                .map_err(|error| CommandError::new("app.io", error.message))?
        };
        #[cfg(target_os = "android")]
        let path = staged.path();
        crate::reveal::share_or_reveal(&app, path, Some(&newest.name))
            .map_err(|error| CommandError::new("app.io", error))?;
        #[cfg(target_os = "android")]
        staged.keep_until_sweep();
        Ok(())
    }
}

/// Lists available daily log files in the application log directory, newest first.
#[tauri::command]
pub fn logs_list() -> Result<Vec<crate::logs::LogFileInfo>, CommandError> {
    let dir = crate::logs::dir()
        .ok_or_else(|| CommandError::new("app.io", "the log is not written to files"))?;
    crate::logs::flush();
    crate::logs::list_files_in(dir).map_err(|error| CommandError::new("app.io", error.to_string()))
}

/// Reads the trailing lines of a log file (or the newest one when `file` is omitted).
#[tauri::command]
pub fn logs_read(
    file: Option<String>,
    tail_lines: Option<usize>,
) -> Result<crate::logs::LogReadResult, CommandError> {
    let dir = crate::logs::dir()
        .ok_or_else(|| CommandError::new("app.io", "the log is not written to files"))?;
    crate::logs::flush();
    crate::logs::read_tail_in(dir, file.as_deref(), tail_lines).map_err(|error| match error {
        crate::logs::LogReadError::InvalidName => {
            CommandError::new("input.invalid", "invalid log file name")
        }
        crate::logs::LogReadError::NotFound => CommandError::new("app.io", "log file not found"),
        crate::logs::LogReadError::Io(io) => CommandError::new("app.io", io.to_string()),
    })
}
