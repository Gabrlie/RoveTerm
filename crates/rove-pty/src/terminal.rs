//! PTY-backed [`TerminalSession`] implementation.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use rove_session::{
    ExitStatus, OpenedTerminal, SessionError, SessionInfo, SessionKind, TermSize, TerminalOutput,
    TerminalSession,
};
use tokio::sync::mpsc;

use crate::{LaunchSpec, LocalRequest, PtyConfig};

const READ_CHUNK: usize = 16 * 1024;
const OUTPUT_QUEUE: usize = 256;

/// Opens local PTY terminals.
pub struct LocalTerminals {
    config: PtyConfig,
}

impl LocalTerminals {
    /// Creates the provider with the plugin configuration.
    pub fn new(config: PtyConfig) -> Self {
        Self { config }
    }

    /// Resolves `request` and starts the shell.
    ///
    /// Spawns three threads per terminal: a reader forwarding output, a writer serializing input,
    /// and a waiter collecting the exit status. All three end when the shell exits or the session
    /// is closed.
    pub fn open(&self, request: LocalRequest) -> Result<OpenedTerminal, SessionError> {
        let spec = request.resolve(&self.config);
        open_spec(spec)
    }
}

fn pty_size(size: TermSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn provider_error(context: &str, error: impl std::fmt::Display) -> SessionError {
    SessionError::Provider(format!("{context}: {error}"))
}

fn open_spec(spec: LaunchSpec) -> Result<OpenedTerminal, SessionError> {
    let pair = native_pty_system()
        .openpty(pty_size(spec.size))
        .map_err(|error| provider_error("open pty", error))?;

    let mut command = CommandBuilder::new(&spec.program);
    command.args(&spec.args);
    if let Some(cwd) = spec.cwd.as_deref().filter(|cwd| cwd.is_dir()) {
        command.cwd(cwd);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| provider_error("spawn shell", error))?;
    // The parent must not hold the slave side, or the reader never sees end-of-file.
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| provider_error("clone pty reader", error))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| provider_error("take pty writer", error))?;
    let killer = child.clone_killer();

    let (output_tx, output_rx) = mpsc::channel(OUTPUT_QUEUE);
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (status_tx, status_rx) = std::sync::mpsc::channel::<ExitStatus>();

    let session = Arc::new(PtySession {
        master: Mutex::new(Some(pair.master)),
        killer: Mutex::new(killer),
        input: Mutex::new(Some(input_tx)),
        closed: AtomicBool::new(false),
    });

    spawn_named("pty-writer", move || write_loop(writer, input_rx))?;
    spawn_named("pty-reader", move || {
        read_loop(reader, output_tx, status_rx)
    })?;
    let waiter_session = session.clone();
    spawn_named("pty-waiter", move || {
        let status = match child.wait() {
            Ok(status) => ExitStatus {
                code: i32::try_from(status.exit_code()).ok(),
                signal: status.signal().map(str::to_owned),
            },
            Err(error) => {
                tracing::warn!(%error, "waiting for shell failed");
                ExitStatus::default()
            }
        };
        // Releasing the pseudo console lets the reader observe end-of-file on Windows.
        if cfg!(windows) {
            waiter_session.release();
        }
        let _receiver_may_be_gone = status_tx.send(status);
    })?;

    let title = Path::new(&spec.program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&spec.program)
        .to_owned();
    Ok(OpenedTerminal {
        session,
        output: output_rx,
        info: SessionInfo {
            kind: SessionKind::Local {
                shell: spec.program,
            },
            title,
        },
    })
}

fn spawn_named(name: &str, body: impl FnOnce() + Send + 'static) -> Result<(), SessionError> {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .map(drop)
        .map_err(|error| provider_error("spawn thread", error))
}

fn write_loop(mut writer: Box<dyn Write + Send>, input: std::sync::mpsc::Receiver<Vec<u8>>) {
    while let Ok(data) = input.recv() {
        if let Err(error) = writer.write_all(&data).and_then(|()| writer.flush()) {
            tracing::debug!(%error, "pty write failed; stopping writer");
            break;
        }
    }
}

fn read_loop(
    mut reader: Box<dyn Read + Send>,
    output: mpsc::Sender<TerminalOutput>,
    status: std::sync::mpsc::Receiver<ExitStatus>,
) {
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let chunk = Bytes::copy_from_slice(&buffer[..read]);
                if output.blocking_send(TerminalOutput::Data(chunk)).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            // Linux reports EIO on the master once the slave side is fully closed.
            Err(_eof_or_failure) => break,
        }
    }
    let status = status.recv().unwrap_or_default();
    let _receiver_may_be_gone = output.blocking_send(TerminalOutput::Exit(status));
}

struct PtySession {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    input: Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>,
    closed: AtomicBool,
}

impl PtySession {
    fn release(&self) {
        self.input.lock().take();
        self.master.lock().take();
    }
}

#[async_trait]
impl TerminalSession for PtySession {
    async fn write(&self, data: &[u8]) -> Result<(), SessionError> {
        let input = self.input.lock();
        let sender = input.as_ref().ok_or(SessionError::Closed)?;
        sender.send(data.to_vec()).map_err(|_| SessionError::Closed)
    }

    async fn resize(&self, size: TermSize) -> Result<(), SessionError> {
        let master = self.master.lock();
        let master = master.as_ref().ok_or(SessionError::Closed)?;
        master
            .resize(pty_size(size))
            .map_err(|error| provider_error("resize pty", error))
    }

    async fn close(&self) -> Result<(), SessionError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Err(error) = self.killer.lock().kill() {
            // The shell may already have exited; nothing remains to terminate.
            tracing::debug!(%error, "killing shell failed");
        }
        self.release();
        Ok(())
    }
}
