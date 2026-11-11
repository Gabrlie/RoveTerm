//! The reviewer in the review mode: what it is shown, and when its word replaces the user's.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use rove_ai::Answer;
use rove_ai::host::LocalHost;
use rove_ai::review::{ReviewRequest, Reviewer, Verdict};
use rove_ai::session::{AiEvent, AiSession, SessionSettings};
use tokio::sync::mpsc;

/// Allows everything with a fixed confidence, counting what it is shown.
struct Agreeable {
    confidence: f64,
    seen: Arc<AtomicUsize>,
}

#[async_trait]
impl Reviewer for Agreeable {
    fn name(&self) -> &'static str {
        "test"
    }

    async fn review(&self, _request: &ReviewRequest) -> Result<Verdict, String> {
        self.seen.fetch_add(1, Ordering::SeqCst);
        Ok(Verdict {
            allow: true,
            confidence: self.confidence,
        })
    }
}

/// A session in the review mode whose asked approvals are denied; returns it, how many
/// approvals were asked, and the reviewer's count.
fn session(confidence: f64) -> (Arc<AiSession>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let (events, mut received) = mpsc::unbounded_channel::<AiEvent>();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    ));
    let seen = Arc::new(AtomicUsize::new(0));
    session.set_reviewer(Some((
        Arc::new(Agreeable {
            confidence,
            seen: seen.clone(),
        }),
        0.8,
    )));
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let denier = Arc::downgrade(&session);
    tokio::spawn(async move {
        while let Some(event) = received.recv().await {
            if let (AiEvent::Approval { request_id, .. }, Some(session)) =
                (&event, denier.upgrade())
            {
                counter.fetch_add(1, Ordering::SeqCst);
                session.answer(*request_id, Answer::Deny).await;
            }
        }
    });
    (session, asked, seen)
}

#[tokio::test]
async fn a_sure_reviewer_lets_ordinary_changes_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.conf");
    let path = path.to_str().unwrap();
    let (session, asked, seen) = session(0.9);
    session.write_file(path, "port 80\n").await.unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "port 80\n");
    assert_eq!(
        (asked.load(Ordering::SeqCst), seen.load(Ordering::SeqCst)),
        (0, 1)
    );

    // Dangerous operations always go to the user; the reviewer is not asked.
    assert!(session.run_command("rm -rf /", None, None).await.is_err());
    assert_eq!(
        (asked.load(Ordering::SeqCst), seen.load(Ordering::SeqCst)),
        (1, 1)
    );
}

#[tokio::test]
async fn an_unsure_reviewer_leaves_it_to_the_user() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.conf");
    let (session, asked, seen) = session(0.5);
    assert!(
        session
            .write_file(path.to_str().unwrap(), "x")
            .await
            .is_err()
    );
    assert!(!path.exists());
    assert_eq!(
        (asked.load(Ordering::SeqCst), seen.load(Ordering::SeqCst)),
        (1, 1)
    );
}

#[tokio::test]
async fn jev_is_asked_a_choice_and_its_confidence_is_kept() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let (mut length, mut auth) = (0usize, String::new());
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let line = line.trim_end().to_owned();
            if line.is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            if lower.starts_with("authorization:") {
                auth = line["authorization:".len()..].trim().to_owned();
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).await.unwrap();
        let reply = r#"{"model":"jev-1.13.0","answers":{"decision":{"type":"choice","choice":"allow","confidence":0.91,"probabilities":{"allow":0.93,"ask":0.07}}},"usage":{"input_tokens":50,"output_tokens":0}}"#;
        write
            .write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).as_bytes())
            .await
            .unwrap();
        (
            auth,
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        )
    });
    let reviewer = rove_ai::review::JevReviewer::new(&url, "jk").unwrap();
    let request = ReviewRequest {
        host: "web".into(),
        operation: "run the shell command: systemctl restart nginx".into(),
        reasons: vec!["unknown_program: systemctl".into()],
    };
    let verdict = reviewer.review(&request).await.unwrap();
    assert!(verdict.allow);
    assert!((verdict.confidence - 0.91).abs() < 1e-9);
    let (auth, body) = server.await.unwrap();
    assert_eq!(auth, "Bearer jk");
    assert_eq!(body["model"], "jev-latest");
    assert_eq!(body["questions"]["decision"]["type"], "choice");
    assert!(
        body["state"]
            .as_str()
            .unwrap()
            .contains("systemctl restart nginx")
    );
}
