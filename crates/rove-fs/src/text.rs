//! Bounded UTF-8 snapshots and optimistic, staged replacement of existing text files.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::{EntryKind, FileSystem, FsError, Metadata, replace::replace_file};

/// Maximum bytes accepted by the interactive text editor, including a UTF-8 BOM.
pub const TEXT_FILE_LIMIT: usize = 2 * 1024 * 1024;

/// Complete, unmodified text and the lowercase SHA-256 of its original bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextFile {
    pub text: String,
    pub version: String,
}

/// Failures of bounded text reads and version-checked writes. No variant includes file contents.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextEditError {
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error("the file exceeds the text editor limit of {TEXT_FILE_LIMIT} bytes")]
    TooLarge,
    #[error("the file is not UTF-8 text without NUL bytes")]
    NotText,
    #[error("only regular files can be edited")]
    NotRegularFile,
    #[error("the file changed since it was read")]
    Conflict,
    #[error("expected a lowercase SHA-256 file version")]
    InvalidVersion,
    #[error("the file system cannot create a private file exclusively")]
    UnsupportedCreate,
}

/// Accepts at most [`TEXT_FILE_LIMIT`] bytes of valid UTF-8 without NUL. A BOM and line endings
/// are retained exactly; this function neither decodes other encodings nor normalizes text.
pub fn validate_text(bytes: &[u8]) -> Result<&str, TextEditError> {
    if bytes.len() > TEXT_FILE_LIMIT {
        return Err(TextEditError::TooLarge);
    }
    if bytes.contains(&0) {
        return Err(TextEditError::NotText);
    }
    std::str::from_utf8(bytes).map_err(|_| TextEditError::NotText)
}

/// Lowercase SHA-256 of the exact bytes, for comparing local edits with a remote snapshot.
pub fn text_version(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_path(path: &str) -> Result<(), FsError> {
    if path.contains('\0') || !(path.starts_with('/') || Path::new(path).is_absolute()) {
        return Err(FsError::InvalidPath(path.to_owned()));
    }
    Ok(())
}

async fn read_snapshot(
    fs: &dyn FileSystem,
    path: &str,
) -> Result<(TextFile, Metadata), TextEditError> {
    validate_path(path)?;
    let before = fs.lstat(path).await?;
    if before.kind != EntryKind::File {
        return Err(TextEditError::NotRegularFile);
    }
    if before.size > TEXT_FILE_LIMIT as u64 {
        return Err(TextEditError::TooLarge);
    }
    let mut bytes = Vec::new();
    fs.open_read(path)
        .await?
        .take(TEXT_FILE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| FsError::from_io(&error, path))?;
    let text = validate_text(&bytes)?.to_owned();
    let after = fs.lstat(path).await?;
    if before != after {
        return Err(TextEditError::Conflict);
    }
    Ok((
        TextFile {
            text,
            version: text_version(&bytes),
        },
        after,
    ))
}

/// Reads an existing regular file, rejecting symlinks, non-UTF-8 data, NUL and oversized files.
/// The read is bounded even if metadata understates the size. A detected change during the read
/// returns [`TextEditError::Conflict`]; provider metadata cannot guarantee an atomic snapshot.
pub async fn read_text_file(fs: &dyn FileSystem, path: &str) -> Result<TextFile, TextEditError> {
    read_snapshot(fs, path).await.map(|(file, _)| file)
}

/// Replaces an existing regular file only if its current content has `expected_version`.
/// Writes the complete text to an exclusive, private file in the same directory, closes it,
/// restores the target's permission bits and checks the target again immediately before rename.
/// Errors remove the temporary file when possible and never truncate the original. A provider
/// without exclusive private creation fails closed. Callers should await completion rather than
/// abort the future: cancellation can leave a temporary file. SFTP has no compare-and-swap, so a
/// concurrent change between the last check and rename can still win or be overwritten.
pub async fn save_text_file(
    fs: &dyn FileSystem,
    path: &str,
    expected_version: &str,
    text: &str,
) -> Result<String, TextEditError> {
    validate_path(path)?;
    validate_text(text.as_bytes())?;
    if expected_version.len() != 64
        || !expected_version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(TextEditError::InvalidVersion);
    }
    let (current, metadata) = read_snapshot(fs, path).await?;
    if current.version != expected_version {
        return Err(TextEditError::Conflict);
    }
    if current.text == text {
        return Ok(current.version);
    }
    let parent = fs
        .parent(path)
        .ok_or_else(|| FsError::InvalidPath(path.to_owned()))?;
    let temp = fs.join(&parent, &format!(".rove-edit-{}.part", Uuid::now_v7()));
    let mut writer = fs
        .create_new_private(&temp)
        .await
        .map_err(|error| match error {
            FsError::Unsupported(_) => TextEditError::UnsupportedCreate,
            other => TextEditError::Fs(other),
        })?;
    let staged = async {
        writer
            .write_all(text.as_bytes())
            .await
            .map_err(|error| FsError::from_io(&error, &temp))?;
        writer
            .shutdown()
            .await
            .map_err(|error| FsError::from_io(&error, &temp))
    }
    .await;
    drop(writer);
    let result = async {
        staged?;
        if let Some(mode) = metadata.permissions {
            fs.set_permissions(&temp, mode).await?;
        }
        let (latest, latest_metadata) = match read_snapshot(fs, path).await {
            Ok(snapshot) => snapshot,
            Err(
                TextEditError::NotRegularFile
                | TextEditError::NotText
                | TextEditError::TooLarge
                | TextEditError::Fs(FsError::NotFound(_)),
            ) => return Err(TextEditError::Conflict),
            Err(error) => return Err(error),
        };
        if latest.version != expected_version || latest_metadata.permissions != metadata.permissions
        {
            return Err(TextEditError::Conflict);
        }
        replace_file(fs, &temp, path, &format!("{temp}.old")).await?;
        Ok(text_version(text.as_bytes()))
    }
    .await;
    if result.is_err()
        && let Err(error) = fs.remove_file(&temp).await
        && !matches!(error, FsError::NotFound(_))
    {
        tracing::warn!(%error, temp, "removing an edit temporary file failed");
    }
    result
}
