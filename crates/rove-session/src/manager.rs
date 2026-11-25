//! The session table: attaches provider output to a sink, coalescing and recording it for replay.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use parking_lot::Mutex;
use rove_kernel::{Event, EventBus};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout_at};

use crate::replay::ReplayBuffer;
use crate::{
    ExitStatus, SessionError, SessionId, SessionInfo, TermSize, TerminalOutput, TerminalSession,
};

/// Receives coalesced output for one session, usually forwarding it to a UI view.
pub trait OutputSink: Send + Sync + 'static {
    /// Delivers a chunk. Must not block. Returns `false` when the receiver is gone; the manager
    /// then stops forwarding but keeps recording output for replay.
    fn send(&self, chunk: OutputChunk) -> bool;
}

/// A chunk delivered to an [`OutputSink`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputChunk {
    /// Coalesced terminal bytes.
    Data(Bytes),
    /// The session ended.
    Exit(ExitStatus),
}

/// A provider's freshly opened terminal, handed to [`SessionManager::attach`].
pub struct OpenedTerminal {
    /// Control handle.
    pub session: Arc<dyn TerminalSession>,
    /// Output stream; the provider closes it after sending [`TerminalOutput::Exit`] or on failure.
    pub output: mpsc::Receiver<TerminalOutput>,
    /// Descriptive metadata.
    pub info: SessionInfo,
}

/// Session manager tunables.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SessionConfig {
    /// Maximum time to hold output before forwarding it, in milliseconds (1–50).
    pub coalesce_ms: u64,
    /// Forward immediately once this many bytes are buffered (4 KiB–1 MiB).
    pub max_chunk_bytes: usize,
    /// Bytes of recent output kept per session for replay (0–16 MiB).
    pub replay_bytes: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            coalesce_ms: 6,
            max_chunk_bytes: 64 * 1024,
            replay_bytes: 256 * 1024,
        }
    }
}

impl SessionConfig {
    /// Checks every field against its documented range.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=50).contains(&self.coalesce_ms) {
            return Err(format!(
                "coalesce_ms must be 1..=50, got {}",
                self.coalesce_ms
            ));
        }
        if !(4 * 1024..=1024 * 1024).contains(&self.max_chunk_bytes) {
            return Err(format!(
                "max_chunk_bytes must be 4096..=1048576, got {}",
                self.max_chunk_bytes
            ));
        }
        if self.replay_bytes > 16 * 1024 * 1024 {
            return Err(format!(
                "replay_bytes must be at most 16777216, got {}",
                self.replay_bytes
            ));
        }
        Ok(())
    }
}

/// Live event: a session was attached.
#[derive(Debug, Clone, Serialize)]
pub struct SessionOpened {
    /// The new session.
    pub id: SessionId,
    /// Its metadata.
    pub info: SessionInfo,
}
impl Event for SessionOpened {}

/// Live event: a session's output ended, either because the target exited or it was closed.
#[derive(Debug, Clone, Serialize)]
pub struct SessionExited {
    /// The ended session.
    pub id: SessionId,
    /// Exit status reported by the provider; default when the stream closed without one.
    pub status: ExitStatus,
}
impl Event for SessionExited {}

/// Live event emitted after a session is removed from the table, before closing its transport.
#[derive(Debug, Clone)]
pub struct SessionClosed {
    /// The removed session.
    pub id: SessionId,
}
impl Event for SessionClosed {}

/// A listing row returned by [`SessionManager::list`].
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    /// Session identifier.
    pub id: SessionId,
    /// Metadata.
    pub info: SessionInfo,
    /// Exit status once the session has ended.
    pub exited: Option<ExitStatus>,
}

struct Output {
    replay: ReplayBuffer,
    sink: Option<Arc<dyn OutputSink>>,
    exited: Option<ExitStatus>,
}

struct Entry {
    session: Arc<dyn TerminalSession>,
    info: SessionInfo,
    output: Mutex<Output>,
}

/// Owns every live session. Sessions outlive UI views: a view re-attaches with
/// [`SessionManager::reattach`] and repaints from the replay buffer.
pub struct SessionManager {
    config: SessionConfig,
    events: EventBus,
    sessions: Mutex<HashMap<SessionId, Arc<Entry>>>,
}

impl SessionManager {
    /// Creates a manager that publishes live events on `events`.
    pub fn new(config: SessionConfig, events: EventBus) -> Self {
        Self {
            config,
            events,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Registers `opened`, starts forwarding its output to `sink`, and emits [`SessionOpened`].
    ///
    /// Must be called inside a Tokio runtime; the output pump runs as a task that ends when the
    /// provider closes the output stream.
    pub fn attach(&self, opened: OpenedTerminal, sink: Arc<dyn OutputSink>) -> SessionId {
        let id = SessionId::new();
        let entry = Arc::new(Entry {
            session: opened.session,
            info: opened.info,
            output: Mutex::new(Output {
                replay: ReplayBuffer::new(self.config.replay_bytes),
                sink: Some(sink),
                exited: None,
            }),
        });
        self.sessions.lock().insert(id, entry.clone());
        self.events.emit(&SessionOpened {
            id,
            info: entry.info.clone(),
        });
        tokio::spawn(pump(
            id,
            entry,
            opened.output,
            self.config.clone(),
            self.events.clone(),
        ));
        id
    }

    fn entry(&self, id: SessionId) -> Result<Arc<Entry>, SessionError> {
        self.sessions
            .lock()
            .get(&id)
            .cloned()
            .ok_or(SessionError::NotFound(id))
    }

    /// Writes input bytes to the session.
    pub async fn write(&self, id: SessionId, data: &[u8]) -> Result<(), SessionError> {
        self.entry(id)?.session.write(data).await
    }

    /// Resizes the session's terminal.
    pub async fn resize(&self, id: SessionId, size: TermSize) -> Result<(), SessionError> {
        self.entry(id)?.session.resize(size).await
    }

    /// Closes the session and removes it from the table. Closing an unknown id returns
    /// [`SessionError::NotFound`].
    pub async fn close(&self, id: SessionId) -> Result<(), SessionError> {
        let entry = self
            .sessions
            .lock()
            .remove(&id)
            .ok_or(SessionError::NotFound(id))?;
        self.events.emit(&SessionClosed { id });
        entry.output.lock().sink = None;
        entry.session.close().await
    }

    /// Replaces the session's sink and returns the replay buffer contents.
    ///
    /// The snapshot and the sink swap happen under one lock, so the new sink receives exactly the
    /// output that follows the returned bytes. Returns the exit status too when the session ended.
    pub fn reattach(
        &self,
        id: SessionId,
        sink: Arc<dyn OutputSink>,
    ) -> Result<(Bytes, Option<ExitStatus>), SessionError> {
        let entry = self.entry(id)?;
        let mut output = entry.output.lock();
        let snapshot = output.replay.snapshot();
        output.sink = Some(sink);
        Ok((snapshot, output.exited.clone()))
    }

    /// The replay buffer's contents, without touching the session's sink.
    pub fn snapshot(&self, id: SessionId) -> Result<Bytes, SessionError> {
        Ok(self.entry(id)?.output.lock().replay.snapshot())
    }

    /// Lists sessions in no particular order.
    pub fn list(&self) -> Vec<SessionSummary> {
        self.sessions
            .lock()
            .iter()
            .map(|(id, entry)| SessionSummary {
                id: *id,
                info: entry.info.clone(),
                exited: entry.output.lock().exited.clone(),
            })
            .collect()
    }

    /// Closes every session. Used on shutdown.
    pub async fn close_all(&self) {
        let entries: Vec<_> = self.sessions.lock().drain().collect();
        for &(id, _) in &entries {
            self.events.emit(&SessionClosed { id });
        }
        for (_, entry) in entries {
            if let Err(error) = entry.session.close().await {
                tracing::warn!(%error, "closing session during shutdown failed");
            }
        }
    }
}

fn forward(entry: &Entry, chunk: Bytes) {
    if chunk.is_empty() {
        return;
    }
    let mut output = entry.output.lock();
    output.replay.push(&chunk);
    if let Some(sink) = &output.sink
        && !sink.send(OutputChunk::Data(chunk))
    {
        output.sink = None;
    }
}

async fn pump(
    id: SessionId,
    entry: Arc<Entry>,
    mut rx: mpsc::Receiver<TerminalOutput>,
    config: SessionConfig,
    events: EventBus,
) {
    let window = Duration::from_millis(config.coalesce_ms);
    let mut buffer = BytesMut::new();
    let mut status = None;
    'outer: while let Some(first) = rx.recv().await {
        match first {
            TerminalOutput::Exit(exit) => {
                status = Some(exit);
                break;
            }
            TerminalOutput::Data(data) => buffer.extend_from_slice(&data),
        }
        let deadline = Instant::now() + window;
        while buffer.len() < config.max_chunk_bytes {
            match timeout_at(deadline, rx.recv()).await {
                Ok(Some(TerminalOutput::Data(data))) => buffer.extend_from_slice(&data),
                Ok(Some(TerminalOutput::Exit(exit))) => {
                    status = Some(exit);
                    forward(&entry, buffer.split().freeze());
                    break 'outer;
                }
                Ok(None) => {
                    forward(&entry, buffer.split().freeze());
                    break 'outer;
                }
                Err(_elapsed) => break,
            }
        }
        forward(&entry, buffer.split().freeze());
    }
    let status = status.unwrap_or_default();
    {
        let mut output = entry.output.lock();
        output.exited = Some(status.clone());
        if let Some(sink) = output.sink.take() {
            sink.send(OutputChunk::Exit(status.clone()));
        }
    }
    events.emit(&SessionExited { id, status });
}
