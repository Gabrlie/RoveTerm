//! SFTP subsystem backed by the real file system. Relative paths resolve against `root`.

use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

enum Open {
    File(tokio::fs::File),
    /// Directory entries not yet returned; `None` once listed.
    Dir(Option<Vec<File>>),
}

pub(crate) struct SftpHandler {
    root: PathBuf,
    handles: HashMap<String, Open>,
    next_handle: u64,
}

impl SftpHandler {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            handles: HashMap::new(),
            next_handle: 0,
        }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn insert(&mut self, open: Open) -> String {
        self.next_handle += 1;
        let handle = self.next_handle.to_string();
        self.handles.insert(handle.clone(), open);
        handle
    }

    fn file(&mut self, handle: &str) -> Result<&mut tokio::fs::File, StatusCode> {
        match self.handles.get_mut(handle) {
            Some(Open::File(file)) => Ok(file),
            _ => Err(StatusCode::Failure),
        }
    }
}

fn status(error: std::io::Error) -> StatusCode {
    match error.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl russh_sftp::server::Handler for SftpHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let options: std::fs::OpenOptions = pflags.into();
        let file = tokio::fs::OpenOptions::from(options)
            .open(self.resolve(&filename))
            .await
            .map_err(status)?;
        Ok(Handle {
            id,
            handle: self.insert(Open::File(file)),
        })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        match self.handles.remove(&handle) {
            Some(Open::File(mut file)) => file.flush().await.map_err(status)?,
            Some(Open::Dir(_)) => {}
            None => return Err(StatusCode::Failure),
        }
        Ok(ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let file = self.file(&handle)?;
        file.seek(SeekFrom::Start(offset)).await.map_err(status)?;
        let mut data = vec![0; len.min(256 * 1024) as usize];
        let read = file.read(&mut data).await.map_err(status)?;
        if read == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(read);
        Ok(Data { id, data })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let file = self.file(&handle)?;
        file.seek(SeekFrom::Start(offset)).await.map_err(status)?;
        file.write_all(&data).await.map_err(status)?;
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata = tokio::fs::symlink_metadata(self.resolve(&path))
            .await
            .map_err(status)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let metadata = tokio::fs::metadata(self.resolve(&path))
            .await
            .map_err(status)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let metadata = self.file(&handle)?.metadata().await.map_err(status)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        #[cfg(unix)]
        if let Some(mode) = attrs.permissions {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(
                self.resolve(&path),
                std::fs::Permissions::from_mode(mode & 0o7777),
            )
            .await
            .map_err(status)?;
        }
        #[cfg(not(unix))]
        let _ = (path, attrs);
        Ok(ok(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        _handle: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        // Accepted and ignored: clients send it to preserve times after uploads.
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let mut reader = tokio::fs::read_dir(self.resolve(&path))
            .await
            .map_err(status)?;
        let mut files = Vec::new();
        while let Some(entry) = reader.next_entry().await.map_err(status)? {
            let Ok(metadata) = tokio::fs::symlink_metadata(entry.path()).await else {
                continue;
            };
            files.push(File::new(
                entry.file_name().to_string_lossy().into_owned(),
                FileAttributes::from(&metadata),
            ));
        }
        Ok(Handle {
            id,
            handle: self.insert(Open::Dir(Some(files))),
        })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        match self.handles.get_mut(&handle) {
            Some(Open::Dir(files)) => match files.take() {
                Some(files) => Ok(Name { id, files }),
                None => Err(StatusCode::Eof),
            },
            _ => Err(StatusCode::Failure),
        }
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        tokio::fs::remove_file(self.resolve(&filename))
            .await
            .map_err(status)?;
        Ok(ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        tokio::fs::create_dir(self.resolve(&path))
            .await
            .map_err(status)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        tokio::fs::remove_dir(self.resolve(&path))
            .await
            .map_err(status)?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let resolved = self.resolve(&path);
        let canonical = tokio::fs::canonicalize(&resolved).await.unwrap_or(resolved);
        Ok(Name {
            id,
            files: vec![File::dummy(display(&canonical))],
        })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        tokio::fs::rename(self.resolve(&oldpath), self.resolve(&newpath))
            .await
            .map_err(status)?;
        Ok(ok(id))
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let target = tokio::fs::read_link(self.resolve(&path))
            .await
            .map_err(status)?;
        Ok(Name {
            id,
            files: vec![File::dummy(display(&target))],
        })
    }

    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> Result<Status, Self::Error> {
        #[cfg(unix)]
        {
            tokio::fs::symlink(self.resolve(&targetpath), self.resolve(&linkpath))
                .await
                .map_err(status)?;
            Ok(ok(id))
        }
        #[cfg(not(unix))]
        {
            let _ = (id, linkpath, targetpath);
            Err(StatusCode::OpUnsupported)
        }
    }
}
