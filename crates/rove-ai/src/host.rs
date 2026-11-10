//! Where an AI session's operations run: this machine, or a saved host over the connection the
//! app already holds. The policy has decided before anything here is called.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rove_fs::{EntryKind, FileSystem, FsError, LocalFs};
use rove_ssh::SshConnection;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// Bounds on what one operation may take or return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Output kept per stream of a command, in bytes.
    pub output: usize,
    /// Longest a command may run.
    pub timeout: Duration,
    /// Largest file read or written, in bytes.
    pub file: usize,
    /// Most entries listed from one directory.
    pub entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            output: 64 * 1024,
            timeout: Duration::from_secs(120),
            file: 1024 * 1024,
            entries: 1000,
        }
    }
}

/// Errors of host operations, reported back to the agent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum HostError {
    /// The host has no open connection; the user has to connect a terminal first.
    #[error("the host is not connected; open a terminal to it first")]
    NotConnected,
    #[error("timed out after {0} seconds")]
    Timeout(u64),
    #[error("{0} is larger than the limit of {1} bytes")]
    TooLarge(String, usize),
    #[error("{0} is not a text file")]
    Binary(String),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error("{0}")]
    Failed(String),
}

/// What a command printed and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    /// Exit code, when the command exited normally.
    pub exit_code: Option<i32>,
    /// Some output beyond the limit was dropped.
    pub truncated: bool,
}

/// A text file's contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileText {
    pub content: String,
    /// The file was longer than the limit; `content` is its start.
    pub truncated: bool,
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    /// `file`, `dir`, `symlink` or `other`.
    pub kind: &'static str,
    pub size: u64,
}

/// The host an AI session works on.
#[async_trait]
pub trait Host: Send + Sync {
    /// Shown on approval cards and recorded with every operation: `local` or `user@host:port`.
    fn label(&self) -> String;

    /// Runs `command` through the host's shell in `cwd` (the login directory when `None`).
    async fn run(&self, command: &str, cwd: Option<&str>, limits: &Limits) -> Result<CommandOutput, HostError>;

    /// The host's file system.
    async fn files(&self) -> Result<Arc<dyn FileSystem>, HostError>;
}

/// Reads the text file `path`, keeping at most `limits.file` bytes.
pub async fn read_text(host: &dyn Host, path: &str, limits: &Limits) -> Result<FileText, HostError> {
    let files = host.files().await?;
    let mut reader = files.open_read(path).await?;
    let mut bytes = Vec::new();
    (&mut reader)
        .take(limits.file as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| HostError::Failed(error.to_string()))?;
    let truncated = bytes.len() > limits.file;
    bytes.truncate(limits.file);
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return Err(HostError::Binary(path.to_owned()));
    }
    // A cut in the middle of a character loses only that character.
    let content = String::from_utf8_lossy(&bytes).into_owned();
    Ok(FileText { content, truncated })
}

/// Creates or replaces the file `path` with `content`.
pub async fn write_text(host: &dyn Host, path: &str, content: &str, limits: &Limits) -> Result<(), HostError> {
    if content.len() > limits.file {
        return Err(HostError::TooLarge(path.to_owned(), limits.file));
    }
    let files = host.files().await?;
    let mut writer = files.open_write(path).await?;
    writer
        .write_all(content.as_bytes())
        .await
        .map_err(|error| HostError::Failed(error.to_string()))?;
    writer
        .shutdown()
        .await
        .map_err(|error| HostError::Failed(error.to_string()))?;
    Ok(())
}

/// Lists `path`, sorted by name, keeping at most `limits.entries`; returns whether more were left.
pub async fn list_dir(host: &dyn Host, path: &str, limits: &Limits) -> Result<(Vec<Entry>, bool), HostError> {
    let files = host.files().await?;
    let mut entries: Vec<Entry> = files
        .list(path)
        .await?
        .into_iter()
        .map(|entry| Entry {
            kind: match entry.metadata.kind {
                EntryKind::File => "file",
                EntryKind::Dir => "dir",
                EntryKind::Symlink => "symlink",
                EntryKind::Other => "other",
            },
            size: entry.metadata.size,
            name: entry.name,
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let more = entries.len() > limits.entries;
    entries.truncate(limits.entries);
    Ok((entries, more))
}

/// Quotes `text` as one word for a POSIX shell.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// This machine.
pub struct LocalHost;

#[async_trait]
impl Host for LocalHost {
    fn label(&self) -> String {
        "local".into()
    }

    async fn run(&self, command: &str, cwd: Option<&str>, limits: &Limits) -> Result<CommandOutput, HostError> {
        #[cfg(windows)]
        let mut process = {
            let mut process = tokio::process::Command::new("cmd");
            process.arg("/C").arg(command);
            process
        };
        #[cfg(not(windows))]
        let mut process = {
            let mut process = tokio::process::Command::new("sh");
            process.arg("-c").arg(command);
            process
        };
        let dir = match cwd {
            Some(dir) => std::path::PathBuf::from(dir),
            None => std::env::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
        };
        process
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = process.spawn().map_err(|error| HostError::Failed(error.to_string()))?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            return Err(HostError::Failed("no output pipes".into()));
        };
        let limit = limits.output;
        let work = async {
            let (out, err, status) = tokio::join!(read_capped(stdout, limit), read_capped(stderr, limit), child.wait());
            let status = status.map_err(|error| HostError::Failed(error.to_string()))?;
            let ((stdout, cut_out), (stderr, cut_err)) = (out, err);
            Ok(CommandOutput {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                exit_code: status.code(),
                truncated: cut_out || cut_err,
            })
        };
        // Dropping the future on timeout drops the child, which kills it.
        tokio::time::timeout(limits.timeout, work)
            .await
            .map_err(|_| HostError::Timeout(limits.timeout.as_secs()))?
    }

    async fn files(&self) -> Result<Arc<dyn FileSystem>, HostError> {
        Ok(Arc::new(LocalFs))
    }
}

/// Reads all of `stream`, keeping the first `limit` bytes; returns whether some were dropped.
async fn read_capped(mut stream: impl tokio::io::AsyncRead + Unpin, limit: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut dropped = false;
    let mut buffer = [0u8; 8192];
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return (kept, dropped),
            Ok(read) => {
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..read.min(room)]);
                dropped |= read > room;
            }
        }
    }
}

/// Gives the live connection of a remote host, if there is one. The app implements it with its
/// connection pool; it never connects or asks for credentials.
#[async_trait]
pub trait ConnectionSource: Send + Sync {
    async fn connection(&self) -> Option<Arc<SshConnection>>;
}

/// An SFTP session and the connection it belongs to.
type OpenSftp = (Arc<SshConnection>, Arc<dyn FileSystem>);

/// A saved host, reached over the connection the app already holds.
pub struct RemoteHost {
    label: String,
    source: Arc<dyn ConnectionSource>,
    /// The SFTP session, opened on first use and reopened when its connection changes.
    sftp: Mutex<Option<OpenSftp>>,
}

impl RemoteHost {
    pub fn new(label: impl Into<String>, source: Arc<dyn ConnectionSource>) -> Self {
        Self {
            label: label.into(),
            source,
            sftp: Mutex::new(None),
        }
    }

    async fn connection(&self) -> Result<Arc<SshConnection>, HostError> {
        match self.source.connection().await {
            Some(connection) if !connection.is_closed() => Ok(connection),
            _ => Err(HostError::NotConnected),
        }
    }
}

#[async_trait]
impl Host for RemoteHost {
    fn label(&self) -> String {
        self.label.clone()
    }

    async fn run(&self, command: &str, cwd: Option<&str>, limits: &Limits) -> Result<CommandOutput, HostError> {
        let connection = self.connection().await?;
        let line = match cwd {
            Some(dir) => format!("cd {} && {command}", shell_quote(dir)),
            None => command.to_owned(),
        };
        let output = tokio::time::timeout(limits.timeout, connection.exec(&line, limits.output))
            .await
            .map_err(|_| HostError::Timeout(limits.timeout.as_secs()))?
            .map_err(|error| HostError::Failed(error.to_string()))?;
        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.code,
            truncated: output.truncated,
        })
    }

    async fn files(&self) -> Result<Arc<dyn FileSystem>, HostError> {
        let connection = self.connection().await?;
        let mut cached = self.sftp.lock().await;
        if let Some((owner, files)) = cached.as_ref()
            && Arc::ptr_eq(owner, &connection)
        {
            return Ok(files.clone());
        }
        let files: Arc<dyn FileSystem> = Arc::new(
            connection
                .open_sftp()
                .await
                .map_err(|error| HostError::Failed(error.to_string()))?,
        );
        *cached = Some((connection, files.clone()));
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_for_the_shell() {
        assert_eq!(shell_quote("/srv/it's here"), r"'/srv/it'\''s here'");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_commands_capture_output_exit_and_limits() {
        let dir = tempfile::tempdir().unwrap();
        let limits = Limits::default();
        let output = LocalHost
            .run("echo out; echo err >&2; pwd; exit 3", dir.path().to_str(), &limits)
            .await
            .unwrap();
        assert_eq!(output.stderr, "err\n");
        assert!(output.stdout.starts_with("out\n"));
        assert!(output.stdout.trim_end().ends_with(dir.path().file_name().unwrap().to_str().unwrap()));
        assert_eq!(output.exit_code, Some(3));

        let small = Limits { output: 10, ..limits };
        let output = LocalHost.run("seq 1 1000", None, &small).await.unwrap();
        assert_eq!(output.stdout.len(), 10);
        assert!(output.truncated);

        let quick = Limits { timeout: Duration::from_millis(200), ..limits };
        assert_eq!(LocalHost.run("sleep 5", None, &quick).await, Err(HostError::Timeout(0)));
    }

    #[tokio::test]
    async fn local_files_read_write_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        let path = path.to_str().unwrap();
        let limits = Limits::default();
        write_text(&LocalHost, path, "hello\nworld\n", &limits).await.unwrap();
        let text = read_text(&LocalHost, path, &limits).await.unwrap();
        assert_eq!(text, FileText { content: "hello\nworld\n".into(), truncated: false });
        let short = read_text(&LocalHost, path, &Limits { file: 5, ..limits }).await.unwrap();
        assert_eq!(short, FileText { content: "hello".into(), truncated: true });
        assert!(matches!(write_text(&LocalHost, path, "too long", &Limits { file: 3, ..limits }).await, Err(HostError::TooLarge(..))));

        std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2]).unwrap();
        assert!(matches!(read_text(&LocalHost, dir.path().join("blob.bin").to_str().unwrap(), &limits).await, Err(HostError::Binary(_))));
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let (entries, more) = list_dir(&LocalHost, dir.path().to_str().unwrap(), &limits).await.unwrap();
        let names: Vec<(&str, &str)> = entries.iter().map(|entry| (entry.name.as_str(), entry.kind)).collect();
        assert_eq!(names, [("blob.bin", "file"), ("notes.txt", "file"), ("sub", "dir")]);
        assert!(!more);
        let (entries, more) = list_dir(&LocalHost, dir.path().to_str().unwrap(), &Limits { entries: 1, ..limits }).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(more);
    }
}
