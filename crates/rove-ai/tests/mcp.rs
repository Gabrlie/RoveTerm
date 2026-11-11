//! The built-in MCP server over a bridge connection.

use std::sync::Arc;

use async_trait::async_trait;
use rove_ai::Answer;
use rove_ai::host::LocalHost;
use rove_ai::mcp::{ENV_ADDR, ENV_TOKEN, McpListener};
use rove_ai::session::{AiEvent, AiSession, SessionSettings, TerminalSource};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

struct Screen;

#[async_trait]
impl TerminalSource for Screen {
    async fn recent(&self, lines: usize) -> Option<String> {
        Some(format!("$ uptime ({lines} lines)\n"))
    }
}

struct Connection {
    lines: tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    next: u64,
}

impl Connection {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let message =
            json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params });
        self.writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        let reply: Value =
            serde_json::from_str(&self.lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(reply["id"], self.next);
        reply
    }

    async fn tool(&mut self, name: &str, arguments: Value) -> (String, bool) {
        let reply = self
            .call(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        let result = &reply["result"];
        (
            result["content"][0]["text"].as_str().unwrap().to_owned(),
            result["isError"].as_bool().unwrap(),
        )
    }
}

#[tokio::test]
async fn tools_run_through_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let (events, mut received) = mpsc::unbounded_channel::<AiEvent>();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    ));
    session.set_terminal_source(Arc::new(Screen));
    // Approve everything that is asked.
    let approver = Arc::downgrade(&session);
    tokio::spawn(async move {
        while let Some(event) = received.recv().await {
            if let (AiEvent::Approval { request_id, .. }, Some(session)) =
                (&event, approver.upgrade())
            {
                session.answer(*request_id, Answer::AllowOnce).await;
            }
        }
    });
    let listener = McpListener::bind().await.unwrap();
    let env = listener.bridge_env().unwrap();
    let addr = env
        .iter()
        .find(|(name, _)| name == ENV_ADDR)
        .unwrap()
        .1
        .clone();
    let token = env
        .iter()
        .find(|(name, _)| name == ENV_TOKEN)
        .unwrap()
        .1
        .clone();
    let server = listener.serve(session.clone());

    // A wrong token is refused.
    let mut intruder = TcpStream::connect(&addr).await.unwrap();
    intruder
        .write_all(b"guess\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let mut buffer = Vec::new();
    intruder.read_to_end(&mut buffer).await.unwrap();
    assert!(buffer.is_empty());

    let stream = TcpStream::connect(&addr).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(format!("{token}\n").as_bytes())
        .await
        .unwrap();
    let mut mcp = Connection {
        lines: BufReader::new(reader).lines(),
        writer,
        next: 0,
    };
    let init = mcp.call("initialize", json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } })).await;
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    let tools = mcp.call("tools/list", json!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "run_command",
            "read_file",
            "write_file",
            "edit_file",
            "list_dir",
            "terminal_snapshot"
        ]
    );

    let (output, failed) = mcp
        .tool(
            "run_command",
            json!({ "command": "echo hi", "cwd": dir.path() }),
        )
        .await;
    assert_eq!(
        (output.as_str(), failed),
        ("exit code: 0\nstdout:\nhi\n\n", false)
    );
    let path = dir.path().join("a.txt");
    let (_, failed) = mcp
        .tool("write_file", json!({ "path": path, "content": "x\ny\n" }))
        .await;
    assert!(!failed);
    assert_eq!(
        mcp.tool("read_file", json!({ "path": path, "line": 2 }))
            .await,
        ("y\n".to_owned(), false)
    );
    assert_eq!(
        mcp.tool("list_dir", json!({ "path": dir.path() })).await,
        ("a.txt\t4\n".to_owned(), false)
    );
    assert_eq!(
        mcp.tool("terminal_snapshot", json!({ "lines": 5 })).await,
        ("$ uptime (5 lines)\n".to_owned(), false)
    );
    let relative = mcp
        .call(
            "tools/call",
            json!({ "name": "read_file", "arguments": { "path": "a.txt" } }),
        )
        .await;
    assert!(relative.get("error").is_some());
    server.abort();
}
