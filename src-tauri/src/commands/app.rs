//! The application's own commands: its log.

use crate::error::CommandError;

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

/// Opens the folder of the log files in the system file manager.
#[tauri::command]
pub fn logs_reveal() -> Result<(), CommandError> {
    let dir = crate::logs::dir()
        .ok_or_else(|| CommandError::new("app.io", "the log is not written to files"))?;
    // Where no file manager opens, the path still lets the user find the files.
    crate::reveal::reveal(dir).map_err(|error| {
        CommandError::new(
            "app.io",
            format!("{error}; the log files are in {}", dir.display()),
        )
    })
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
