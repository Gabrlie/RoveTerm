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

fn durable_assistant(
    base: &str,
    sink: rove_ai::assistant::CheckpointSink,
    toolbox: Option<Arc<dyn rove_ai::mcp::McpTools>>,
) -> Assistant {
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(|_| {}),
    ));
    let client = Client::new(
        Endpoint {
            format: Format::Chat,
            base_url: base.into(),
            model: "m".into(),
            effort: None,
            max_tokens: None,
        },
        "",
        Duration::from_secs(5),
    )
    .unwrap();
    let assistant = match toolbox {
        Some(toolbox) => Assistant::with_toolbox(client, session, "test worker".into(), toolbox),
        None => Assistant::new(client, session, &HostMeta::default()),
    };
    assistant.with_checkpoint_sink(sink)
}

#[tokio::test]
async fn durable_checkpoints_restore_complete_tool_history_and_turn_numbering() {
    let (base, _) = model(vec![
        call(
            "c1",
            "run_command",
            json!({"command":"echo preserved-result"}),
        ),
        say("first complete"),
    ])
    .await;
    let saved = Arc::new(Mutex::new(Vec::new()));
    let records = saved.clone();
    let assistant = durable_assistant(
        &base,
        Arc::new(move |checkpoint| {
            records.lock().unwrap().push(checkpoint);
            Ok(())
        }),
        None,
    );
    assistant.prompt("inspect it").await.unwrap();
    let checkpoint = assistant.checkpoint().await.unwrap();
    assert_eq!(checkpoint.turns, 1);
    assert!(checkpoint.pending_tool.is_none());
    assert!(checkpoint.messages.iter().any(|message| matches!(message,
        rove_ai::llm::Message::Tool { output, .. } if output.contains("preserved-result"))));
    {
        let saved = saved.lock().unwrap();
        assert!(
            saved
                .iter()
                .any(|checkpoint| checkpoint.pending_tool.as_deref() == Some("c1"))
        );
        assert_eq!(saved.last().unwrap(), &checkpoint);
    }

    let (base, seen) = model(vec![say("continued")]).await;
    let restored = durable_assistant(&base, Arc::new(|_| Ok(())), None);
    let encoded = serde_json::to_vec(&checkpoint).unwrap();
    restored
        .restore_checkpoint(serde_json::from_slice(&encoded).unwrap())
        .await
        .unwrap();
    restored.prompt("follow up").await.unwrap();
    assert_eq!(restored.checkpoint().await.unwrap().turns, 2);
    let messages = seen.lock().unwrap()[0]["messages"].to_string();
    assert!(messages.contains("preserved-result"));
    assert!(messages.contains("first complete"));
    assert!(messages.contains("follow up"));
}

#[tokio::test]
async fn a_failed_initial_checkpoint_prevents_the_model_request() {
    let (base, seen) = model(vec![say("must not run")]).await;
    let assistant = durable_assistant(&base, Arc::new(|_| Err("storage unavailable".into())), None);
    assert!(matches!(
        assistant.prompt("inspect it").await,
        Err(rove_ai::assistant::AssistantError::Checkpoint(_))
    ));
    assert!(seen.lock().unwrap().is_empty());
}

struct CountTools(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait::async_trait]
impl rove_ai::mcp::McpTools for CountTools {
    fn instructions(&self) -> String {
        "test counter".into()
    }
    fn specs(&self) -> Value {
        json!([{"name":"count","description":"count one execution","inputSchema":{"type":"object"}}])
    }
    async fn call(
        &self,
        _name: &str,
        _arguments: &Value,
    ) -> Result<rove_ai::tools::ToolResult, (i64, String)> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(rove_ai::tools::ToolResult {
            text: "counted".into(),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn a_failed_tool_intent_checkpoint_prevents_the_tool() {
    let (base, seen) = model(vec![call("c1", "count", json!({}))]).await;
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let assistant = durable_assistant(
        &base,
        Arc::new(|checkpoint| {
            if checkpoint.pending_tool.is_some() {
                Err("storage unavailable".into())
            } else {
                Ok(())
            }
        }),
        Some(Arc::new(CountTools(count.clone()))),
    );
    assert!(matches!(
        assistant.prompt("count").await,
        Err(rove_ai::assistant::AssistantError::Checkpoint(_))
    ));
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_result_checkpoint_keeps_the_interrupted_marker_and_never_replays() {
    let (base, seen) = model(vec![
        call("c1", "count", json!({})),
        say("must not request"),
    ])
    .await;
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let assistant = durable_assistant(
        &base,
        Arc::new(|checkpoint| {
            if matches!(
                checkpoint.messages.last(),
                Some(rove_ai::llm::Message::Tool { .. })
            ) {
                Err("result storage unavailable".into())
            } else {
                Ok(())
            }
        }),
        Some(Arc::new(CountTools(count.clone()))),
    );
    assert!(assistant.prompt("count").await.is_err());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    let checkpoint = assistant.checkpoint().await.unwrap();
    assert_eq!(checkpoint.pending_tool.as_deref(), Some("c1"));
    assert!(assistant.prompt("continue").await.is_err());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn restore_rejects_versions_and_unresolved_effects_and_clears_incompatible_opaque_content() {
    let (base, seen) = model(Vec::new()).await;
    let assistant = durable_assistant(&base, Arc::new(|_| Ok(())), None);
    let mut checkpoint = assistant.checkpoint().await.unwrap();
    checkpoint.version += 1;
    assert!(assistant.restore_checkpoint(checkpoint).await.is_err());
    let mut checkpoint = assistant.checkpoint().await.unwrap();
    checkpoint.messages.push(rove_ai::llm::Message::Assistant {
        text: "write requested".into(),
        tool_calls: vec![rove_ai::llm::ToolCall {
            id: "c1".into(),
            name: "count".into(),
            arguments: "{}".into(),
        }],
        opaque: Some(json!({"signed":"provider content"})),
    });
    checkpoint.pending_tool = Some("c1".into());
    assert!(
        assistant
            .restore_checkpoint(checkpoint.clone())
            .await
            .is_err()
    );
    checkpoint.resolve_interrupted();
    checkpoint.format = Format::Anthropic;
    assistant.restore_checkpoint(checkpoint).await.unwrap();
    let restored = assistant.checkpoint().await.unwrap();
    assert!(restored.pending_tool.is_none());
    assert!(matches!(
        &restored.messages[0],
        rove_ai::llm::Message::Assistant { opaque: None, .. }
    ));
    assert!(
        matches!(&restored.messages[1], rove_ai::llm::Message::Tool { output,is_error:true,.. } if output.contains("unknown"))
    );
    assert!(seen.lock().unwrap().is_empty());
}
