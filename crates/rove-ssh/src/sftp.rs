//! SFTP as a [`FileSystem`], on a channel of a shared [`SshConnection`](crate::SshConnection).

use async_trait::async_trait;
use rove_fs::{DirEntry, EntryKind, FileSystem, FsError, FsReader, FsWriter, Metadata};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, StatusCode};

use crate::connection::ChannelLease;

/// An SFTP session. Keeps its connection open until dropped.
pub struct SftpFs {
    session: SftpSession,
    label: String,
    _lease: ChannelLease,
}

impl SftpFs {
    pub(crate) fn new(session: SftpSession, label: String, lease: ChannelLease) -> Self {
        Self {
            session,
            label,
            _lease: lease,
        }
    }
}

fn fs_error(error: SftpError, path: &str) -> FsError {
    match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => FsError::NotFound(path.to_owned()),
            StatusCode::PermissionDenied => FsError::PermissionDenied(path.to_owned()),
            StatusCode::OpUnsupported => FsError::Unsupported(path.to_owned()),
            StatusCode::NoConnection | StatusCode::ConnectionLost => FsError::Closed,
            _ => FsError::Io(format!("{path}: {}", status.error_message)),
        },
        SftpError::Timeout => FsError::Io(format!("{path}: request timed out")),
        other => FsError::Io(format!("{path}: {other}")),
    }
}

fn metadata(attrs: &FileAttributes) -> Metadata {
    let file_type = attrs.file_type();
    let kind = if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Metadata {
        kind,
        size: if kind == EntryKind::Dir {
            0
        } else {
            attrs.size.unwrap_or(0)
        },
        modified: attrs.mtime.map(i64::from),
        permissions: attrs.permissions.map(|mode| mode & 0o7777),
    }
}

#[async_trait]
impl FileSystem for SftpFs {
    fn label(&self) -> String {
        self.label.clone()
    }

    async fn home(&self) -> Result<String, FsError> {
        self.session
            .canonicalize(".")
            .await
            .map_err(|error| fs_error(error, "."))
    }

    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let entries = self
            .session
            .read_dir(path)
            .await
            .map_err(|error| fs_error(error, path))?;
        Ok(entries
            .filter(|entry| {
                let name = entry.file_name();
                name != "." && name != ".."
            })
            .map(|entry| DirEntry {
                name: entry.file_name(),
                metadata: metadata(&entry.metadata()),
            })
            .collect())
    }

    async fn stat(&self, path: &str) -> Result<Metadata, FsError> {
        self.session
            .metadata(path)
            .await
            .map(|attrs| metadata(&attrs))
            .map_err(|error| fs_error(error, path))
    }

    async fn lstat(&self, path: &str) -> Result<Metadata, FsError> {
        self.session
            .symlink_metadata(path)
            .await
            .map(|attrs| metadata(&attrs))
            .map_err(|error| fs_error(error, path))
    }

    async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), FsError> {
        let attrs = FileAttributes {
            permissions: Some(mode & 0o7777),
            ..FileAttributes::empty()
        };
        self.session
            .set_metadata(path, attrs)
            .await
            .map_err(|error| fs_error(error, path))
    }

    async fn mkdir(&self, path: &str) -> Result<(), FsError> {
        self.session
            .create_dir(path)
            .await
            .map_err(|error| fs_error(error, path))
    }

    async fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.session
            .remove_file(path)
            .await
            .map_err(|error| fs_error(error, path))
    }

    async fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        self.session
            .remove_dir(path)
            .await
            .map_err(|error| fs_error(error, path))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        self.session
            .rename(from, to)
            .await
            .map_err(|error| fs_error(error, from))
    }

    async fn open_read(&self, path: &str) -> Result<FsReader, FsError> {
        let file = self
            .session
            .open(path)
            .await
            .map_err(|error| fs_error(error, path))?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, path: &str) -> Result<FsWriter, FsError> {
        let file = self
            .session
            .create(path)
            .await
            .map_err(|error| fs_error(error, path))?;
        Ok(Box::new(file))
    }
}
