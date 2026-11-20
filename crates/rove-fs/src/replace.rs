//! Replacing a completed temporary file while retaining the original if installation fails.

use crate::{EntryKind, FileSystem, FsError};

/// Moves `temp` onto `dst`. Providers that reject an existing target first move it to `backup`,
/// restoring it if installation fails. A failed restore leaves the original at `backup` and
/// reports its location. The caller owns `temp` and must remove it after an error.
pub(crate) async fn replace_file(
    fs: &dyn FileSystem,
    temp: &str,
    dst: &str,
    backup: &str,
) -> Result<(), FsError> {
    let first = match fs.rename(temp, dst).await {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    match fs.lstat(dst).await {
        Ok(metadata) if metadata.kind != EntryKind::Dir => {}
        // A directory may have appeared between the caller's check and the rename.
        Ok(_) | Err(FsError::NotFound(_)) => return Err(first),
        Err(error) => return Err(error),
    }
    // A backup is never knowingly replaced, even if an earlier attempt left one behind.
    match fs.lstat(backup).await {
        Ok(_) => return Err(FsError::AlreadyExists(backup.to_owned())),
        Err(FsError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    fs.rename(dst, backup).await?;
    match fs.rename(temp, dst).await {
        Ok(()) => {
            if let Err(error) = fs.remove_file(backup).await {
                tracing::warn!(%error, backup, "removing the replaced file failed");
            }
            Ok(())
        }
        Err(error) => {
            if let Err(restore) = fs.rename(backup, dst).await {
                return Err(FsError::Io(format!(
                    "{error}; restoring the original failed: {restore}; original retained at {backup}"
                )));
            }
            Err(error)
        }
    }
}
