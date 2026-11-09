//! SFTP browsing and file transfer commands.

use std::path::Path;
use std::sync::Arc;

use rove_fs::{
    Conflict, DirEntry, FileSystem, FsError, FsId, TransferId, TransferRequest, TransferUpdate,
    remove_all, validate_name,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::connect::{Source, with_connection};
use crate::error::CommandError;
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
    if Path::new(path).is_absolute() && !path.contains('\0') {
        Ok(path)
    } else {
        Err(CommandError::new(
            "input.invalid",
            format!("not an absolute local path: {path:?}"),
        ))
    }
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

/// Closes an SFTP session. Running transfers keep their own reference until they finish.
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
    Ok(enqueue_all(
        &state,
        &local,
        &local_paths,
        &remote,
        remote_path(&remote_dir)?,
        conflict,
        "upload",
    ))
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
}
