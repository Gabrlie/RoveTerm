//! SFTP browsing and file transfer commands.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rove_fs::{
    Conflict, DirEntry, FileSystem, FsError, FsId, TextEditError, TextFile, TransferId,
    TransferRequest, TransferState, TransferUpdate, read_text_file, remove_all, save_text_file,
    validate_name,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_rove_mobile::MobileBridgeExt;

use crate::connect::{Source, with_connection};
use crate::error::CommandError;
use crate::mobile_files::{StagedUpload, absolute_local_path, mobile_error};
use crate::state::AppState;

/// Most paths accepted by one command.
const MAX_PATHS: usize = 1000;

/// Result of [`sftp_open`].
#[derive(Serialize)]
pub struct SftpOpened {
    /// File system session to pass to the other `sftp_*` commands.
    pub id: FsId,
    /// `user@host:port`.
    pub label: String,
    /// The login directory.
    pub home: String,
}

fn fs_id(id: &str) -> Result<FsId, CommandError> {
    Ok(id.parse()?)
}

/// Accepts an absolute POSIX path without NUL bytes.
fn remote_path(path: &str) -> Result<&str, CommandError> {
    if path.starts_with('/') && !path.contains('\0') {
        Ok(path)
    } else {
        Err(CommandError::new(
            "input.invalid",
            format!("not an absolute remote path: {path:?}"),
        ))
    }
}

/// Accepts an absolute local path.
fn local_path(path: &str) -> Result<&str, CommandError> {
    absolute_local_path(path)?;
    Ok(path)
}

fn check_count(paths: &[String]) -> Result<(), CommandError> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err(CommandError::new(
            "input.invalid",
            format!("expected 1..={MAX_PATHS} paths, got {}", paths.len()),
        ));
    }
    Ok(())
}

/// Opens an SFTP session to a saved host (`host_id`) or a quick-connect `address`. Credentials
/// follow the same rules as opening a terminal.
#[tauri::command]
pub async fn sftp_open(
    state: State<'_, AppState>,
    host_id: Option<String>,
    address: Option<String>,
    password: Option<String>,
    password_kind: Option<String>,
    remember: bool,
) -> Result<SftpOpened, CommandError> {
    let source = Source::from_args(host_id, address, password, password_kind, remember)?;
    let sftp = with_connection(&state, &source, None, |connection| async move {
        connection.open_sftp().await
    })
    .await?;
    let home = sftp.home().await?;
    let label = sftp.label();
    let id = state.fs_sessions.insert(Arc::new(sftp));
    Ok(SftpOpened { id, label, home })
}

/// Lists a remote directory.
#[tauri::command]
pub async fn sftp_list(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<Vec<DirEntry>, CommandError> {
    let fs = state.fs_sessions.get(fs_id(&id)?)?;
    Ok(fs.list(remote_path(&path)?).await?)
}

/// Reads a bounded UTF-8 text file and returns a version token for optimistic saves.
#[tauri::command]
pub async fn sftp_read_text(
    state: State<'_, AppState>,
    id: FsId,
    path: String,
) -> Result<TextFile, CommandError> {
    let fs = state.fs_sessions.get(id)?;
    read_text_file(fs.as_ref(), remote_path(&path)?)
        .await
        .map_err(text_edit_error)
}

/// Saves text only when the remote file still has the version that was read for editing.
#[tauri::command]
pub async fn sftp_save_text(
    state: State<'_, AppState>,
    id: FsId,
    path: String,
    version: String,
    text: String,
) -> Result<String, CommandError> {
    let fs = state.fs_sessions.get(id)?;
    save_text_file(fs.as_ref(), remote_path(&path)?, &version, &text)
        .await
        .map_err(text_edit_error)
}

fn text_edit_error(error: TextEditError) -> CommandError {
    CommandError::new(
        crate::external_edit::text_error_code(&error),
        error.to_string(),
    )
}

/// Creates the directory `name` inside `dir` and returns its path.
#[tauri::command]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    id: String,
    dir: String,
    name: String,
) -> Result<String, CommandError> {
    let fs = state.fs_sessions.get(fs_id(&id)?)?;
    validate_name(&name)?;
    let path = fs.join(remote_path(&dir)?, &name);
    fs.mkdir(&path).await?;
    Ok(path)
}

/// Renames `path` to `name` within the same directory and returns the new path.
#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    id: String,
    path: String,
    name: String,
) -> Result<String, CommandError> {
    let fs = state.fs_sessions.get(fs_id(&id)?)?;
    validate_name(&name)?;
    let from = remote_path(&path)?;
    let parent = fs
        .parent(from)
        .ok_or_else(|| CommandError::new("input.invalid", "cannot rename the root directory"))?;
    let to = fs.join(&parent, &name);
    fs.rename(from, &to).await?;
    Ok(to)
}

/// Deletes files and directory trees. Stops at the first failure.
#[tauri::command]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> Result<(), CommandError> {
    let fs = state.fs_sessions.get(fs_id(&id)?)?;
    check_count(&paths)?;
    for path in &paths {
        let path = remote_path(path)?;
        if fs.parent(path).is_none() {
            return Err(CommandError::new(
                "input.invalid",
                "refusing to delete the root directory",
            ));
        }
        remove_all(fs.as_ref(), path).await?;
    }
    Ok(())
}

/// Takes a separate handle to the same authenticated filesystem for an editor draft.
#[tauri::command]
pub fn sftp_retain(state: State<'_, AppState>, id: FsId) -> Result<FsId, CommandError> {
    let fs = state.fs_sessions.get(id)?;
    Ok(state.fs_sessions.insert(fs))
}

/// Releases this handle. Transfers and editor drafts keep their own references.
#[tauri::command]
pub fn sftp_close(state: State<'_, AppState>, id: String) -> Result<bool, CommandError> {
    Ok(state.fs_sessions.remove(fs_id(&id)?))
}

fn enqueue_all(
    state: &AppState,
    src: &Arc<dyn FileSystem>,
    paths: &[String],
    dst: &Arc<dyn FileSystem>,
    dst_dir: &str,
    conflict: Conflict,
    tag: &str,
) -> Vec<TransferId> {
    paths
        .iter()
        .map(|path| {
            state.transfers.enqueue(TransferRequest {
                src: src.clone(),
                src_path: path.clone(),
                dst: dst.clone(),
                dst_dir: dst_dir.to_owned(),
                conflict,
                tag: tag.to_owned(),
            })
        })
        .collect()
}

/// Resolves upload names and stages files selected through the native picker.
#[tauri::command]
pub async fn transfer_prepare_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Vec<StagedUpload>, CommandError> {
    check_count(&paths)?;
    state.mobile_files.prepare_upload(&app, &paths).await
}

/// Releases prepared copies that were not queued for upload.
#[tauri::command]
pub fn transfer_discard_staged(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<(), CommandError> {
    if paths.len() > MAX_PATHS {
        return Err(CommandError::new("input.invalid", "too many staged paths"));
    }
    state.mobile_files.discard_staged(&paths);
    Ok(())
}

/// Queues uploads of local files or directories into `remote_dir`.
#[tauri::command]
pub async fn transfer_upload(
    state: State<'_, AppState>,
    id: String,
    local_paths: Vec<String>,
    remote_dir: String,
    conflict: Conflict,
) -> Result<Vec<TransferId>, CommandError> {
    let remote = state.fs_sessions.get(fs_id(&id)?)?;
    check_count(&local_paths)?;
    for path in &local_paths {
        local_path(path)?;
    }
    let local: Arc<dyn FileSystem> = state.local_fs.clone();
    let ids = enqueue_all(
        &state,
        &local,
        &local_paths,
        &remote,
        remote_path(&remote_dir)?,
        conflict,
        "upload",
    );
    for (path, id) in local_paths.iter().zip(ids.iter().copied()) {
        state.mobile_files.bind_transfer(path, id);
    }
    // A fast transfer may finish and be cleared before its staging directory is bound.
    let mut finished: HashSet<_> = ids.iter().copied().collect();
    for row in state.transfers.list() {
        if !row.state.is_final() {
            finished.remove(&row.id);
        }
    }
    for id in finished {
        state.mobile_files.finish_transfer(id);
    }
    Ok(ids)
}

/// Queues downloads of remote files or directories into `local_dir`, or into the system download
/// directory when absent.
#[tauri::command]
pub async fn transfer_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    remote_paths: Vec<String>,
    local_dir: Option<String>,
    conflict: Conflict,
) -> Result<Vec<TransferId>, CommandError> {
    let remote = state.fs_sessions.get(fs_id(&id)?)?;
    check_count(&remote_paths)?;
    for path in &remote_paths {
        remote_path(path)?;
    }
    let local_dir = match local_dir {
        Some(dir) => local_path(&dir)?.to_owned(),
        None => download_dir(&app)?,
    };
    let local: Arc<dyn FileSystem> = state.local_fs.clone();
    Ok(enqueue_all(
        &state,
        &remote,
        &remote_paths,
        &local,
        &local_dir,
        conflict,
        "download",
    ))
}

/// Returns which of `names` already exist in `local_dir` (or the system download directory), so
/// the user can choose how to resolve the clash before downloading.
#[tauri::command]
pub async fn transfer_local_clashes(
    app: AppHandle,
    state: State<'_, AppState>,
    names: Vec<String>,
    local_dir: Option<String>,
) -> Result<Vec<String>, CommandError> {
    check_count(&names)?;
    let dir = match local_dir {
        Some(dir) => local_path(&dir)?.to_owned(),
        None => download_dir(&app)?,
    };
    let mut clashes = Vec::new();
    for name in names {
        rove_fs::validate_name(&name)?;
        let path = state.local_fs.join(&dir, &name);
        match state.local_fs.stat(&path).await {
            Ok(_) => clashes.push(name),
            Err(rove_fs::FsError::NotFound(_)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(clashes)
}

/// The platform download directory; falls back to an existing `~/Downloads` where the platform
/// reports none (Linux without `user-dirs.dirs`).
#[cfg(desktop)]
fn download_dir(app: &AppHandle) -> Result<String, CommandError> {
    let paths = app.path();
    let dir = match paths.download_dir() {
        Ok(dir) => dir,
        Err(error) => paths
            .home_dir()
            .map(|home| home.join("Downloads"))
            .ok()
            .filter(|dir| dir.is_dir())
            .ok_or_else(|| CommandError::new("fs.no_download_dir", error.to_string()))?,
    };
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(mobile)]
fn download_dir(app: &AppHandle) -> Result<String, CommandError> {
    let paths = app.path();
    let dir = paths
        .download_dir()
        .ok()
        .filter(|dir| std::fs::create_dir_all(dir).is_ok())
        .or_else(|| {
            app_download_dir(app)
                .ok()
                .filter(|dir| std::fs::create_dir_all(dir).is_ok())
        })
        .ok_or_else(|| {
            CommandError::new("fs.no_download_dir", "download directory is unavailable")
        })?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(mobile)]
fn app_download_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(app.path().app_data_dir()?.join("downloads"))
}

#[cfg(target_os = "android")]
fn stage_fallback_download(
    app: &AppHandle,
    mobile_files: &crate::mobile_files::MobileFiles,
    path: &Path,
) -> Result<Option<crate::mobile_files::StagedTempFile>, CommandError> {
    let Ok(dir) = app_download_dir(app) else {
        return Ok(None);
    };
    path.starts_with(&dir)
        .then(|| mobile_files.stage_share_source(path, &dir))
        .transpose()
}

/// Lists known transfers, oldest first.
#[tauri::command]
pub fn transfer_list(state: State<'_, AppState>) -> Vec<TransferUpdate> {
    state.transfers.list()
}

/// Cancels a transfer. Returns `false` when it is unknown or already finished.
#[tauri::command]
pub fn transfer_cancel(state: State<'_, AppState>, id: String) -> Result<bool, CommandError> {
    let id: TransferId = id
        .parse()
        .map_err(|error: FsError| CommandError::from(error))?;
    Ok(state.transfers.cancel(id))
}

fn finished_download(rows: Vec<TransferUpdate>, id: &str) -> Result<TransferUpdate, CommandError> {
    let id: TransferId = id.parse()?;
    rows.into_iter()
        .find(|row| {
            row.id == id
                && row.tag == "download"
                && row.state == TransferState::Done
                && !row.skipped
        })
        .ok_or_else(|| CommandError::new("fs.not_found", "no finished download with that id"))
}

/// Reveals a finished download on desktop or opens the native share sheet on mobile.
#[tauri::command(async)]
pub fn transfer_reveal(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), CommandError> {
    let row = finished_download(state.transfers.list(), &id)?;
    let path = PathBuf::from(&row.dst_path);
    if !path.exists() {
        return Err(CommandError::new(
            "fs.not_found",
            "the downloaded file is gone",
        ));
    }
    #[cfg(target_os = "android")]
    let staged = stage_fallback_download(&app, &state.mobile_files, &path)?;
    #[cfg(target_os = "android")]
    let path = staged.as_ref().map_or(path.as_path(), |file| file.path());
    #[cfg(not(target_os = "android"))]
    let path = path.as_path();
    crate::reveal::share_or_reveal(&app, path, Some(&row.name))
        .map_err(|error| CommandError::new("app.io", error))?;
    #[cfg(target_os = "android")]
    if let Some(staged) = staged {
        staged.keep_until_sweep();
    }
    Ok(())
}

/// Exports a finished file download. Returns false when the picker is cancelled.
#[tauri::command]
pub async fn transfer_export(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, CommandError> {
    let row = finished_download(state.transfers.list(), &id)?;
    #[cfg(target_os = "android")]
    let mobile_files = state.mobile_files.clone();
    tauri::async_runtime::spawn_blocking(move || {
        check_export_file(Path::new(&row.dst_path))?;
        #[cfg(target_os = "android")]
        let staged = stage_fallback_download(&app, &mobile_files, Path::new(&row.dst_path))?;
        #[cfg(target_os = "android")]
        let path = match &staged {
            Some(file) => file
                .path()
                .to_str()
                .ok_or_else(|| CommandError::new("input.invalid", "file path must be UTF-8"))?,
            None => &row.dst_path,
        };
        #[cfg(not(target_os = "android"))]
        let path = &row.dst_path;
        app.rove_mobile()
            .export_file(path, &row.name)
            .map_err(|error| mobile_error(error, "app.io"))
    })
    .await
    .map_err(|error| CommandError::new("app.task", error.to_string()))?
}

fn check_export_file(path: &Path) -> Result<(), CommandError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        let code = if error.kind() == std::io::ErrorKind::NotFound {
            "fs.not_found"
        } else {
            "app.io"
        };
        CommandError::new(code, error.to_string())
    })?;
    if !metadata.is_file() {
        return Err(CommandError::new("input.invalid", "export requires a file"));
    }
    Ok(())
}

/// Forgets finished transfers.
#[tauri::command]
pub fn transfer_clear(state: State<'_, AppState>) {
    state.transfers.clear_finished();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_validation() {
        assert!(remote_path("/srv/www").is_ok());
        assert!(remote_path("relative").is_err());
        assert!(remote_path("/a\0b").is_err());
        assert!(check_count(&[]).is_err());
        assert!(check_count(&["/a".into()]).is_ok());
    }

    #[test]
    fn export_requires_an_existing_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("download.txt");
        assert_eq!(check_export_file(&file).unwrap_err().code, "fs.not_found");
        assert_eq!(
            check_export_file(dir.path()).unwrap_err().code,
            "input.invalid"
        );
        std::fs::write(&file, b"download").unwrap();
        assert!(check_export_file(&file).is_ok());
    }
}
