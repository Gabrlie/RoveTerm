//! Bridges session lifecycle live events into journal records without blocking the emitter.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use rove_kernel::{Disposer, EventBus};
use rove_session::{SessionExited, SessionOpened};
use serde_json::json;

use crate::{JournalStore, NewRecord};

/// Journal kind of [`SessionOpened`] records.
pub const KIND_SESSION_OPENED: &str = "session/opened";
/// Journal kind of [`SessionExited`] records.
pub const KIND_SESSION_EXITED: &str = "session/exited";

/// Appends records on a dedicated writer thread, so live-event listeners never wait on disk I/O.
///
/// The writer thread ends after every `Recorder` clone and every subscription has been dropped and
/// the queue has drained.
#[derive(Clone)]
pub struct Recorder {
    queue: Sender<NewRecord>,
}

impl Recorder {
    /// Starts the writer thread for `store`.
    pub fn start(store: Arc<dyn JournalStore>) -> std::io::Result<Self> {
        let (queue, records) = channel::<NewRecord>();
        std::thread::Builder::new()
            .name("journal-writer".into())
            .spawn(move || {
                while let Ok(record) = records.recv() {
                    if let Err(error) = store.append(record) {
                        tracing::error!(%error, "journal append failed");
                    }
                }
            })?;
        Ok(Self { queue })
    }

    /// Queues `record` for appending. Records queued after the writer stopped are dropped.
    pub fn record(&self, record: NewRecord) {
        if self.queue.send(record).is_err() {
            tracing::warn!("journal writer stopped; record dropped");
        }
    }

    /// Subscribes to session lifecycle events on `events`.
    pub fn subscribe(&self, events: &EventBus) -> Vec<Disposer> {
        let opened = self.clone();
        let exited = self.clone();
        vec![
            events.on::<SessionOpened>(move |event| {
                opened.record(NewRecord {
                    kind: KIND_SESSION_OPENED.into(),
                    session_id: Some(event.id.to_string()),
                    payload: json!({ "info": event.info }),
                });
            }),
            events.on::<SessionExited>(move |event| {
                exited.record(NewRecord {
                    kind: KIND_SESSION_EXITED.into(),
                    session_id: Some(event.id.to_string()),
                    payload: json!({ "status": event.status }),
                });
            }),
        ]
    }
}
