//! The local file system as a [`FileSystem`].

use std::path::Path;

use async_trait::async_trait;

use crate::{DirEntry, EntryKind, FileSystem, FsError, FsReader, FsWriter, Metadata};

/// This device's file system. Paths use the platform's own syntax.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFs;

fn metadata(meta: &std::fs::Metadata) -> Metadata {
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok());
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let permissions = None;
    Metadata {
        kind,
        size: if kind == EntryKind::Dir {
            0
        } else {
            meta.len()
        },
        modified,
        permissions,
    }
}

#[async_trait]
impl FileSystem for LocalFs {
    fn label(&self) -> String {
        "local".into()
    }

    async fn home(&self) -> Result<String, FsError> {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| FsError::NotFound("home directory".into()))
    }

    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let io = |error: std::io::Error| FsError::from_io(&error, path);
        let mut reader = tokio::fs::read_dir(path).await.map_err(io)?;
        let mut entries = Vec::new();
        while let Some(entry) = reader.next_entry().await.map_err(io)? {
            // Entries can vanish between listing and stat; skip them.
            let Ok(meta) = tokio::fs::symlink_metadata(entry.path()).await else {
                continue;
            };
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                metadata: metadata(&meta),
            });
        }
        Ok(entries)
    }

    async fn stat(&self, path: &str) -> Result<Metadata, FsError> {
        tokio::fs::metadata(path)
            .await
            .map(|meta| metadata(&meta))
            .map_err(|error| FsError::from_io(&error, path))
    }

    async fn lstat(&self, path: &str) -> Result<Metadata, FsError> {
        tokio::fs::symlink_metadata(path)
            .await
            .map(|meta| metadata(&meta))
            .map_err(|error| FsError::from_io(&error, path))
    }

    async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), FsError> {
        let io = |error: std::io::Error| FsError::from_io(&error, path);
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::Permissions::from_mode(mode & 0o7777)
        };
        #[cfg(not(unix))]
        let permissions = {
            let mut permissions = tokio::fs::metadata(path).await.map_err(io)?.permissions();
            permissions.set_readonly(mode & 0o200 == 0);
            permissions
        };
        tokio::fs::set_permissions(path, permissions)
            .await
            .map_err(io)
    }

    async fn mkdir(&self, path: &str) -> Result<(), FsError> {
        tokio::fs::create_dir(path)
            .await
            .map_err(|error| FsError::from_io(&error, path))
    }

    async fn remove_file(&self, path: &str) -> Result<(), FsError> {
        tokio::fs::remove_file(path)
            .await
            .map_err(|error| FsError::from_io(&error, path))
    }

    async fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        tokio::fs::remove_dir(path)
            .await
            .map_err(|error| FsError::from_io(&error, path))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        tokio::fs::rename(from, to)
            .await
            .map_err(|error| FsError::from_io(&error, from))
    }

    async fn open_read(&self, path: &str) -> Result<FsReader, FsError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|error| FsError::from_io(&error, path))?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, path: &str) -> Result<FsWriter, FsError> {
        let file = tokio::fs::File::create(path)
            .await
            .map_err(|error| FsError::from_io(&error, path))?;
        Ok(Box::new(file))
    }

    fn join(&self, dir: &str, name: &str) -> String {
        Path::new(dir).join(name).to_string_lossy().into_owned()
    }

    fn parent(&self, path: &str) -> Option<String> {
        Path::new(path)
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
    }
}
