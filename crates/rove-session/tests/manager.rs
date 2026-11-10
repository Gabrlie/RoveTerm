use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use rove_kernel::EventBus;
use rove_session::*;
use tokio::sync::mpsc;

#[derive(Default)]
struct FakeSession {
    written: Mutex<Vec<u8>>,
    closed: Mutex<bool>,
}

#[async_trait]
impl TerminalSession for FakeSession {
    async fn write(&self, data: &[u8]) -> Result<(), SessionError> {
        self.written.lock().extend_from_slice(data);
        Ok(())
    }
    async fn resize(&self, _size: TermSize) -> Result<(), SessionError> {
        Ok(())
    }
    async fn close(&self) -> Result<(), SessionError> {
        *self.closed.lock() = true;
        Ok(())
    }
}

#[derive(Default)]
struct RecordingSink {
    chunks: Mutex<Vec<OutputChunk>>,
}

impl OutputSink for RecordingSink {
    fn send(&self, chunk: OutputChunk) -> bool {
        self.chunks.lock().push(chunk);
        true
    }
}

impl RecordingSink {
    fn data(&self) -> Vec<u8> {
        self.chunks
            .lock()
            .iter()
            .filter_map(|chunk| match chunk {
                OutputChunk::Data(data) => Some(data.to_vec()),
                OutputChunk::Exit(_) => None,
            })
            .flatten()
            .collect()
    }
}

fn info() -> SessionInfo {
    SessionInfo {
        kind: SessionKind::Local { shell: "sh".into() },
        title: "sh".into(),
    }
}

fn open(
    manager: &SessionManager,
    sink: Arc<RecordingSink>,
) -> (SessionId, Arc<FakeSession>, mpsc::Sender<TerminalOutput>) {
    let session = Arc::new(FakeSession::default());
    let (tx, rx) = mpsc::channel(64);
    let id = manager.attach(
        OpenedTerminal {
            session: session.clone(),
            output: rx,
            info: info(),
        },
        sink,
    );
    (id, session, tx)
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(30)).await;
}

#[tokio::test]
async fn coalesces_bursts_into_one_chunk() {
    let manager = SessionManager::new(SessionConfig::default(), EventBus::default());
    let sink = Arc::new(RecordingSink::default());
    let (_id, _session, tx) = open(&manager, sink.clone());
    for part in [&b"a"[..], b"b", b"c"] {
        tx.send(TerminalOutput::Data(Bytes::copy_from_slice(part)))
            .await
            .unwrap();
    }
    settle().await;
    assert_eq!(
        *sink.chunks.lock(),
        vec![OutputChunk::Data(Bytes::from_static(b"abc"))]
    );
}

#[tokio::test]
async fn splits_at_max_chunk_bytes() {
    let config = SessionConfig {
        coalesce_ms: 50,
        max_chunk_bytes: 4096,
        ..SessionConfig::default()
    };
    let manager = SessionManager::new(config, EventBus::default());
    let sink = Arc::new(RecordingSink::default());
    let (_id, _session, tx) = open(&manager, sink.clone());
    for _ in 0..3 {
        tx.send(TerminalOutput::Data(Bytes::from(vec![b'x'; 2048])))
            .await
            .unwrap();
    }
    settle().await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    let sizes: Vec<usize> = sink
        .chunks
        .lock()
        .iter()
        .map(|chunk| match chunk {
            OutputChunk::Data(data) => data.len(),
            OutputChunk::Exit(_) => 0,
        })
        .collect();
    assert_eq!(sizes, vec![4096, 2048]);
}

#[tokio::test]
async fn reattach_replays_history_and_switches_sink() {
    let manager = SessionManager::new(SessionConfig::default(), EventBus::default());
    let first = Arc::new(RecordingSink::default());
    let (id, _session, tx) = open(&manager, first.clone());
    tx.send(TerminalOutput::Data(Bytes::from_static(b"hello ")))
        .await
        .unwrap();
    settle().await;

    let second = Arc::new(RecordingSink::default());
    let (replay, exited) = manager.reattach(id, second.clone()).unwrap();
    assert_eq!(&replay[..], b"hello ");
    assert!(exited.is_none());

    tx.send(TerminalOutput::Data(Bytes::from_static(b"world")))
        .await
        .unwrap();
    settle().await;
    assert_eq!(first.data(), b"hello ");
    assert_eq!(second.data(), b"world");
}

#[tokio::test]
async fn snapshot_reads_history_and_keeps_the_sink() {
    let manager = SessionManager::new(SessionConfig::default(), EventBus::default());
    let sink = Arc::new(RecordingSink::default());
    let (id, _session, tx) = open(&manager, sink.clone());
    tx.send(TerminalOutput::Data(Bytes::from_static(b"a")))
        .await
        .unwrap();
    settle().await;
    assert_eq!(&manager.snapshot(id).unwrap()[..], b"a");
    tx.send(TerminalOutput::Data(Bytes::from_static(b"b")))
        .await
        .unwrap();
    settle().await;
    assert_eq!(sink.data(), b"ab");
    assert_eq!(&manager.snapshot(id).unwrap()[..], b"ab");
}

#[tokio::test]
async fn exit_is_forwarded_and_announced() {
    let events = EventBus::default();
    let exits = Arc::new(Mutex::new(Vec::new()));
    let seen = exits.clone();
    let _listener = events.on::<SessionExited>(move |event| seen.lock().push(event.status.code));
    let manager = SessionManager::new(SessionConfig::default(), events);
    let sink = Arc::new(RecordingSink::default());
    let (id, _session, tx) = open(&manager, sink.clone());
    tx.send(TerminalOutput::Data(Bytes::from_static(b"bye")))
        .await
        .unwrap();
    tx.send(TerminalOutput::Exit(ExitStatus {
        code: Some(3),
        signal: None,
    }))
    .await
    .unwrap();
    settle().await;
    assert_eq!(sink.data(), b"bye");
    assert!(
        matches!(sink.chunks.lock().last(), Some(OutputChunk::Exit(status)) if status.code == Some(3))
    );
    assert_eq!(*exits.lock(), vec![Some(3)]);
    assert_eq!(
        manager.list()[0].exited.as_ref().and_then(|s| s.code),
        Some(3)
    );
    assert_eq!(manager.list()[0].id, id);
}

#[tokio::test]
async fn write_close_and_unknown_ids() {
    let manager = SessionManager::new(SessionConfig::default(), EventBus::default());
    let sink = Arc::new(RecordingSink::default());
    let (id, session, _tx) = open(&manager, sink);
    manager.write(id, b"ls\r").await.unwrap();
    assert_eq!(&session.written.lock()[..], b"ls\r");
    manager.close(id).await.unwrap();
    assert!(*session.closed.lock());
    assert!(matches!(
        manager.write(id, b"x").await,
        Err(SessionError::NotFound(_))
    ));
    assert!(manager.list().is_empty());
}

#[test]
fn rejects_out_of_range_sizes_and_config() {
    assert!(TermSize::new(0, 24).is_err());
    assert!(TermSize::new(80, 1001).is_err());
    assert!(TermSize::new(80, 24).is_ok());
    let config = SessionConfig {
        coalesce_ms: 0,
        ..SessionConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn plugin_rejects_invalid_config() {
    use rove_kernel::{Plugin, PluginConfig};
    let ctx = rove_kernel::Context::new();
    let table: toml::Table = toml::from_str("coalesce_ms = 500").unwrap();
    let result = SessionPlugin.apply(&ctx, &PluginConfig::new("session", table));
    assert!(result.is_err());
}
