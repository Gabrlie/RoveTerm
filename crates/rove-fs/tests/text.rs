use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use rove_fs::{
    DirEntry, FileSystem, FsError, FsReader, FsWriter, LocalFs, Metadata, TEXT_FILE_LIMIT,
    TextEditError, read_text_file, save_text_file, text_version, validate_text,
};
use tokio::io::AsyncWriteExt;

#[test]
fn text_validation_preserves_bytes_and_checks_the_exact_limit() {
    let text = "\u{feff}中文\r\nline\n";
    assert_eq!(validate_text(text.as_bytes()).unwrap(), text);
    assert!(validate_text(&vec![b'x'; TEXT_FILE_LIMIT]).is_ok());
    assert_eq!(
        validate_text(&vec![b'x'; TEXT_FILE_LIMIT + 1]),
        Err(TextEditError::TooLarge)
    );
    assert_eq!(validate_text(b"a\0b"), Err(TextEditError::NotText));
    assert_eq!(validate_text(&[0xff]), Err(TextEditError::NotText));
    assert_eq!(
        text_version(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[tokio::test]
async fn saves_complete_text_and_checks_versions_without_touching_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.conf");
    std::fs::write(&path, "\u{feff}old\r\n").unwrap();
    let path = path.to_str().unwrap();
    let file = read_text_file(&LocalFs, path).await.unwrap();
    assert_eq!(file.text, "\u{feff}old\r\n");
    let text = "\u{feff}new 中文\r\n";
    let version = save_text_file(&LocalFs, path, &file.version, text)
        .await
        .unwrap();
    assert_eq!(version, text_version(text.as_bytes()));
    assert_eq!(std::fs::read(path).unwrap(), text.as_bytes());
    assert_eq!(
        save_text_file(&LocalFs, path, &file.version, "overwritten").await,
        Err(TextEditError::Conflict)
    );
    assert_eq!(std::fs::read(path).unwrap(), text.as_bytes());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(
        save_text_file(&LocalFs, path, "*", "x").await,
        Err(TextEditError::InvalidVersion)
    );
    assert_eq!(
        save_text_file(&LocalFs, path, &version, "").await.unwrap(),
        text_version(b"")
    );
}

#[tokio::test]
async fn rejects_relative_paths_directories_binary_and_oversized_files() {
    assert!(matches!(
        read_text_file(&LocalFs, "relative").await,
        Err(TextEditError::Fs(FsError::InvalidPath(_)))
    ));
    assert!(matches!(
        read_text_file(&LocalFs, "/bad\0path").await,
        Err(TextEditError::Fs(FsError::InvalidPath(_)))
    ));
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        read_text_file(&LocalFs, dir.path().to_str().unwrap()).await,
        Err(TextEditError::NotRegularFile)
    );
    for (bytes, expected) in [
        (b"a\0b".to_vec(), TextEditError::NotText),
        (vec![0xff], TextEditError::NotText),
        (vec![b'x'; TEXT_FILE_LIMIT + 1], TextEditError::TooLarge),
    ] {
        let path = dir.path().join("bad");
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            read_text_file(&LocalFs, path.to_str().unwrap()).await,
            Err(expected)
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlinks_and_preserves_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.conf");
    let link = dir.path().join("link.conf");
    std::fs::write(&path, b"old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&path, &link).unwrap();
    assert_eq!(
        read_text_file(&LocalFs, link.to_str().unwrap()).await,
        Err(TextEditError::NotRegularFile)
    );
    assert_eq!(
        save_text_file(
            &LocalFs,
            link.to_str().unwrap(),
            &text_version(b"old"),
            "new"
        )
        .await,
        Err(TextEditError::NotRegularFile)
    );
    let file = read_text_file(&LocalFs, path.to_str().unwrap())
        .await
        .unwrap();
    save_text_file(&LocalFs, path.to_str().unwrap(), &file.version, "new")
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
}

#[tokio::test]
async fn private_creation_is_exclusive_and_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private");
    let mut writer = LocalFs
        .create_new_private(path.to_str().unwrap())
        .await
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    writer.write_all(b"keep").await.unwrap();
    writer.shutdown().await.unwrap();
    assert!(matches!(
        LocalFs.create_new_private(path.to_str().unwrap()).await,
        Err(FsError::AlreadyExists(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
}

struct RejectWrites;

impl tokio::io::AsyncWrite for RejectWrites {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::task::Poll::Ready(Err(std::io::Error::other("write failed")))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn bounds_the_read_even_when_metadata_understates_the_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, vec![b'x'; TEXT_FILE_LIMIT + 1]).unwrap();
    let fs = ControlledFs {
        path: path.to_string_lossy().into_owned(),
        behavior: Behavior::UnderstateSize,
        changed: AtomicBool::new(false),
    };
    assert_eq!(
        read_text_file(&fs, &fs.path).await,
        Err(TextEditError::TooLarge)
    );
}

#[derive(Clone, Copy)]
enum Behavior {
    Mutate,
    Unsupported,
    RefuseOverwrite,
    FailInstall,
    FailWrite,
    UnderstateSize,
}

struct ControlledFs {
    path: String,
    behavior: Behavior,
    changed: AtomicBool,
}

#[async_trait]
impl FileSystem for ControlledFs {
    fn label(&self) -> String {
        "controlled".into()
    }
    async fn home(&self) -> Result<String, FsError> {
        LocalFs.home().await
    }
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        LocalFs.list(path).await
    }
    async fn stat(&self, path: &str) -> Result<Metadata, FsError> {
        LocalFs.stat(path).await
    }
    async fn lstat(&self, path: &str) -> Result<Metadata, FsError> {
        let mut metadata = LocalFs.lstat(path).await?;
        if matches!(self.behavior, Behavior::UnderstateSize) {
            metadata.size = 0;
        }
        Ok(metadata)
    }
    async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), FsError> {
        LocalFs.set_permissions(path, mode).await
    }
    async fn mkdir(&self, path: &str) -> Result<(), FsError> {
        LocalFs.mkdir(path).await
    }
    async fn remove_file(&self, path: &str) -> Result<(), FsError> {
        LocalFs.remove_file(path).await
    }
    async fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        LocalFs.remove_dir(path).await
    }
    async fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        if from.ends_with(".part") && to == self.path {
            if matches!(self.behavior, Behavior::FailInstall) {
                return Err(FsError::Io("install failed".into()));
            }
            if matches!(self.behavior, Behavior::RefuseOverwrite)
                && std::path::Path::new(to).exists()
            {
                return Err(FsError::AlreadyExists(to.into()));
            }
        }
        LocalFs.rename(from, to).await
    }
    async fn open_read(&self, path: &str) -> Result<FsReader, FsError> {
        LocalFs.open_read(path).await
    }
    async fn open_write(&self, path: &str) -> Result<FsWriter, FsError> {
        LocalFs.open_write(path).await
    }
    async fn create_new_private(&self, path: &str) -> Result<FsWriter, FsError> {
        if matches!(self.behavior, Behavior::Unsupported) {
            return Err(FsError::Unsupported("no exclusive create".into()));
        }
        if matches!(self.behavior, Behavior::Mutate) && !self.changed.swap(true, Ordering::SeqCst) {
            std::fs::write(&self.path, b"someone else's edit").unwrap();
        }
        let writer = LocalFs.create_new_private(path).await?;
        if matches!(self.behavior, Behavior::FailWrite) {
            drop(writer);
            return Ok(Box::new(RejectWrites));
        }
        Ok(writer)
    }
    fn join(&self, dir: &str, name: &str) -> String {
        LocalFs.join(dir, name)
    }
    fn parent(&self, path: &str) -> Option<String> {
        LocalFs.parent(path)
    }
}

#[tokio::test]
async fn staging_detects_new_changes_and_removes_its_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, b"old").unwrap();
    let fs = ControlledFs {
        path: path.to_string_lossy().into_owned(),
        behavior: Behavior::Mutate,
        changed: AtomicBool::new(false),
    };
    assert_eq!(
        save_text_file(&fs, &fs.path, &text_version(b"old"), "mine").await,
        Err(TextEditError::Conflict)
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"someone else's edit");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn failed_private_creation_and_installation_retain_original() {
    for behavior in [
        Behavior::Unsupported,
        Behavior::FailInstall,
        Behavior::FailWrite,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, b"old").unwrap();
        let fs = ControlledFs {
            path: path.to_string_lossy().into_owned(),
            behavior,
            changed: AtomicBool::new(false),
        };
        let result = save_text_file(&fs, &fs.path, &text_version(b"old"), "mine").await;
        if matches!(behavior, Behavior::Unsupported) {
            assert_eq!(result, Err(TextEditError::UnsupportedCreate));
        } else {
            assert!(result.is_err());
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[tokio::test]
async fn rename_without_overwrite_support_moves_original_aside_before_installing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, b"old").unwrap();
    let fs = ControlledFs {
        path: path.to_string_lossy().into_owned(),
        behavior: Behavior::RefuseOverwrite,
        changed: AtomicBool::new(false),
    };
    assert_eq!(
        save_text_file(&fs, &fs.path, &text_version(b"old"), "mine")
            .await
            .unwrap(),
        text_version(b"mine")
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
