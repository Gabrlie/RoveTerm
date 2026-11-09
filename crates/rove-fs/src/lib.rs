//! File system seam for RoveTerm.
//!
//! Defines the [`FileSystem`] Definition, the [`LocalFs`] provider, [`FsSessions`] (open file
//! systems addressed by [`FsId`]) and the [`TransferQueue`], which copies files and directory trees
//! between any two file systems. SFTP is provided by `rove-ssh`. See `docs/subsystems/fs.md`.

mod local;
mod sessions;
mod transfer;

use std::sync::Arc;

use async_trait::async_trait;
use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

pub use local::LocalFs;
pub use sessions::{FsId, FsSessions, FsSummary};
pub use transfer::{
    Conflict, TransferConfig, TransferId, TransferQueue, TransferRequest, TransferState,
    TransferUpdate,
};

/// What a directory entry is. Symlinks are reported as such and not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symbolic link.
    Symlink,
    /// Socket, device, FIFO or anything else.
    Other,
}

/// Metadata of one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// Entry kind.
    pub kind: EntryKind,
    /// Size in bytes; 0 when the provider does not report it.
    pub size: u64,
    /// Modification time, Unix seconds.
    pub modified: Option<i64>,
    /// POSIX permission bits (`0o7777` mask).
    pub permissions: Option<u32>,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// File name without directory.
    pub name: String,
    /// Entry metadata (not following symlinks).
    #[serde(flatten)]
    pub metadata: Metadata,
}

/// Errors returned by file system operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum FsError {
    /// The path does not exist.
    #[error("no such file or directory: {0}")]
    NotFound(String),
    /// The provider refused access.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// The path already exists.
    #[error("already exists: {0}")]
    AlreadyExists(String),
    /// The provider does not support the operation.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The path or name is not acceptable.
    #[error("invalid path: {0}")]
    InvalidPath(String),
    /// The file system session has ended.
    #[error("file system closed")]
    Closed,
    /// The operation was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// Any other I/O or protocol failure.
    #[error("i/o error: {0}")]
    Io(String),
}

impl FsError {
    /// Maps a `std::io::Error` concerning `path`.
    pub fn from_io(error: &std::io::Error, path: &str) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound(path.to_owned()),
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied(path.to_owned()),
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists(path.to_owned()),
            _ => Self::Io(format!("{path}: {error}")),
        }
    }
}

/// Byte stream read from a file.
pub type FsReader = Box<dyn AsyncRead + Send + Unpin>;
/// Byte stream written to a file. Callers must `shutdown` it to flush and close the file.
pub type FsWriter = Box<dyn AsyncWrite + Send + Unpin>;

/// A browsable, readable and writable file tree. Definition of the `FileSystem` seam.
///
/// Paths are absolute strings in the provider's own syntax; build them with [`FileSystem::join`]
/// and split them with [`FileSystem::parent`], never by hand.
#[async_trait]
pub trait FileSystem: Send + Sync {
    /// Short label for display, such as `local` or `user@host:22`.
    fn label(&self) -> String;

    /// Returns the starting directory (the login or user home directory).
    async fn home(&self) -> Result<String, FsError>;

    /// Lists `path`. Order is unspecified; `.` and `..` are omitted.
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError>;

    /// Returns metadata of `path`, following symlinks.
    async fn stat(&self, path: &str) -> Result<Metadata, FsError>;

    /// Creates the directory `path`; its parent must exist.
    async fn mkdir(&self, path: &str) -> Result<(), FsError>;

    /// Removes the file or symlink `path`.
    async fn remove_file(&self, path: &str) -> Result<(), FsError>;

    /// Removes the empty directory `path`.
    async fn remove_dir(&self, path: &str) -> Result<(), FsError>;

    /// Renames `from` to `to`. Fails when `to` exists on providers that cannot replace atomically.
    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError>;

    /// Opens `path` for reading from the start.
    async fn open_read(&self, path: &str) -> Result<FsReader, FsError>;

    /// Creates or truncates `path` for writing.
    async fn open_write(&self, path: &str) -> Result<FsWriter, FsError>;

    /// Joins a directory and a single file name.
    fn join(&self, dir: &str, name: &str) -> String {
        posix_join(dir, name)
    }

    /// Returns the parent directory of `path`, or `None` at the root.
    fn parent(&self, path: &str) -> Option<String> {
        posix_parent(path)
    }
}

/// Joins POSIX paths: `("/a", "b")` → `/a/b`, `("/", "b")` → `/b`.
pub fn posix_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Returns the parent of a POSIX path: `/a/b` → `/a`, `/a` → `/`, `/` → `None`.
pub fn posix_parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".into()),
        Some(index) => Some(trimmed[..index].to_owned()),
        None => None,
    }
}

/// Checks that `name` is a single path component: non-empty, not `.`/`..`, no separators or NUL.
pub fn validate_name(name: &str) -> Result<(), FsError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || name.len() > 255;
    if bad {
        Err(FsError::InvalidPath(name.to_owned()))
    } else {
        Ok(())
    }
}

/// Removes `path` and, when it is a directory, everything below it. Symlinks are removed, not
/// followed.
pub async fn remove_all(fs: &dyn FileSystem, path: &str) -> Result<(), FsError> {
    // Depth-first with an explicit stack: (path, children already removed).
    let mut stack = vec![(path.to_owned(), false)];
    let top_is_dir = match fs.list(path).await {
        Ok(_) => true,
        Err(FsError::NotFound(_)) => return Err(FsError::NotFound(path.to_owned())),
        Err(_) => false,
    };
    if !top_is_dir {
        return fs.remove_file(path).await;
    }
    while let Some((dir, expanded)) = stack.pop() {
        if expanded {
            fs.remove_dir(&dir).await?;
            continue;
        }
        stack.push((dir.clone(), true));
        for entry in fs.list(&dir).await? {
            let child = fs.join(&dir, &entry.name);
            if entry.metadata.kind == EntryKind::Dir {
                stack.push((child, false));
            } else {
                fs.remove_file(&child).await?;
            }
        }
    }
    Ok(())
}

/// Registers [`FsSessions`], [`TransferQueue`] and a [`LocalFs`].
pub struct FsPlugin;

impl Plugin for FsPlugin {
    fn name(&self) -> &'static str {
        "fs"
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: TransferConfig = config.parse()?;
        config
            .validate()
            .map_err(|message| KernelError::plugin("fs", message))?;
        Ok(vec![
            ctx.provide(Arc::new(FsSessions::default()))?,
            ctx.provide(Arc::new(TransferQueue::new(config, ctx.events().clone())))?,
            ctx.provide(Arc::new(LocalFs))?,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_helpers() {
        assert_eq!(posix_join("/", "a"), "/a");
        assert_eq!(posix_join("/a", "b"), "/a/b");
        assert_eq!(posix_parent("/a/b"), Some("/a".into()));
        assert_eq!(posix_parent("/a/"), Some("/".into()));
        assert_eq!(posix_parent("/a"), Some("/".into()));
        assert_eq!(posix_parent("/"), None);
    }

    #[test]
    fn names_are_single_components() {
        assert!(validate_name("file.txt").is_ok());
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }
}
