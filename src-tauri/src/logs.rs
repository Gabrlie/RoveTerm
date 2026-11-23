//! The application log: to the terminal, and once the data directory is known, to daily files
//! under `<data>/logs` (`roveterm.YYYY-MM-DD.log`, the newest seven kept). Logs never contain
//! secrets: code logs ids and states, never passwords, keys or tokens.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Daily files kept.
const KEPT: usize = 7;
/// Default number of trailing log lines returned by [`read_tail_in`].
pub const DEFAULT_TAIL_LINES: usize = 500;
/// Maximum number of trailing log lines returned by [`read_tail_in`].
pub const MAX_TAIL_LINES: usize = 2000;
/// Maximum bytes read from the tail of a log file in a single call.
pub const MAX_READ_BYTES: usize = 256 * 1024;

static FILE: OnceLock<Mutex<RollingFileAppender>> = OnceLock::new();
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Summary of one log file on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogFileInfo {
    pub name: String,
    pub size_bytes: u64,
    /// Last modification timestamp in Unix milliseconds.
    pub modified_at: i64,
}

/// Tail of a log file returned to the in-app log viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogReadResult {
    pub file: String,
    pub content: String,
    pub truncated: bool,
    pub total_bytes: u64,
}

/// Errors while listing or reading log files.
#[derive(Debug, thiserror::Error)]
pub enum LogReadError {
    #[error("invalid log file name")]
    InvalidName,
    #[error("log file not found")]
    NotFound,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Starts logging to the terminal; level from `RUST_LOG`, `info` by default.
pub fn init() {
    let filter =
        || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
                .with_filter(filter()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .fmt_fields(PlainFields::default())
                .with_writer(Files)
                .with_filter(filter()),
        )
        .init();
}

/// Also writes the log to files under `data_dir/logs` from now on.
pub fn start_files(data_dir: &Path) -> std::io::Result<()> {
    let dir = data_dir.join("logs");
    std::fs::create_dir_all(&dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("roveterm")
        .filename_suffix("log")
        .max_log_files(KEPT)
        .build(&dir)
        .map_err(std::io::Error::other)?;
    // Set once per process; a second call keeps the first directory.
    let _ = FILE.set(Mutex::new(appender));
    let _ = DIR.set(dir);
    Ok(())
}

/// Where the log files are, once they are written.
pub fn dir() -> Option<&'static Path> {
    DIR.get().map(PathBuf::as_path)
}

/// Flushes buffered log lines to disk so an immediate read sees the latest entries.
pub fn flush() {
    if let Some(file) = FILE.get() {
        let _ = file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .flush();
    }
}

/// Checks that `name` is a single safe log filename (`roveterm*.log`, no separators or traversal).
pub fn is_valid_log_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 128
        || !name.starts_with("roveterm")
        || !name.ends_with(".log")
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

/// Lists valid log files in `dir`, newest first.
pub fn list_files_in(dir: &Path) -> std::io::Result<Vec<LogFileInfo>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_valid_log_name(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let modified_at = meta
            .modified()
            .ok()
            .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        files.push(LogFileInfo {
            name,
            size_bytes: meta.len(),
            modified_at,
        });
    }
    files.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| b.name.cmp(&a.name)));
    Ok(files)
}

/// Reads the trailing lines of `file` (or the newest file in `dir` when `None`), bounded by
/// [`MAX_TAIL_LINES`] and [`MAX_READ_BYTES`].
pub fn read_tail_in(
    dir: &Path,
    file: Option<&str>,
    tail_lines: Option<usize>,
) -> Result<LogReadResult, LogReadError> {
    let name = match file.map(str::trim).filter(|value| !value.is_empty()) {
        Some(name) => {
            if !is_valid_log_name(name) {
                return Err(LogReadError::InvalidName);
            }
            name.to_owned()
        }
        None => {
            let files = list_files_in(dir)?;
            let Some(first) = files.into_iter().next() else {
                return Ok(LogReadResult {
                    file: String::new(),
                    content: String::new(),
                    truncated: false,
                    total_bytes: 0,
                });
            };
            first.name
        }
    };
    let path = dir.join(&name);
    let mut handle = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(LogReadError::NotFound);
        }
        Err(error) => return Err(LogReadError::Io(error)),
    };
    let total_bytes = handle.metadata()?.len();
    let mut truncated = false;
    let mut buf = Vec::new();
    if total_bytes > MAX_READ_BYTES as u64 {
        truncated = true;
        handle.seek(SeekFrom::End(-(MAX_READ_BYTES as i64)))?;
        handle.read_to_end(&mut buf)?;
        // Drop the first partial line when starting mid-file.
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            buf.drain(..=pos);
        }
    } else {
        handle.read_to_end(&mut buf)?;
    }

    let text = String::from_utf8_lossy(&buf);
    let max_lines = tail_lines
        .unwrap_or(DEFAULT_TAIL_LINES)
        .clamp(1, MAX_TAIL_LINES);
    let lines: Vec<&str> = text.lines().collect();
    let content = if lines.len() > max_lines {
        truncated = true;
        lines[lines.len() - max_lines..].join("\n")
    } else {
        lines.join("\n")
    };

    Ok(LogReadResult {
        file: name,
        content,
        truncated,
        total_bytes,
    })
}

/// Distinct field formatter type so the plain-text file layer never reuses ANSI-styled
/// `FormattedFields<DefaultFields>` cached by the terminal layer on shared spans.
#[derive(Default)]
struct PlainFields(tracing_subscriber::fmt::format::DefaultFields);

impl<'writer> tracing_subscriber::fmt::FormatFields<'writer> for PlainFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: tracing_subscriber::fmt::format::Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        self.0.format_fields(writer, fields)
    }
}

/// Writes to the current log file, or nowhere before there is one.
struct Files;

impl<'a> MakeWriter<'a> for Files {
    type Writer = FileHandle;

    fn make_writer(&'a self) -> Self::Writer {
        FileHandle
    }
}

struct FileHandle;

impl Write for FileHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match FILE.get() {
            Some(file) => file
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match FILE.get() {
            Some(file) => file
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_log_file_names() {
        assert!(is_valid_log_name("roveterm.2026-11-22.log"));
        assert!(is_valid_log_name("roveterm.log"));
        assert!(!is_valid_log_name(""));
        assert!(!is_valid_log_name("../roveterm.log"));
        assert!(!is_valid_log_name("sub/roveterm.log"));
        assert!(!is_valid_log_name("sub\\roveterm.log"));
        assert!(!is_valid_log_name("roveterm..log"));
        assert!(!is_valid_log_name("other.log"));
        assert!(!is_valid_log_name("roveterm.txt"));
    }

    #[test]
    fn lists_and_reads_tail_safely() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("unrelated.txt"), "ignore").unwrap();
        std::fs::write(dir.path().join("roveterm.2026-11-20.log"), "old-1\nold-2\n").unwrap();
        let lines: String = (1..=10).map(|i| format!("line-{i}\n")).collect();
        std::fs::write(dir.path().join("roveterm.2026-11-22.log"), lines).unwrap();

        let listed = list_files_in(dir.path()).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|f| f.name == "roveterm.2026-11-22.log"));

        let read = read_tail_in(dir.path(), Some("roveterm.2026-11-22.log"), Some(3)).unwrap();
        assert_eq!(read.file, "roveterm.2026-11-22.log");
        assert!(read.truncated);
        assert_eq!(read.content, "line-8\nline-9\nline-10");

        assert!(matches!(
            read_tail_in(dir.path(), Some("../roveterm.log"), None),
            Err(LogReadError::InvalidName)
        ));
        assert!(matches!(
            read_tail_in(dir.path(), Some("roveterm.missing.log"), None),
            Err(LogReadError::NotFound)
        ));
    }

    #[test]
    fn caps_reads_on_large_log_files() {
        let dir = tempfile::tempdir().unwrap();
        let row = "2026-11-22T01:00:00Z INFO rove_sync: 测试中文日志行 payload-data-0123456789\n";
        let repeats = (MAX_READ_BYTES / row.len()) + 40;
        let mut big = String::with_capacity(row.len() * repeats);
        for i in 0..repeats {
            big.push_str(&format!("{i:05} {row}"));
        }
        std::fs::write(dir.path().join("roveterm.2026-11-22.log"), &big).unwrap();

        let read = read_tail_in(dir.path(), None, Some(5)).unwrap();
        assert!(read.truncated);
        assert_eq!(read.content.lines().count(), 5);
        assert!(read.content.contains(&format!("{:05}", repeats - 1)));
    }

    #[test]
    fn file_layer_never_inherits_ansi_span_fields_from_terminal_layer() {
        use std::sync::Arc;

        #[derive(Clone, Default)]
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);

        struct GuardWriter(Arc<Mutex<Vec<u8>>>);

        impl Write for GuardWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> MakeWriter<'a> for SharedBuf {
            type Writer = GuardWriter;

            fn make_writer(&'a self) -> Self::Writer {
                GuardWriter(self.0.clone())
            }
        }

        let tty_buf = SharedBuf::default();
        let file_buf = SharedBuf::default();
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(true)
                    .with_writer(tty_buf.clone()),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .fmt_fields(PlainFields::default())
                    .with_writer(file_buf.clone()),
            );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "actor.lifecycle",
                actor.name = "ts_runtime::derp_latency::DerpLatencyMeasurer",
                actor.id = 23
            );
            let _entered = span.enter();
            tracing::info!(region_id = 10, latency_ms = 395, "selected home DERP");
        });

        let tty_out = String::from_utf8(tty_buf.0.lock().unwrap().clone()).unwrap();
        let file_out = String::from_utf8(file_buf.0.lock().unwrap().clone()).unwrap();
        assert!(tty_out.contains('\x1b'));
        assert!(!file_out.contains('\x1b'), "unexpected ANSI escape in file log: {file_out:?}");
        assert!(file_out.contains("actor.lifecycle{actor.name=\"ts_runtime::derp_latency::DerpLatencyMeasurer\" actor.id=23}"));
        assert!(file_out.contains("selected home DERP"));
    }
}
