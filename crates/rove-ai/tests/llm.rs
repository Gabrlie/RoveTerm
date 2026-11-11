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
        let _ = sender.send(exchange(stream, status, &body).await);
    });
    (base, receiver)
}

/// Serves one request per reply, in order; returns the base URL and what each asked.
async fn serve_each(
    replies: Vec<(u16, String)>,
) -> (String, tokio::sync::mpsc::UnboundedReceiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        for (status, body) in replies {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = sender.send(exchange(stream, status, &body).await);
        }
    });
    (base, receiver)
}

/// Reads one request from `stream` and answers it with `status` and `body`.
async fn exchange(stream: tokio::net::TcpStream, status: u16, body: &str) -> Seen {
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
    Seen {
        path,
        headers,
        body: serde_json::from_slice(&raw).unwrap_or(Value::Null),
    }
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
    let openai = json!({ "object": "list", "data": [{ "id": "gpt-5" }, { "id": "deepseek-chat", "context_length": 131072 }, { "id": "gpt-5" }] });
    let (base, seen) = serve_json(openai.to_string()).await;
    let models = client(Format::Chat, &base, "sk-list")
        .list_models()
        .await
        .unwrap();
    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["deepseek-chat", "gpt-5"]);
    assert_eq!((models[0].context, models[1].context), (Some(131072), None));
    let seen = seen.await.unwrap();
    assert_eq!(seen.path, "/v1/models");
    assert_eq!(seen.header("authorization"), Some("Bearer sk-list"));

    let anthropic = json!({ "data": [{ "id": "claude-opus-5-5", "type": "model", "max_input_tokens": 1000000 }], "has_more": false, "last_id": "claude-opus-5-5" });
    let (base, seen) = serve_json(anthropic.to_string()).await;
    let models = client(Format::Anthropic, &base, "ant")
        .list_models()
        .await
        .unwrap();
    assert_eq!(models[0].id, "claude-opus-5-5");
    assert_eq!(models[0].context, Some(1_000_000));
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

#[tokio::test]
async fn a_model_without_reasoning_settings_is_asked_again_without_them() {
    let refused = json!({ "error": { "message": "Unsupported parameter: 'reasoning_effort' is not supported with this model." } });
    let reply = || {
        sse(&[
            ("", json!({ "choices": [{ "delta": { "content": "ok" } }] })),
            (
                "",
                json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] }),
            ),
        ]) + "data: [DONE]\n\n"
    };
    let (base, mut seen) = serve_each(vec![
        (400, refused.to_string()),
        (200, reply()),
        (200, reply()),
    ])
    .await;
    let client = client(Format::Chat, &base, "k").with_effort(Some(rove_ai::llm::Effort::High));
    let messages = [Message::User { text: "hi".into() }];
    let reply = client
        .complete("s", &messages, &[], &mut |_| {})
        .await
        .unwrap();
    assert_eq!(reply.text, "ok");
    assert_eq!(seen.recv().await.unwrap().body["reasoning_effort"], "high");
    assert!(
        seen.recv()
            .await
            .unwrap()
            .body
            .get("reasoning_effort")
            .is_none()
    );
    assert!(client.effort_refused());
    // Later requests leave it out from the start.
    client
        .complete("s", &messages, &[], &mut |_| {})
        .await
        .unwrap();
    assert!(
        seen.recv()
            .await
            .unwrap()
            .body
            .get("reasoning_effort")
            .is_none()
    );
}

#[tokio::test]
async fn token_counts_are_read_in_each_format() {
    let messages = [Message::User { text: "hi".into() }];

    let chat = sse(&[
        ("", json!({ "choices": [{ "delta": { "content": "ok" } }] })),
        (
            "",
            json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] }),
        ),
        (
            "",
            json!({ "choices": [], "usage": { "prompt_tokens": 120, "completion_tokens": 7 } }),
        ),
    ]) + "data: [DONE]\n\n";
    let (base, seen) = serve(200, chat).await;
    let reply = client(Format::Chat, &base, "k")
        .complete("s", &messages, &[], &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        seen.await.unwrap().body["stream_options"]["include_usage"],
        true
    );
    assert_eq!(
        (reply.stats.input_tokens, reply.stats.output_tokens),
        (Some(120), Some(7))
    );
    assert!(reply.stats.first_token_ms.is_some());

    let responses = sse(&[
        ("response.output_text.delta", json!({ "delta": "ok" })),
        (
            "response.completed",
            json!({ "response": { "usage": { "input_tokens": 50, "output_tokens": 3 } } }),
        ),
    ]);
    let (base, _seen) = serve(200, responses).await;
    let reply = client(Format::Responses, &base, "k")
        .complete("s", &messages, &[], &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        (reply.stats.input_tokens, reply.stats.output_tokens),
        (Some(50), Some(3))
    );

    let anthropic = sse(&[
        (
            "message_start",
            json!({ "message": { "usage": { "input_tokens": 10, "cache_read_input_tokens": 90, "output_tokens": 1 } } }),
        ),
        (
            "content_block_start",
            json!({ "index": 0, "content_block": { "type": "text" } }),
        ),
        (
            "content_block_delta",
            json!({ "index": 0, "delta": { "type": "text_delta", "text": "ok" } }),
        ),
        (
            "message_delta",
            json!({ "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 12 } }),
        ),
        ("message_stop", json!({})),
    ]);
    let (base, _seen) = serve(200, anthropic).await;
    let reply = client(Format::Anthropic, &base, "k")
        .complete("s", &messages, &[], &mut |_| {})
        .await
        .unwrap();
    assert_eq!(
        (reply.stats.input_tokens, reply.stats.output_tokens),
        (Some(100), Some(12))
    );
}
