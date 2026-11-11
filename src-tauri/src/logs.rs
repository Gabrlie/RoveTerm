//! The application log: to the terminal, and once the data directory is known, to daily files
//! under `<data>/logs` (`roveterm.YYYY-MM-DD.log`, the newest seven kept). Logs never contain
//! secrets: code logs ids and states, never passwords, keys or tokens.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Layer as _;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Daily files kept.
const KEPT: usize = 7;

static FILE: OnceLock<Mutex<RollingFileAppender>> = OnceLock::new();
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Starts logging to the terminal; level from `RUST_LOG`, `info` by default.
pub fn init() {
    let filter =
        || tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
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
