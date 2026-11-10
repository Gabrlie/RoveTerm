//! Where a sync space lives. Backends are plain file stores: list a directory, read, write and
//! delete whole files. They are untrusted; everything written through them is sealed first.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::SyncError;

/// Largest file read back from a backend.
pub const MAX_FILE: usize = 16 * 1024 * 1024;

/// A file store holding one sync space. Paths are relative, `/`-separated, and made of
/// [`check_path`]-valid components.
#[async_trait]
pub trait SyncBackend: Send + Sync {
    /// Names of the entries directly inside `dir` (`""` for the root); empty when it is missing.
    async fn list(&self, dir: &str) -> Result<Vec<String>, SyncError>;
    /// Contents of `path`, or `None` when it does not exist.
    async fn get(&self, path: &str) -> Result<Option<Vec<u8>>, SyncError>;
    /// Writes `path` whole, creating parent directories. Readers never see a partial file.
    async fn put(&self, path: &str, data: &[u8]) -> Result<(), SyncError>;
    /// Writes `path` only when it does not exist yet, without ever replacing it; returns whether
    /// it was written. Two devices creating the same file concurrently cannot both succeed.
    async fn put_new(&self, path: &str, data: &[u8]) -> Result<bool, SyncError>;
    /// Removes `path`; succeeds when it is already gone.
    async fn delete(&self, path: &str) -> Result<(), SyncError>;
}

/// Accepts relative paths of letters, digits, `.`, `_` and `-` components (no `.` or `..`).
pub fn check_path(path: &str) -> Result<(), SyncError> {
    let valid = path.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part.len() <= 128
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    });
    if valid {
        Ok(())
    } else {
        Err(SyncError::Format(format!("invalid sync path `{path}`")))
    }
}

/// A directory on this device, for spaces kept in a synced folder (iCloud Drive, Dropbox,
/// Syncthing, a network share) and for tests.
pub struct FolderBackend {
    root: PathBuf,
}

impl FolderBackend {
    /// Uses `root` (created on first write) as the space's directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, SyncError> {
        if path.is_empty() {
            return Ok(self.root.clone());
        }
        check_path(path)?;
        Ok(path
            .split('/')
            .fold(self.root.clone(), |dir, part| dir.join(part)))
    }
}

fn io(error: std::io::Error) -> SyncError {
    SyncError::Backend(error.to_string())
}

async fn exists(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok()
}

#[async_trait]
impl SyncBackend for FolderBackend {
    async fn list(&self, dir: &str) -> Result<Vec<String>, SyncError> {
        let dir = self.resolve(dir)?;
        if !exists(&dir).await {
            return Ok(Vec::new());
        }
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(io)?;
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(io)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Temporary files of an interrupted write.
            if !name.ends_with(".tmp") {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    async fn get(&self, path: &str) -> Result<Option<Vec<u8>>, SyncError> {
        let path = self.resolve(path)?;
        match tokio::fs::metadata(&path).await {
            Ok(metadata) if metadata.len() > MAX_FILE as u64 => {
                return Err(SyncError::Format("sync file too large".into()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io(error)),
        }
        tokio::fs::read(&path).await.map(Some).map_err(io)
    }

    async fn put(&self, path: &str, data: &[u8]) -> Result<(), SyncError> {
        let target = self.resolve(path)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let temp = target.with_extension(format!("{}.tmp", uuid::Uuid::now_v7().simple()));
        tokio::fs::write(&temp, data).await.map_err(io)?;
        if let Err(error) = tokio::fs::rename(&temp, &target).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(io(error));
        }
        Ok(())
    }

    async fn put_new(&self, path: &str, data: &[u8]) -> Result<bool, SyncError> {
        let target = self.resolve(path)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        // Written aside, then linked into place: the link fails when the file exists, and a
        // reader never sees it half written.
        let temp = target.with_extension(format!("{}.tmp", uuid::Uuid::now_v7().simple()));
        tokio::fs::write(&temp, data).await.map_err(io)?;
        let linked = tokio::fs::hard_link(&temp, &target).await;
        let _ = tokio::fs::remove_file(&temp).await;
        match linked {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            // File systems without hard links: create the file exclusively instead.
            Err(_) => {
                use tokio::io::AsyncWriteExt;
                match tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .await
                {
                    Ok(mut file) => {
                        file.write_all(data).await.map_err(io)?;
                        file.sync_all().await.map_err(io)?;
                        Ok(true)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
                    Err(error) => Err(io(error)),
                }
            }
        }
    }

    async fn delete(&self, path: &str) -> Result<(), SyncError> {
        match tokio::fs::remove_file(self.resolve(path)?).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escaping_and_odd_paths() {
        for bad in ["../x", "a/../b", "/abs", "a//b", "a b", "a\\b", ""] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        assert!(check_path("devices/0190a0b0-0000/0000000000000001.seg").is_ok());
    }

    #[tokio::test]
    async fn folder_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let backend = FolderBackend::new(dir.path().join("space"));
        assert!(backend.list("devices").await.unwrap().is_empty());
        assert_eq!(backend.get("space.json").await.unwrap(), None);
        backend.put("devices/a/1.seg", b"one").await.unwrap();
        backend.put("devices/a/2.seg", b"two").await.unwrap();
        assert_eq!(backend.list("devices").await.unwrap(), ["a"]);
        assert_eq!(backend.list("devices/a").await.unwrap(), ["1.seg", "2.seg"]);
        assert_eq!(
            backend.get("devices/a/2.seg").await.unwrap().unwrap(),
            b"two"
        );
        backend.delete("devices/a/1.seg").await.unwrap();
        backend.delete("devices/a/1.seg").await.unwrap();
        assert_eq!(backend.list("devices/a").await.unwrap(), ["2.seg"]);
        assert!(backend.get("../escape").await.is_err());
    }
}
