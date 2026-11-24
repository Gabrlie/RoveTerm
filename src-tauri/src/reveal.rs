//! Opening files and URLs in system applications.

/// Opens the file manager: a folder is opened, a file is shown selected in its folder where the
/// platform can (Linux opens its folder).
pub fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    let folder = path.is_dir();
    let mut command;
    #[cfg(target_os = "macos")]
    {
        command = std::process::Command::new("open");
        if !folder {
            command.arg("-R");
        }
        command.arg(path);
    }
    #[cfg(target_os = "windows")]
    {
        command = std::process::Command::new("explorer");
        if folder {
            command.arg(path);
        } else {
            let mut select = std::ffi::OsString::from("/select,");
            select.push(path);
            command.arg(select);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        command = std::process::Command::new("xdg-open");
        command.arg(if folder {
            path
        } else {
            path.parent().unwrap_or(path)
        });
    }
    // The file manager runs on its own; its exit is collected off this thread.
    let mut child = command.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(desktop)]
pub fn share_or_reveal(
    _app: &tauri::AppHandle,
    path: &std::path::Path,
    _title: Option<&str>,
) -> Result<(), String> {
    reveal(path).map_err(|error| error.to_string())
}

#[cfg(mobile)]
pub fn share_or_reveal(
    app: &tauri::AppHandle,
    path: &std::path::Path,
    title: Option<&str>,
) -> Result<(), String> {
    use tauri_plugin_rove_mobile::MobileBridgeExt;

    let path = path.to_str().ok_or("file path must be UTF-8")?;
    app.rove_mobile()
        .share_file(path, title)
        .map_err(|error| error.to_string())
}

/// Opens a validated HTTP or HTTPS URL in the system browser.
#[cfg(desktop)]
pub fn open_url(_app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");

    let mut child = command
        .arg(url)
        .spawn()
        .map_err(|error| error.to_string())?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(mobile)]
pub fn open_url(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    use tauri_plugin_rove_mobile::MobileBridgeExt;

    app.rove_mobile()
        .open_external_url(url)
        .map_err(|error| error.to_string())
}
