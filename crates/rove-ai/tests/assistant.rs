//! The built-in assistant against a scripted model server (Chat Completions format).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rove_ai::assistant::{Assistant, HostMeta};
use rove_ai::host::LocalHost;
use rove_ai::llm::{Client, Endpoint, Format};
use rove_ai::session::{AiEvent, AiSession, Outcome, SessionSettings};
use rove_ai::{Answer, Risk};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Serves the scripted replies in order, one per request; requests after the script never get
/// an answer. Returns the base URL and the request bodies seen.
async fn model(replies: Vec<String>) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let bodies = seen.clone();
    tokio::spawn(async move {
        let mut replies = replies.into_iter();
        while let Ok((stream, _)) = listener.accept().await {
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            let mut length = 0;
            let mut request = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                request = true;
                let line = line.trim_end().to_ascii_lowercase();
                if line.is_empty() {
                    break;
                }
                if let Some(value) = line.strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            // A connection the client opened and closed again without a request.
            if !request {
                continue;
            }
            let mut raw = vec![0u8; length];
            reader.read_exact(&mut raw).await.unwrap();
            bodies
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&raw).unwrap());
            let Some(body) = replies.next() else {
                // Hold the connection open: the model "thinks" until the turn is cancelled.
                tokio::spawn(async move {
                    let _keep = (reader, write);
                    tokio::time::sleep(Duration::from_secs(60)).await;
                });
                continue;
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            write.write_all(head.as_bytes()).await.unwrap();
            write.write_all(body.as_bytes()).await.unwrap();
        }
    });
    (base, seen)
}

fn call(id: &str, name: &str, arguments: Value) -> String {
    let chunk = json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": id, "function": { "name": name, "arguments": arguments.to_string() } }] }, "finish_reason": "tool_calls" }] });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

fn say(text: &str) -> String {
    let chunk = json!({ "choices": [{ "delta": { "content": text }, "finish_reason": "stop" }] });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

fn assistant(base: &str, answer: Answer) -> (Arc<Assistant>, mpsc::UnboundedReceiver<AiEvent>) {
    let (events, received) = mpsc::unbounded_channel();
    let (approvals, mut asked) = mpsc::unbounded_channel::<u64>();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(move |event| {
            if let AiEvent::Approval { request_id, .. } = &event {
                let _ = approvals.send(*request_id);
            }
            let _ = events.send(event);
        }),
    ));
    let answering = Arc::downgrade(&session);
    tokio::spawn(async move {
        while let Some(request_id) = asked.recv().await {
            if let Some(session) = answering.upgrade() {
                session.answer(request_id, answer).await;
            }
        }
    });
    let endpoint = Endpoint {
        format: Format::Chat,
        base_url: base.to_owned(),
        model: "m".into(),
        max_tokens: None,
        effort: None,
    };
    let client = Client::new(endpoint, "k", Duration::from_secs(10)).unwrap();
    let meta = HostMeta {
        name: "test".into(),
        address: "local".into(),
        system: Some("Linux".into()),
        notes: None,
    };
    (Arc::new(Assistant::new(client, session, &meta)), received)
}

fn drain(received: &mut mpsc::UnboundedReceiver<AiEvent>) -> Vec<AiEvent> {
    let mut events = Vec::new();
    while let Ok(event) = received.try_recv() {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn a_turn_runs_tools_through_the_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    let (base, seen) = model(vec![
        call("c1", "run_command", json!({ "command": "echo hi" })),
        call("c2", "write_file", json!({ "path": path, "content": "x" })),
        say("Done."),
    ])
    .await;
    let (assistant, mut received) = assistant(&base, Answer::Deny);
    assert_eq!(
        assistant.prompt("say hi then write a note").await.unwrap(),
        "end_turn"
    );
    assert!(!path.exists(), "the write was refused");

    let events = drain(&mut received);
    let outcomes: Vec<(Risk, &Outcome)> = events
        .iter()
        .filter_map(|event| match event {
            AiEvent::Executed {
                assessment,
                outcome,
                ..
            } => Some((assessment.risk, outcome)),
            _ => None,
        })
        .collect();
    assert!(
        matches!(outcomes[0], (Risk::ReadOnly, Outcome::Command { output }) if output.stdout == "hi\n")
    );
    assert!(matches!(outcomes[1], (Risk::Modify, Outcome::Denied)));
    assert!(events.iter().any(|event| matches!(event, AiEvent::Update { update: rove_ai::acp::SessionUpdate::Message { text, .. } } if text == "Done.")));

    let bodies = seen.lock().unwrap();
    assert_eq!(bodies.len(), 3);
    let system = bodies[0]["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("untrusted-data") && system.contains("Linux"));
    assert_eq!(bodies[0]["tools"].as_array().unwrap().len(), 6);
    // The model saw the command's output, marked as data, and then the refusal.
    let first_result = bodies[1]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        first_result.contains("<untrusted-data source=\"tool:run_command\">")
            && first_result.contains("hi")
    );
    let second_result = bodies[2]["messages"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(second_result.contains("did not allow"));
}

#[tokio::test]
async fn cancelling_stops_the_turn_and_keeps_the_conversation_valid() {
    // The first reply asks for a write; the user never answers, and the turn is cancelled.
    let (base, seen) = model(vec![
        call(
            "c1",
            "write_file",
            json!({ "path": "/tmp/rove-never", "content": "x" }),
        ),
        say("Okay."),
    ])
    .await;
    let (events, mut received) = mpsc::unbounded_channel();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    ));
    let endpoint = Endpoint {
        format: Format::Chat,
        base_url: base,
        model: "m".into(),
        max_tokens: None,
        effort: None,
    };
    let client = Client::new(endpoint, "k", Duration::from_secs(10)).unwrap();
    let assistant = Arc::new(Assistant::new(client, session, &HostMeta::default()));
    let turn = {
        let assistant = assistant.clone();
        tokio::spawn(async move { assistant.prompt("write it").await })
    };
    // Wait for the approval request, then cancel.
    loop {
        if let Some(AiEvent::Approval { .. }) = received.recv().await {
            break;
        }
    }
    assistant.cancel().await;
    assert_eq!(turn.await.unwrap().unwrap(), "cancelled");
    assert!(!std::path::Path::new("/tmp/rove-never").exists());

    // The next turn sends a result for the cancelled call, so the provider accepts it.
    assert_eq!(assistant.prompt("never mind").await.unwrap(), "end_turn");
    let bodies = seen.lock().unwrap();
    let messages = bodies[1]["messages"].as_array().unwrap();
    let tool = messages
        .iter()
        .find(|message| message["role"] == "tool")
        .unwrap();
    assert_eq!(tool["tool_call_id"], "c1");
    assert_eq!(messages.last().unwrap()["content"], "never mind");
}

#[tokio::test]
async fn switching_between_providers_with_the_same_model_keeps_tool_history() {
    let (first, first_seen) = model(vec![
        call("c1", "run_command", json!({"command":"echo first-result"})),
        say("first reply"),
    ])
    .await;
    let (next, next_seen) = model(vec![say("second reply")]).await;
    let (mut assistant, _events) = assistant(&first, Answer::AllowOnce);
    assistant.prompt("first question").await.unwrap();
    let client = Client::new(
        Endpoint {
            format: Format::Chat,
            base_url: next,
            model: "m".into(),
            effort: None,
            max_tokens: None,
        },
        "other",
        Duration::from_secs(5),
    )
    .unwrap();
    Arc::get_mut(&mut assistant).unwrap().retarget(
        client,
        vec!["m".into()],
        Default::default(),
        None,
        true,
    );
    assistant.prompt("second question").await.unwrap();
    assert_eq!(first_seen.lock().unwrap().len(), 2);
    let seen = next_seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let messages = seen[0]["messages"].to_string();
    for text in [
        "first question",
        "first-result",
        "first reply",
        "second question",
    ] {
        assert!(messages.contains(text));
    }
}
