//! The host's notes as agent tools: offered only with a notes source, read freely, changed
//! after approval.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rove_ai::Answer;
use rove_ai::host::LocalHost;
use rove_ai::session::{AiEvent, AiSession, NotesSource, SessionSettings};
use serde_json::json;
use tokio::sync::mpsc;

#[derive(Default)]
struct Memory(Mutex<String>);

#[async_trait]
impl NotesSource for Memory {
    async fn read(&self) -> Result<String, String> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn write(&self, text: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = text.to_owned();
        Ok(())
    }
}

#[tokio::test]
async fn notes_are_read_freely_and_changed_with_approval() {
    let (events, mut received) = mpsc::unbounded_channel::<AiEvent>();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    ));
    let names = |specs: serde_json::Value| -> Vec<String> {
        specs
            .as_array()
            .unwrap()
            .iter()
            .map(|spec| spec["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(
        !names(rove_ai::tools::specs("h", session.has_notes())).contains(&"read_notes".to_owned())
    );
    let memory = Arc::new(Memory::default());
    session.set_notes_source(memory.clone());
    assert!(
        names(rove_ai::tools::specs("h", session.has_notes())).contains(&"update_notes".to_owned())
    );

    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    let approver = Arc::downgrade(&session);
    tokio::spawn(async move {
        while let Some(event) = received.recv().await {
            if let (
                AiEvent::Approval {
                    request_id,
                    operation,
                    ..
                },
                Some(session),
            ) = (&event, approver.upgrade())
            {
                seen.lock()
                    .unwrap()
                    .push(serde_json::to_value(operation).unwrap());
                session.answer(*request_id, Answer::AllowOnce).await;
            }
        }
    });
    let read = rove_ai::tools::call("read_notes", &json!({}), &session)
        .await
        .unwrap();
    assert_eq!(read.text, "(no notes yet)");
    let written = rove_ai::tools::call("update_notes", &json!({ "text": "nginx 1.26" }), &session)
        .await
        .unwrap();
    assert!(!written.is_error);
    assert_eq!(*memory.0.lock().unwrap(), "nginx 1.26");
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["op"], "update_notes");
    assert_eq!(asked[0]["diff"], "+ nginx 1.26\n");
}
