//! The model client against a scripted HTTP server, in each wire format.

use std::time::Duration;

use rove_ai::llm::{Client, Delta, Endpoint, Format, LlmError, Message, ToolSpec};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// What the server saw.
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Serves one request with `status` and `body`; returns the base URL and what was asked.
async fn serve(status: u16, body: String) -> (String, oneshot::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let mut headers = Vec::new();
        let mut length = 0;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).await.unwrap();
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            let (name, value) = header.split_once(':').unwrap();
            let (name, value) = (name.to_ascii_lowercase(), value.trim().to_owned());
            if name == "content-length" {
                length = value.parse().unwrap();
            }
            headers.push((name, value));
        }
        let mut raw = vec![0u8; length];
        reader.read_exact(&mut raw).await.unwrap();
        let _ = sender.send(Seen {
            path,
            headers,
            body: serde_json::from_slice(&raw).unwrap(),
        });
        let kind = if status == 200 {
            "text/event-stream"
        } else {
            "application/json"
        };
        let head = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        write.write_all(head.as_bytes()).await.unwrap();
        write.write_all(body.as_bytes()).await.unwrap();
    });
    (base, receiver)
}

fn client(format: Format, base: &str, key: &str) -> Client {
    let endpoint = Endpoint {
        format,
        base_url: base.to_owned(),
        model: "test-model".into(),
        max_tokens: None,
        effort: None,
    };
    Client::new(endpoint, key, Duration::from_secs(5)).unwrap()
}

fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "run_command".into(),
        description: "Run".into(),
        parameters: json!({ "type": "object" }),
    }]
}

fn sse(events: &[(&str, Value)]) -> String {
    events
        .iter()
        .map(|(event, data)| {
            if event.is_empty() {
                format!("data: {data}\n\n")
            } else {
                format!("event: {event}\ndata: {data}\n\n")
            }
        })
        .collect()
}

async fn run(
    client: &Client,
    messages: &[Message],
) -> (Result<rove_ai::llm::Reply, LlmError>, Vec<Delta>) {
    let mut deltas = Vec::new();
    let reply = client
        .complete("be brief", messages, &tools(), &mut |delta| {
            deltas.push(delta)
        })
        .await;
    (reply, deltas)
}

#[tokio::test]
async fn chat_completions_stream_text_reasoning_and_tool_calls() {
    let mut body = sse(&[
        (
            "",
            json!({ "choices": [{ "delta": { "reasoning_content": "Let me" } }] }),
        ),
        (
            "",
            json!({ "choices": [{ "delta": { "content": "Checking" } }] }),
        ),
        (
            "",
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "c1", "function": { "name": "run_command", "arguments": "{\"comm" } }] } }] }),
        ),
        (
            "",
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "and\":\"ls\"}" } }] }, "finish_reason": "tool_calls" }] }),
        ),
    ]);
    body.push_str("data: [DONE]\n\n");
    let (base, seen) = serve(200, body).await;
    let messages = vec![
        Message::User {
            text: "list".into(),
        },
        Message::Assistant {
            text: String::new(),
            tool_calls: vec![rove_ai::llm::ToolCall {
                id: "c0".into(),
                name: "run_command".into(),
                arguments: "{}".into(),
            }],
            opaque: None,
        },
        Message::Tool {
            call_id: "c0".into(),
            name: "run_command".into(),
            output: "ok".into(),
            is_error: false,
        },
    ];
    let (reply, deltas) = run(&client(Format::Chat, &base, "sk-secret"), &messages).await;
    let reply = reply.unwrap();
    assert_eq!(reply.text, "Checking");
    assert_eq!(reply.tool_calls[0].arguments, "{\"command\":\"ls\"}");
    assert_eq!(reply.stop, "tool_calls");
    assert_eq!(
        deltas,
        [
            Delta::Thought("Let me".into()),
            Delta::Text("Checking".into())
        ]
    );
    let seen = seen.await.unwrap();
    assert_eq!(seen.path, "/v1/chat/completions");
    assert_eq!(seen.header("authorization"), Some("Bearer sk-secret"));
    assert_eq!(seen.body["messages"][0]["role"], "system");
    assert_eq!(seen.body["messages"][2]["tool_calls"][0]["id"], "c0");
    assert_eq!(seen.body["messages"][3]["tool_call_id"], "c0");
    assert_eq!(seen.body["tools"][0]["function"]["name"], "run_command");
}

#[tokio::test]
async fn responses_stream_text_and_function_calls() {
    let body = sse(&[
        (
            "response.reasoning_summary_text.delta",
            json!({ "delta": "Plan" }),
        ),
        ("response.output_text.delta", json!({ "delta": "On it" })),
        (
            "response.output_item.done",
            json!({ "item": { "type": "function_call", "call_id": "f1", "name": "run_command", "arguments": "{\"command\":\"uptime\"}" } }),
        ),
        (
            "response.completed",
            json!({ "response": { "status": "completed" } }),
        ),
    ]);
    let (base, seen) = serve(200, body).await;
    let (reply, deltas) = run(
        &client(Format::Responses, &base, "k-123"),
        &[Message::User { text: "up?".into() }],
    )
    .await;
    let reply = reply.unwrap();
    assert_eq!(reply.text, "On it");
    assert_eq!(reply.tool_calls[0].id, "f1");
    assert_eq!(deltas[0], Delta::Thought("Plan".into()));
    let seen = seen.await.unwrap();
    assert_eq!(seen.path, "/v1/responses");
    assert_eq!(seen.body["instructions"], "be brief");
    assert_eq!(seen.body["store"], false);
    assert_eq!(seen.body["tools"][0]["name"], "run_command");
}

#[tokio::test]
async fn anthropic_streams_thinking_text_and_tool_use() {
    let body = sse(&[
        (
            "message_start",
            json!({ "type": "message_start", "message": {} }),
        ),
        (
            "content_block_start",
            json!({ "index": 0, "content_block": { "type": "thinking", "thinking": "" } }),
        ),
        (
            "content_block_delta",
            json!({ "index": 0, "delta": { "type": "thinking_delta", "thinking": "Hmm" } }),
        ),
        (
            "content_block_delta",
            json!({ "index": 0, "delta": { "type": "signature_delta", "signature": "sig" } }),
        ),
        (
            "content_block_start",
            json!({ "index": 1, "content_block": { "type": "text", "text": "" } }),
        ),
        (
            "content_block_delta",
            json!({ "index": 1, "delta": { "type": "text_delta", "text": "Sure" } }),
        ),
        (
            "content_block_start",
            json!({ "index": 2, "content_block": { "type": "tool_use", "id": "t1", "name": "run_command", "input": {} } }),
        ),
        (
            "content_block_delta",
            json!({ "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"command\":" } }),
        ),
        (
            "content_block_delta",
            json!({ "index": 2, "delta": { "type": "input_json_delta", "partial_json": "\"df -h\"}" } }),
        ),
        (
            "message_delta",
            json!({ "delta": { "stop_reason": "tool_use" } }),
        ),
        ("message_stop", json!({ "type": "message_stop" })),
    ]);
    let (base, seen) = serve(200, body).await;
    let (reply, _) = run(
        &client(Format::Anthropic, &base, "ant-key"),
        &[Message::User {
            text: "disk?".into(),
        }],
    )
    .await;
    let reply = reply.unwrap();
    assert_eq!(reply.text, "Sure");
    assert_eq!(reply.tool_calls[0].arguments, "{\"command\":\"df -h\"}");
    assert_eq!(reply.stop, "tool_use");
    assert_eq!(
        reply.opaque,
        Some(json!([{ "type": "thinking", "thinking": "Hmm", "signature": "sig" }]))
    );
    let seen = seen.await.unwrap();
    assert_eq!(seen.path, "/v1/messages");
    assert_eq!(seen.header("x-api-key"), Some("ant-key"));
    assert_eq!(seen.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(seen.body["max_tokens"], 32000);
    assert_eq!(seen.body["tools"][0]["input_schema"]["type"], "object");
}

#[tokio::test]
async fn errors_never_quote_the_key() {
    let (base, _) = serve(
        401,
        json!({ "error": { "message": "Incorrect API key provided: sk-very-secret-key" } })
            .to_string(),
    )
    .await;
    let (reply, _) = run(
        &client(Format::Chat, &base, "sk-very-secret-key"),
        &[Message::User { text: "hi".into() }],
    )
    .await;
    match reply {
        Err(LlmError::Http { status, message }) => {
            assert_eq!(status, 401);
            assert!(!message.contains("sk-very-secret-key"));
            assert!(message.contains("***"));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_reply_cut_short_is_an_error() {
    let (base, _) = serve(
        200,
        sse(&[(
            "",
            json!({ "choices": [{ "delta": { "content": "par" } }] }),
        )]),
    )
    .await;
    let (reply, _) = run(
        &client(Format::Chat, &base, "k"),
        &[Message::User { text: "hi".into() }],
    )
    .await;
    assert!(matches!(reply, Err(LlmError::Protocol(_))));
}

#[tokio::test]
async fn model_lists_are_read_in_both_shapes() {
    // `serve` answers any method; the list is a plain JSON body with status 200.
    let openai = json!({ "object": "list", "data": [{ "id": "gpt-5" }, { "id": "deepseek-chat" }, { "id": "gpt-5" }] });
    let (base, seen) = serve_json(openai.to_string()).await;
    let models = client(Format::Chat, &base, "sk-list")
        .list_models()
        .await
        .unwrap();
    assert_eq!(models, ["deepseek-chat", "gpt-5"]);
    let seen = seen.await.unwrap();
    assert_eq!(seen.path, "/v1/models");
    assert_eq!(seen.header("authorization"), Some("Bearer sk-list"));

    let anthropic = json!({ "data": [{ "id": "claude-opus-5-5", "type": "model" }], "has_more": false, "last_id": "claude-opus-5-5" });
    let (base, seen) = serve_json(anthropic.to_string()).await;
    let models = client(Format::Anthropic, &base, "ant")
        .list_models()
        .await
        .unwrap();
    assert_eq!(models, ["claude-opus-5-5"]);
    let seen = seen.await.unwrap();
    assert_eq!(seen.path, "/v1/models?limit=1000");
    assert_eq!(seen.header("x-api-key"), Some("ant"));
    assert_eq!(seen.header("anthropic-version"), Some("2023-06-01"));
}

/// Serves one GET with a JSON `body`; returns the base URL and what was asked.
async fn serve_json(body: String) -> (String, oneshot::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).await.unwrap();
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            let (name, value) = header.split_once(':').unwrap();
            headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let _ = sender.send(Seen {
            path,
            headers,
            body: Value::Null,
        });
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        write.write_all(head.as_bytes()).await.unwrap();
        write.write_all(body.as_bytes()).await.unwrap();
    });
    (base, receiver)
}
