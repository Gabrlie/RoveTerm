//! Showing a file or folder in the system file manager.

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
