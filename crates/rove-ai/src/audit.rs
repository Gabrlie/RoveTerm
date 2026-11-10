//! The audit trail of AI sessions: everything the agent was given or did, as journal records
//! (`ai/*` kinds, `session_id` = the AI session's id). Streamed replies are gathered into one
//! record per message; long outputs and contents are clipped, with a mark and a hash.

use std::sync::{Arc, Mutex};

use rove_journal::NewRecord;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::acp::SessionUpdate;
use crate::policy::Operation;
use crate::session::{AiEvent, Outcome};

pub const KIND_STARTED: &str = "ai/session_started";
pub const KIND_PROMPT: &str = "ai/prompt";
pub const KIND_MESSAGE: &str = "ai/message";
pub const KIND_THOUGHT: &str = "ai/thought";
pub const KIND_TOOL_CALL: &str = "ai/tool_call";
pub const KIND_APPROVAL: &str = "ai/approval";
pub const KIND_COMMAND: &str = "ai/command";
pub const KIND_FILE_READ: &str = "ai/file_read";
pub const KIND_FILE_WRITE: &str = "ai/file_write";
pub const KIND_LIST_DIR: &str = "ai/list_dir";
pub const KIND_TERMINAL_READ: &str = "ai/terminal_read";
pub const KIND_TURN_ENDED: &str = "ai/turn_ended";
pub const KIND_ENDED: &str = "ai/session_ended";

/// Text kept per field in a record.
const CLIP: usize = 16 * 1024;

/// Writes a record somewhere durable (the app's journal recorder).
pub type RecordSink = Arc<dyn Fn(NewRecord) + Send + Sync>;

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `text` cut to [`CLIP`] bytes, and whether it was cut.
fn clip(text: &str) -> (String, bool) {
    if text.len() <= CLIP {
        return (text.to_owned(), false);
    }
    let mut end = CLIP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

/// Records one AI session.
pub struct Auditor {
    sink: RecordSink,
    ai_session: String,
    host: String,
    /// The reply and reasoning being streamed, written out when something else happens.
    message: Mutex<String>,
    thought: Mutex<String>,
}

impl Auditor {
    pub fn new(sink: RecordSink, ai_session: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            sink,
            ai_session: ai_session.into(),
            host: host.into(),
            message: Mutex::new(String::new()),
            thought: Mutex::new(String::new()),
        }
    }

    fn write(&self, kind: &str, mut payload: Value) {
        if let Value::Object(fields) = &mut payload {
            fields.insert("host".into(), Value::String(self.host.clone()));
        }
        (self.sink)(NewRecord {
            kind: kind.to_owned(),
            session_id: Some(self.ai_session.clone()),
            payload,
        });
    }

    /// Writes out the gathered reply and reasoning, if any.
    fn flush(&self) {
        for (buffer, kind) in [(&self.thought, KIND_THOUGHT), (&self.message, KIND_MESSAGE)] {
            let text = std::mem::take(
                &mut *buffer
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
            if !text.is_empty() {
                let (kept, clipped) = clip(&text);
                self.write(
                    kind,
                    json!({ "text": kept, "clipped": clipped, "sha256": sha256(&text) }),
                );
            }
        }
    }

    pub fn started(&self, agent: Option<&str>, version: Option<&str>, mode: &str) {
        self.write(
            KIND_STARTED,
            json!({ "agent": agent, "agent_version": version, "mode": mode }),
        );
    }

    pub fn prompt(&self, text: &str) {
        self.flush();
        let (kept, clipped) = clip(text);
        self.write(KIND_PROMPT, json!({ "text": kept, "clipped": clipped }));
    }

    pub fn turn_ended(&self, stop_reason: &str) {
        self.flush();
        self.write(KIND_TURN_ENDED, json!({ "stop_reason": stop_reason }));
    }

    pub fn ended(&self) {
        self.flush();
        self.write(KIND_ENDED, json!({}));
    }

    /// Records a session event.
    pub fn event(&self, event: &AiEvent) {
        match event {
            AiEvent::Update {
                update: SessionUpdate::Message { text, .. },
            } => {
                self.message
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push_str(text);
            }
            AiEvent::Update {
                update: SessionUpdate::Thought { text },
            } => {
                self.thought
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push_str(text);
            }
            // The agent's own tools: what it reported once they finished.
            AiEvent::Update {
                update: SessionUpdate::ToolCall { call, .. },
            } => {
                if matches!(call.status.as_deref(), Some("completed" | "failed")) {
                    self.flush();
                    let input = call
                        .raw_input
                        .as_ref()
                        .map(|input| clip(&input.to_string()).0);
                    self.write(
                        KIND_TOOL_CALL,
                        json!({ "id": call.id, "title": call.title, "kind": call.kind, "status": call.status, "locations": call.locations, "input": input }),
                    );
                }
            }
            AiEvent::Update { .. } => {}
            AiEvent::Approval {
                request_id,
                operation,
                assessment,
                agent_tool,
                ..
            } => {
                self.flush();
                self.write(KIND_APPROVAL, json!({ "request_id": request_id, "asked": operation, "risk": assessment.risk, "reasons": assessment.reasons, "agent_tool": agent_tool }));
            }
            AiEvent::Approved { request_id, answer } => {
                self.write(
                    KIND_APPROVAL,
                    json!({ "request_id": request_id, "answer": answer }),
                );
            }
            AiEvent::Executed {
                operation,
                assessment,
                approved,
                outcome,
            } => {
                self.flush();
                let kind = match operation {
                    Operation::Command { .. } => KIND_COMMAND,
                    Operation::ReadFile { .. } => KIND_FILE_READ,
                    Operation::WriteFile { .. } => KIND_FILE_WRITE,
                    Operation::ListDir { .. } => KIND_LIST_DIR,
                    Operation::ReadTerminal => KIND_TERMINAL_READ,
                    Operation::AgentTool { .. } => KIND_TOOL_CALL,
                };
                let outcome = match outcome {
                    Outcome::Command { output } => {
                        let (stdout, stdout_clipped) = clip(&output.stdout);
                        let (stderr, stderr_clipped) = clip(&output.stderr);
                        json!({
                            "result": "command", "exit_code": output.exit_code, "truncated": output.truncated,
                            "stdout": stdout, "stderr": stderr, "clipped": stdout_clipped || stderr_clipped,
                            "stdout_sha256": sha256(&output.stdout), "stderr_sha256": sha256(&output.stderr),
                        })
                    }
                    other => serde_json::to_value(other).unwrap_or(Value::Null),
                };
                self.write(kind, json!({ "operation": operation, "risk": assessment.risk, "approved": approved, "outcome": outcome }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::ToolCall;
    use crate::host::CommandOutput;
    use crate::risk::{self, Rules};

    fn auditor() -> (Auditor, Arc<Mutex<Vec<NewRecord>>>) {
        let records = Arc::new(Mutex::new(Vec::new()));
        let sink = records.clone();
        (
            Auditor::new(
                Arc::new(move |record| sink.lock().unwrap().push(record)),
                "ai-1",
                "demo@web",
            ),
            records,
        )
    }

    #[test]
    fn a_turn_is_recorded_in_order() {
        let (audit, records) = auditor();
        audit.prompt("restart nginx");
        for text in ["Restarting", " now."] {
            audit.event(&AiEvent::Update {
                update: SessionUpdate::Message {
                    text: text.into(),
                    message_id: None,
                },
            });
        }
        let operation = Operation::Command {
            command: "systemctl restart nginx".into(),
        };
        let assessment = risk::assess_command("systemctl restart nginx", &Rules::default());
        audit.event(&AiEvent::Approval {
            request_id: 1,
            operation: operation.clone(),
            assessment: assessment.clone(),
            rememberable: true,
            agent_tool: false,
        });
        audit.event(&AiEvent::Approved {
            request_id: 1,
            answer: Some(crate::Answer::AllowOnce),
        });
        let output = CommandOutput {
            stdout: "x".repeat(CLIP + 10),
            stderr: String::new(),
            exit_code: Some(0),
            truncated: false,
        };
        audit.event(&AiEvent::Executed {
            operation,
            assessment,
            approved: true,
            outcome: Outcome::Command { output },
        });
        audit.event(&AiEvent::Update {
            update: SessionUpdate::ToolCall {
                call: ToolCall {
                    id: "t".into(),
                    status: Some("in_progress".into()),
                    ..ToolCall::default()
                },
                changed: true,
            },
        });
        audit.turn_ended("end_turn");

        let records = records.lock().unwrap();
        let kinds: Vec<&str> = records.iter().map(|record| record.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                KIND_PROMPT,
                KIND_MESSAGE,
                KIND_APPROVAL,
                KIND_APPROVAL,
                KIND_COMMAND,
                KIND_TURN_ENDED
            ]
        );
        assert!(
            records
                .iter()
                .all(|record| record.session_id.as_deref() == Some("ai-1")
                    && record.payload["host"] == "demo@web")
        );
        assert_eq!(records[1].payload["text"], "Restarting now.");
        let command = &records[4].payload;
        assert_eq!(command["risk"], "modify");
        assert_eq!(command["approved"], true);
        assert_eq!(command["outcome"]["clipped"], true);
        assert_eq!(command["outcome"]["stdout"].as_str().unwrap().len(), CLIP);
        assert_eq!(
            command["outcome"]["stdout_sha256"].as_str().unwrap().len(),
            64
        );
    }

    #[test]
    fn records_can_be_read_back_per_session() {
        use rove_journal::{JournalQuery, JournalStore, SqliteJournal};
        let store: Arc<dyn JournalStore> = Arc::new(SqliteJournal::open_in_memory().unwrap());
        let writer = store.clone();
        let sink: RecordSink = Arc::new(move |record| {
            writer.append(record).unwrap();
        });
        let audit = Auditor::new(sink.clone(), "ai-1", "local");
        audit.started(Some("Codex"), Some("1.0"), "ask_by_risk");
        audit.prompt("hello");
        audit.ended();
        Auditor::new(sink, "ai-2", "local").prompt("other");
        let query = JournalQuery {
            session_id: Some("ai-1".into()),
            ..JournalQuery::default()
        };
        let kinds: Vec<String> = store
            .read(&query)
            .unwrap()
            .into_iter()
            .map(|record| record.kind)
            .collect();
        assert_eq!(kinds, [KIND_STARTED, KIND_PROMPT, KIND_ENDED]);
    }
}
