//! A whole ACP turn against a scripted fake agent, over in-memory pipes.

use std::sync::Arc;

use rove_ai::acp;
use rove_ai::host::LocalHost;
use rove_ai::policy::{Operation, ToolKind};
use rove_ai::runner::{self, AgentSpec};
use rove_ai::session::{AiEvent, AiSession, Outcome, SessionSettings};
use rove_ai::{Answer, Risk};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};
use tokio::sync::mpsc;

/// The agent's end of the pipes.
struct FakeAgent {
    lines: Lines<BufReader<DuplexStream>>,
    out: DuplexStream,
    next_id: u64,
}

impl FakeAgent {
    async fn send(&mut self, message: Value) {
        self.out
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn receive(&mut self) -> Value {
        let line = self
            .lines
            .next_line()
            .await
            .unwrap()
            .expect("client hung up");
        serde_json::from_str(&line).unwrap()
    }

    /// Answers the client's next request, which must be `method`, with `result`.
    async fn answer(&mut self, method: &str, result: Value) -> Value {
        let request = self.receive().await;
        assert_eq!(request["method"], method, "{request}");
        self.send(json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }))
            .await;
        request
    }

    /// Calls `method` on the client and returns its response.
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        let response = self.receive().await;
        assert_eq!(response["id"], id, "{response}");
        response
    }

    async fn notify(&mut self, update: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s1", "update": update } })).await;
    }
}

#[tokio::test]
async fn a_turn_goes_through_the_policy() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "one\ntwo\nthree\n").unwrap();
    let notes = notes.to_str().unwrap().to_owned();

    // Events from the session; approvals are answered as they come: dangerous ones refused.
    let (events, mut received) = mpsc::unbounded_channel::<AiEvent>();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    ));
    let (client_side, agent_side) = (tokio::io::duplex(1 << 16), tokio::io::duplex(1 << 16));
    let (client_read, agent_write) = (client_side.0, client_side.1);
    let (agent_read, client_write) = (agent_side.0, agent_side.1);
    let mut fake = FakeAgent {
        lines: BufReader::new(agent_read).lines(),
        out: agent_write,
        next_id: 100,
    };

    let script = tokio::spawn(async move {
        let init = fake.answer("initialize", json!({ "protocolVersion": 1, "agentInfo": { "name": "fake", "version": "0.1" }, "authMethods": [] })).await;
        assert_eq!(init["params"]["clientCapabilities"]["terminal"], true);
        // The first session/new needs sign-in.
        let request = fake.receive().await;
        assert_eq!(request["method"], "session/new");
        fake.send(json!({ "jsonrpc": "2.0", "id": request["id"], "error": { "code": -32000, "message": "Authentication required" } })).await;
        let auth = fake.answer("authenticate", json!({})).await;
        assert_eq!(auth["params"]["methodId"], "api-key");
        let new = fake
            .answer("session/new", json!({ "sessionId": "s1", "modes": { "currentModeId": "auto", "availableModes": [{ "id": "read-only", "name": "Read only" }, { "id": "auto", "name": "Auto" }] } }))
            .await;
        assert_eq!(new["params"]["mcpServers"], json!([]));
        let mode = fake.answer("session/set_mode", json!({})).await;
        assert_eq!(mode["params"]["modeId"], "read-only");

        let prompt = fake.receive().await;
        assert_eq!(prompt["method"], "session/prompt");
        assert_eq!(prompt["params"]["prompt"][0]["text"], "check the notes");
        fake.notify(json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Looking" } })).await;

        // Reading is allowed by policy; lines 2-3 only.
        let read = fake
            .call(
                "fs/read_text_file",
                json!({ "sessionId": "s1", "path": notes, "line": 2, "limit": 2 }),
            )
            .await;
        assert_eq!(read["result"]["content"], "two\nthree\n");
        // A read-only command runs at once.
        let created = fake
            .call(
                "terminal/create",
                json!({ "sessionId": "s1", "command": "echo", "args": ["hi there"] }),
            )
            .await;
        let terminal = created["result"]["terminalId"].clone();
        let exit = fake
            .call(
                "terminal/wait_for_exit",
                json!({ "sessionId": "s1", "terminalId": terminal }),
            )
            .await;
        assert_eq!(exit["result"]["exitCode"], 0);
        let output = fake
            .call(
                "terminal/output",
                json!({ "sessionId": "s1", "terminalId": terminal }),
            )
            .await;
        assert_eq!(output["result"]["output"], "hi there\n");
        // Writing asks; the test allows it.
        let write = fake
            .call(
                "fs/write_text_file",
                json!({ "sessionId": "s1", "path": notes, "content": "updated\n" }),
            )
            .await;
        assert!(write.get("error").is_none(), "{write}");
        // The agent's own dangerous tool, reported first and then asked about by id only, is
        // refused with its reject option.
        fake.notify(json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Run", "kind": "execute", "status": "pending", "rawInput": { "command": ["bash", "-lc", "rm -rf ~"] }, "content": [{ "type": "content", "content": { "type": "text", "text": "Clean up the home directory" } }] })).await;
        let permission = fake
            .call("session/request_permission", json!({
                "sessionId": "s1",
                "toolCall": { "toolCallId": "t1" },
                "options": [{ "optionId": "yes", "name": "Allow", "kind": "allow_always" }, { "optionId": "no", "name": "Reject", "kind": "reject_once" }],
            }))
            .await;
        assert_eq!(
            permission["result"]["outcome"],
            json!({ "outcome": "selected", "optionId": "no" })
        );
        // Codex asking before a call to RoveTerm's own MCP tools is allowed without asking: the
        // tool goes through the policy itself.
        let own = fake
            .call("session/request_permission", json!({
                "sessionId": "s1",
                "toolCall": { "toolCallId": "mcp-1", "title": "Approve run_command", "rawInput": { "server_name": "roveterm", "id": 1 } },
                "options": [{ "optionId": "approved", "name": "Allow", "kind": "allow_once" }, { "optionId": "cancel", "name": "Cancel", "kind": "reject_once" }],
            }))
            .await;
        assert_eq!(
            own["result"]["outcome"],
            json!({ "outcome": "selected", "optionId": "approved" })
        );
        // Paths must be absolute; requests for other sessions are refused.
        let relative = fake
            .call(
                "fs/read_text_file",
                json!({ "sessionId": "s1", "path": "notes.txt" }),
            )
            .await;
        assert!(relative.get("error").is_some());
        let other = fake
            .call(
                "fs/read_text_file",
                json!({ "sessionId": "s2", "path": "/etc/hostname" }),
            )
            .await;
        assert!(other.get("error").is_some());
        fake.send(
            json!({ "jsonrpc": "2.0", "id": prompt["id"], "result": { "stopReason": "end_turn" } }),
        )
        .await;
        fake
    });

    let agent = acp::connect(client_read, client_write, session.clone())
        .await
        .unwrap();
    assert_eq!(agent.info().name.as_deref(), Some("fake"));
    let spec = AgentSpec {
        cwd: dir.path().to_path_buf(),
        auth_methods: vec!["api-key".into()],
        preferred_modes: vec!["read-only".into()],
        ..AgentSpec::default()
    };
    let running = Arc::new(runner::open(agent, session.clone(), &spec).await.unwrap());
    assert_eq!(running.started().current_mode.as_deref(), Some("read-only"));

    let answering = {
        // Weak: the channel closes once the session is gone.
        let session = Arc::downgrade(&session);
        tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(event) = received.recv().await {
                if let AiEvent::Approval {
                    request_id,
                    assessment,
                    ..
                } = &event
                {
                    let answer = if assessment.risk == Risk::Dangerous {
                        Answer::Deny
                    } else {
                        Answer::AllowOnce
                    };
                    let session = session.upgrade().unwrap();
                    assert!(session.answer(*request_id, answer).await);
                }
                seen.push(event);
            }
            seen
        })
    };
    assert_eq!(running.prompt("check the notes").await.unwrap(), "end_turn");
    let fake = script.await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.txt")).unwrap(),
        "updated\n"
    );

    // Closing the pipes ends the reader, which holds the last references to the session.
    drop(fake);
    drop(running);
    drop(session);
    let seen = tokio::time::timeout(std::time::Duration::from_secs(5), answering)
        .await
        .unwrap()
        .unwrap();
    let approvals: Vec<&Operation> = seen
        .iter()
        .filter_map(|event| match event {
            AiEvent::Approval { operation, .. } => Some(operation),
            _ => None,
        })
        .collect();
    assert_eq!(
        approvals.len(),
        2,
        "the write and the dangerous tool were asked about"
    );
    // The request named the call only; what it runs came from the earlier report.
    assert_eq!(
        approvals[1],
        &Operation::AgentTool {
            kind: ToolKind::Execute,
            title: "Run".into(),
            command: Some("rm -rf ~".into()),
            path: None,
            detail: Some("Clean up the home directory".into()),
        }
    );
    let outcomes: Vec<&Outcome> = seen
        .iter()
        .filter_map(|event| match event {
            AiEvent::Executed { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert!(matches!(
        outcomes[0],
        Outcome::Read {
            truncated: false,
            ..
        }
    ));
    assert!(matches!(outcomes[1], Outcome::Command { output } if output.stdout == "hi there\n"));
    assert!(matches!(
        outcomes[2],
        Outcome::Written {
            lines_before: 3,
            lines_after: 1,
            ..
        }
    ));
    assert!(matches!(outcomes[3], Outcome::Denied));
    assert!(
        seen.iter()
            .any(|event| matches!(event, AiEvent::Update { .. }))
    );
}

#[tokio::test]
async fn a_failed_turn_reports_the_agents_detail() {
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(|_| {}),
    ));
    let (client_side, agent_side) = (tokio::io::duplex(1 << 16), tokio::io::duplex(1 << 16));
    let mut fake = FakeAgent {
        lines: BufReader::new(agent_side.0).lines(),
        out: client_side.1,
        next_id: 100,
    };
    let script = tokio::spawn(async move {
        fake.answer(
            "initialize",
            json!({ "protocolVersion": 1, "authMethods": [] }),
        )
        .await;
        fake.answer("session/new", json!({ "sessionId": "s1" }))
            .await;
        let prompt = fake.receive().await;
        // Codex reports why the turn failed in `data.message`.
        fake.send(json!({ "jsonrpc": "2.0", "id": prompt["id"], "error": { "code": -32603, "message": "Internal error", "data": { "message": "stream disconnected before completion: connection refused (os error 10061)" } } })).await;
        fake
    });
    let agent = acp::connect(client_side.0, agent_side.1, session.clone())
        .await
        .unwrap();
    let running = runner::open(agent, session.clone(), &AgentSpec::default())
        .await
        .unwrap();
    let error = running.prompt("hi").await.unwrap_err();
    assert!(error.to_string().contains("os error 10061"), "{error}");
    drop(script.await.unwrap());
}

/// A session for the tests below, its events, and the fake agent's end.
fn harness() -> (
    Arc<AiSession>,
    mpsc::UnboundedReceiver<AiEvent>,
    FakeAgent,
    (DuplexStream, DuplexStream),
) {
    let (events, received) = mpsc::unbounded_channel::<AiEvent>();
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        Arc::new(move |event| {
            let _ = events.send(event);
        }),
    ));
    let (client_side, agent_side) = (tokio::io::duplex(1 << 16), tokio::io::duplex(1 << 16));
    let fake = FakeAgent {
        lines: BufReader::new(agent_side.0).lines(),
        out: client_side.1,
        next_id: 100,
    };
    (session, received, fake, (client_side.0, agent_side.1))
}

const MODEL_OPTIONS: &str = r#"[
    { "id": "mode", "name": "Approval Preset", "type": "select", "category": "mode", "currentValue": "auto", "options": [{ "value": "auto", "name": "Auto" }] },
    { "id": "model", "name": "Model", "type": "select", "category": "model", "currentValue": "gpt-5", "options": [{ "group": "openai", "name": "OpenAI", "options": [{ "value": "gpt-5", "name": "GPT-5" }, { "value": "gpt-5-mini", "name": "GPT-5 mini", "description": "Faster" }] }] },
    { "id": "notes", "name": "Notes", "type": "text", "currentValue": "" }
]"#;

#[tokio::test]
async fn a_session_is_resumed_and_its_settings_changed() {
    let (session, mut received, mut fake, (read, write)) = harness();
    let options: Value = serde_json::from_str(MODEL_OPTIONS).unwrap();
    let script = tokio::spawn(async move {
        fake.answer("initialize", json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": true }, "authMethods": [] }))
            .await;
        let load = fake.receive().await;
        assert_eq!(load["method"], "session/load");
        assert_eq!(load["params"]["sessionId"], "old-1");
        // The replay of the earlier conversation comes before the answer.
        fake.send(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "old-1", "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "earlier reply" } } } })).await;
        fake.send(
            json!({ "jsonrpc": "2.0", "id": load["id"], "result": { "configOptions": options } }),
        )
        .await;
        let set = fake.receive().await;
        assert_eq!(set["method"], "session/set_config_option");
        assert_eq!(
            set["params"],
            json!({ "sessionId": "old-1", "configId": "model", "value": "gpt-5-mini" })
        );
        let mut changed = options.clone();
        changed[1]["currentValue"] = json!("gpt-5-mini");
        fake.send(
            json!({ "jsonrpc": "2.0", "id": set["id"], "result": { "configOptions": changed } }),
        )
        .await;
        fake
    });
    let agent = acp::connect(read, write, session.clone()).await.unwrap();
    let spec = AgentSpec {
        resume: Some("old-1".into()),
        ..AgentSpec::default()
    };
    let running = runner::open(agent, session.clone(), &spec).await.unwrap();
    let started = running.started();
    assert!(started.resumed);
    assert_eq!(started.agent_session.as_deref(), Some("old-1"));
    // Select options only, grouped choices flattened.
    let ids: Vec<&str> = started
        .config_options
        .iter()
        .map(|option| option.id.as_str())
        .collect();
    assert_eq!(ids, ["mode", "model"]);
    let model = &started.config_options[1];
    assert_eq!(model.category.as_deref(), Some("model"));
    assert_eq!(model.choices.len(), 2);
    assert_eq!(model.choices[1].description.as_deref(), Some("Faster"));

    let options = running
        .set_config_option("model", "gpt-5-mini")
        .await
        .unwrap();
    assert_eq!(options[1].current, "gpt-5-mini");
    drop(script.await.unwrap());
    // The replay was not reported again.
    assert!(received.try_recv().is_err());
}

#[tokio::test]
async fn a_session_that_cannot_be_reopened_starts_anew() {
    let (session, _received, mut fake, (read, write)) = harness();
    let script = tokio::spawn(async move {
        fake.answer("initialize", json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": true }, "authMethods": [] }))
            .await;
        let load = fake.receive().await;
        assert_eq!(load["method"], "session/load");
        fake.send(json!({ "jsonrpc": "2.0", "id": load["id"], "error": { "code": -32002, "message": "Resource not found" } })).await;
        fake.answer("session/new", json!({ "sessionId": "new-1" }))
            .await;
        // Requests for the new session are accepted.
        let read = fake
            .call(
                "fs/read_text_file",
                json!({ "sessionId": "new-1", "path": "/nonexistent-rove" }),
            )
            .await;
        assert_ne!(read["error"]["message"], "unknown session", "{read}");
        fake
    });
    let agent = acp::connect(read, write, session.clone()).await.unwrap();
    let spec = AgentSpec {
        resume: Some("old-1".into()),
        ..AgentSpec::default()
    };
    let running = runner::open(agent, session.clone(), &spec).await.unwrap();
    assert!(!running.started().resumed);
    assert_eq!(running.started().agent_session.as_deref(), Some("new-1"));
    drop(script.await.unwrap());
}

#[tokio::test]
async fn older_model_lists_and_boolean_settings_can_be_changed() {
    let (session, _received, mut fake, (read, write)) = harness();
    let script = tokio::spawn(async move {
        fake.answer(
            "initialize",
            json!({ "protocolVersion": 1, "authMethods": [] }),
        )
        .await;
        fake.answer(
            "session/new",
            json!({
                "sessionId": "s1",
                "models": { "currentModelId": "sonnet", "availableModels": [{ "modelId": "sonnet", "name": "Sonnet" }, { "modelId": "opus", "name": "Opus", "description": "Most capable" }] },
                "configOptions": [{ "id": "fast", "name": "Fast mode", "type": "boolean", "currentValue": false }],
            }),
        )
        .await;
        let model = fake.answer("session/set_model", json!({})).await;
        assert_eq!(
            model["params"],
            json!({ "sessionId": "s1", "modelId": "opus" })
        );
        let fast = fake.answer("session/set_config_option", json!({})).await;
        assert_eq!(
            fast["params"],
            json!({ "sessionId": "s1", "configId": "fast", "value": true })
        );
        fake
    });
    let agent = acp::connect(read, write, session.clone()).await.unwrap();
    let running = runner::open(agent, session.clone(), &AgentSpec::default())
        .await
        .unwrap();
    let options = &running.started().config_options;
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].id, "model");
    assert_eq!(
        options[0].choices[1].description.as_deref(),
        Some("Most capable")
    );
    assert_eq!(options[1].kind, acp::ConfigKind::Boolean);
    assert_eq!(options[1].current, "false");

    let after = running.set_config_option("model", "opus").await.unwrap();
    assert_eq!(after[0].current, "opus");
    let after = running.set_config_option("fast", "true").await.unwrap();
    assert_eq!(after[1].current, "true");
    drop(script.await.unwrap());
}
