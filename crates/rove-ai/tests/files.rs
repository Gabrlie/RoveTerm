//! Writing files after an approval: what changed meanwhile is not lost.

use std::path::PathBuf;
use std::sync::Arc;

use rove_ai::host::LocalHost;
use rove_ai::session::{AiEvent, AiSession, SessionSettings};
use rove_ai::{Answer, Mode};
use tokio::sync::mpsc;

/// A session that asks about every change; on each approval `meanwhile` runs, then the change
/// is allowed.
fn session(meanwhile: impl Fn() + Send + 'static) -> Arc<AiSession> {
    let (events, mut received) = mpsc::unbounded_channel::<AiEvent>();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings {
            mode: Mode::AskByRisk,
            ..SessionSettings::default()
        },
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    ));
    let approver = Arc::downgrade(&session);
    tokio::spawn(async move {
        while let Some(event) = received.recv().await {
            if let (AiEvent::Approval { request_id, .. }, Some(session)) =
                (&event, approver.upgrade())
            {
                meanwhile();
                session.answer(*request_id, Answer::AllowOnce).await;
            }
        }
    });
    session
}

fn file(dir: &tempfile::TempDir, text: &str) -> (PathBuf, String) {
    let path = dir.path().join("app.conf");
    std::fs::write(&path, text).unwrap();
    let name = path.to_str().unwrap().to_owned();
    (path, name)
}

#[tokio::test]
async fn an_edit_is_made_on_the_file_as_it_is_after_the_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (path, name) = file(&dir, "port 80\nhost a\n");
    let other = path.clone();
    // Someone adds a line while the approval waits.
    let session = session(move || std::fs::write(&other, "port 80\nhost a\nlog on\n").unwrap());
    assert_eq!(
        session
            .edit_file(&name, "port 80", "port 8080", false)
            .await,
        Ok(1)
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "port 8080\nhost a\nlog on\n"
    );
}

#[tokio::test]
async fn an_edit_that_no_longer_applies_is_not_made() {
    let dir = tempfile::tempdir().unwrap();
    let (path, name) = file(&dir, "port 80\n");
    let other = path.clone();
    let session = session(move || std::fs::write(&other, "port 443\n").unwrap());
    assert!(
        session
            .edit_file(&name, "port 80", "port 8080", false)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "port 443\n");
}

#[tokio::test]
async fn a_whole_write_does_not_overwrite_a_change_made_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let (path, name) = file(&dir, "a\n");
    let other = path.clone();
    let session = session(move || std::fs::write(&other, "b\n").unwrap());
    assert!(session.write_file(&name, "c\n").await.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "b\n");

    // Unchanged meanwhile: written.
    let session = self::session(|| {});
    assert_eq!(session.write_file(&name, "c\n").await, Ok(()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "c\n");
}

#[tokio::test]
async fn an_incomplete_or_unreadable_snapshot_cannot_authorize_a_whole_write() {
    let dir = tempfile::tempdir().unwrap();
    let large = "a".repeat(rove_ai::host::Limits::default().file + 1);
    let (path, name) = file(&dir, &large);
    let session = session(|| {});
    assert!(session.write_file(&name, "replacement").await.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), large);
    std::fs::write(&path, [0u8, 42]).unwrap();
    assert!(session.write_file(&name, "replacement").await.is_err());
    assert_eq!(std::fs::read(path).unwrap(), [0, 42]);
}
