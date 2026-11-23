//! The audit trail of AI sessions: everything the agent was given or did, as journal records
//! (`ai/*` kinds, `session_id` = the AI session's id). Streamed replies are gathered into one
//! record per message; long outputs and contents are clipped, with a mark and a hash.

use std::sync::{Arc, Mutex};

use rove_journal::NewRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::acp::SessionUpdate;
use crate::llm::Message;
use crate::policy::Operation;
use crate::session::{AiEvent, Outcome};

pub const KIND_STARTED: &str = "ai/session_started";
pub const KIND_PROMPT: &str = "ai/prompt";
pub const KIND_WAKEUP: &str = "ai/ops_wakeup";
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
/// The session's title: written from the first prompt, and again when the user renames it.
pub const KIND_RENAMED: &str = "ai/renamed";
/// The built-in assistant summarized the earlier conversation (`payload.summary`).
pub const KIND_COMPACTED: &str = "ai/compacted";
/// The agent read the host's notes.
pub const KIND_NOTES_READ: &str = "ai/notes_read";
/// The agent replaced the host's notes (`operation.diff`).
pub const KIND_NOTES_WRITE: &str = "ai/notes_write";
/// A reviewer looked at an operation before the user would have been asked (`payload.reviewer`,
/// `allowed`, `confidence`, `operation`).
pub const KIND_REVIEWED: &str = "ai/reviewed";
/// The user deleted a conversation (`payload.session`); the record stays, without a session,
/// so the conversation is not brought back by sync.
pub const KIND_DELETED: &str = "ai/conversation_deleted";

/// Longest title kept.
pub const MAX_TITLE: usize = 120;

/// What a session started with, as its `ai/session_started` record holds it; enough to resume it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub agent: Option<String>,
    pub agent_version: Option<String>,
    pub mode: String,
    /// `builtin:<id>` or `acp:<id>`.
    #[serde(default)]
    pub agent_ref: Option<String>,
    /// The host context, as the app describes it.
    #[serde(default)]
    pub context: Option<Value>,
    /// The agent's own session id, for `session/load`.
    #[serde(default)]
    pub agent_session: Option<String>,
    /// This start continued an earlier run of the session.
    #[serde(default)]
    pub resumed: bool,
    /// The conversation that started this one as a sub-agent (the fleet lead's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// A title from a prompt: its first line, cut to 60 characters.
pub fn title_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    title
}

/// The record that renames session `ai_session`.
pub fn renamed(ai_session: &str, title: &str, auto: bool) -> NewRecord {
    NewRecord {
        kind: KIND_RENAMED.to_owned(),
        session_id: Some(ai_session.to_owned()),
        payload: json!({ "title": title, "auto": auto }),
    }
}

/// Longest text kept per field of a synced conversation.
const SYNC_CLIP: usize = 2 * 1024;
/// Largest synced conversation (JSON bytes); a longer one keeps its newest records.
pub const SYNC_MAX: usize = 64 * 1024;

/// `value` with every string longer than [`SYNC_CLIP`] bytes cut short.
fn clipped(value: &Value) -> Value {
    match value {
        Value::String(text) if text.len() > SYNC_CLIP => {
            let mut end = SYNC_CLIP;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(format!("{}…", &text[..end]))
        }
        Value::Array(items) => Value::Array(items.iter().map(clipped).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), clipped(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Removes local references and tool data from the shareable projection, including legacy rows.
pub fn tool_summary(payload: &mut Value) -> bool {
    let Some(fields) = payload.as_object_mut() else {
        return false;
    };
    fields.remove("body_ref");
    let mut private = fields.remove("input").is_some();
    if fields.get("private_text") == Some(&Value::Bool(true)) {
        for key in ["text", "summary", "title"] {
            private |= fields.remove(key).is_some();
        }
    }
    for field in ["outcome", "asked", "operation"] {
        if let Some(data) = fields.get_mut(field).and_then(Value::as_object_mut) {
            for key in ["stdout", "stderr", "content", "diff", "command", "detail"] {
                private |= data.remove(key).is_some();
            }
        }
    }
    if private && let Some(reasons) = fields.get_mut("reasons").and_then(Value::as_array_mut) {
        for reason in reasons {
            if let Some(fields) = reason.as_object_mut() {
                fields.remove("detail");
            }
        }
    }
    private
}

/// A session's records (`ts`, `kind`, `payload`) as one synced conversation: ordered by time,
/// long texts clipped, at most [`SYNC_MAX`] bytes (the oldest records give way, but never the
/// start and title records); `None` without a start record.
pub fn conversation_body(records: &[(i64, String, Value)]) -> Option<Value> {
    let mut entries: Vec<Value> = records
        .iter()
        .filter(|(_, kind, _)| kind.starts_with("ai/"))
        .map(|(ts, kind, payload)| {
            let mut summary = payload.clone();
            tool_summary(&mut summary);
            json!({ "ts": ts, "kind": kind, "payload": clipped(&summary) })
        })
        .collect();
    entries.sort_by_key(|entry| entry["ts"].as_i64().unwrap_or(0));
    // Without its start record a conversation could not be listed or continued.
    if !entries.iter().any(|entry| entry["kind"] == KIND_STARTED) {
        return None;
    }
    let pinned = |entry: &Value| entry["kind"] == KIND_STARTED || entry["kind"] == KIND_RENAMED;
    let mut size: usize = entries
        .iter()
        .map(|entry| entry.to_string().len() + 1)
        .sum();
    let mut dropped = vec![false; entries.len()];
    for (at, entry) in entries.iter().enumerate() {
        if size <= SYNC_MAX {
            break;
        }
        if !pinned(entry) {
            size -= entry.to_string().len() + 1;
            dropped[at] = true;
        }
    }
    let kept: Vec<&Value> = entries
        .iter()
        .zip(&dropped)
        .filter(|(_, dropped)| !**dropped)
        .map(|(entry, _)| entry)
        .collect();
    Some(json!({ "records": kept }))
}

/// The records of a synced conversation `body` that the local session (its records' `ts` and
/// `kind`) does not have yet. Records are told apart by time and kind, counting repeats, so the
/// clipped copies of local records are not added again.
pub fn missing_records(local: &[(i64, String)], body: &Value) -> Vec<(i64, String, Value)> {
    let mut have: std::collections::HashMap<(i64, &str), usize> = std::collections::HashMap::new();
    for (ts, kind) in local {
        *have.entry((*ts, kind.as_str())).or_default() += 1;
    }
    let mut seen: std::collections::HashMap<(i64, String), usize> =
        std::collections::HashMap::new();
    body.get("records")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let ts = entry.get("ts")?.as_i64()?;
            let kind = entry.get("kind")?.as_str()?;
            if !kind.starts_with("ai/") || kind.len() > 64 {
                return None;
            }
            let count = seen.entry((ts, kind.to_owned())).or_default();
            *count += 1;
            let local = have.get(&(ts, kind)).copied().unwrap_or(0);
            (*count > local).then(|| {
                (ts, kind.to_owned(), {
                    let mut payload = entry.get("payload").cloned().unwrap_or(Value::Null);
                    tool_summary(&mut payload);
                    payload
                })
            })
        })
        .collect()
}

/// The conversation of a session from its records (`kind`, `payload`, oldest first), for the
/// built-in assistant to go on with: prompts and replies, with what ran noted in the replies.
/// Tool output is left out.
pub fn transcript(records: &[(String, Value)]) -> Vec<Message> {
    fn reply(messages: &mut Vec<Message>, text: &str) {
        if let Some(Message::Assistant { text: last, .. }) = messages.last_mut() {
            if !last.is_empty() {
                last.push_str("\n\n");
            }
            last.push_str(text);
        } else {
            messages.push(Message::Assistant {
                text: text.to_owned(),
                tool_calls: Vec::new(),
                opaque: None,
            });
        }
    }
    let mut messages = Vec::new();
    for (kind, payload) in records {
        let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or("");
        let subject = || {
            let operation = &payload["operation"];
            ["command", "path", "title"]
                .iter()
                .find_map(|key| operation.get(*key).and_then(Value::as_str))
                .unwrap_or("")
                .to_owned()
        };
        match kind.as_str() {
            KIND_PROMPT if !text("text").is_empty() => messages.push(Message::User {
                text: text("text").to_owned(),
            }),
            KIND_MESSAGE if !text("text").is_empty() => reply(&mut messages, text("text")),
            KIND_COMMAND | KIND_FILE_READ | KIND_FILE_WRITE | KIND_LIST_DIR => {
                let result = payload["outcome"]["result"].as_str().unwrap_or("");
                let note = match (kind.as_str(), result) {
                    (_, "denied") => "refused by the user".to_owned(),
                    (_, "failed") => "failed".to_owned(),
                    (KIND_COMMAND, _) => format!("exit {}", payload["outcome"]["exit_code"]),
                    _ => "done".to_owned(),
                };
                let verb = kind.trim_start_matches("ai/");
                reply(&mut messages, &format!("[{verb} `{}`: {note}]", subject()));
            }
            _ => {}
        }
    }
    messages
}

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
    /// Serializes the first automatic title with `set_titled` before a manual rename.
    titled: Mutex<bool>,
    /// The start, kept until the session records something else: an agent started and left
    /// without a message leaves no history.
    pending_start: Mutex<Option<SessionInfo>>,
    private_text: std::sync::atomic::AtomicBool,
    /// Something was written.
    written: std::sync::atomic::AtomicBool,
}

impl Auditor {
    pub fn new(sink: RecordSink, ai_session: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            sink,
            ai_session: ai_session.into(),
            host: host.into(),
            message: Mutex::new(String::new()),
            thought: Mutex::new(String::new()),
            titled: Mutex::new(false),
            pending_start: Mutex::new(None),
            private_text: Default::default(),
            written: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Subsequent conversation text can quote an approved sensitive read.
    pub fn protect_text(&self) {
        self.private_text
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Prevents automatic naming and waits for any automatic title write already in progress.
    pub fn set_titled(&self) {
        *self
            .titled
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }

    fn write(&self, kind: &str, mut payload: Value) {
        let start = self
            .pending_start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(info) = start {
            self.write_now(
                KIND_STARTED,
                serde_json::to_value(&info).unwrap_or_else(|_| json!({})),
            );
        }
        self.write_now(kind, payload.take());
    }

    fn write_now(&self, kind: &str, mut payload: Value) {
        self.written
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Value::Object(fields) = &mut payload {
            fields.insert("host".into(), Value::String(self.host.clone()));
        }
        if self.private_text.load(std::sync::atomic::Ordering::SeqCst) {
            payload["private_text"] = true.into();
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
                let (kept, clipped) = if self.private_text.load(std::sync::atomic::Ordering::SeqCst)
                {
                    (text.clone(), false)
                } else {
                    clip(&text)
                };
                self.write(
                    kind,
                    json!({ "text": kept, "clipped": clipped, "sha256": sha256(&text) }),
                );
            }
        }
    }

    /// Notes how the session started; written with its first record (normally the first prompt).
    pub fn started(&self, info: &SessionInfo) {
        *self
            .pending_start
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(info.clone());
    }

    pub fn model_changed(&self, info: &SessionInfo) {
        self.started(info);
        if self.written.load(std::sync::atomic::Ordering::SeqCst) {
            self.write(
                "ai/model_changed",
                json!({"provider":info.agent, "model":info.agent_version}),
            );
        }
    }

    fn turn_input(&self, kind: &str, text: &str) {
        self.flush();
        let (kept, clipped) = if self.private_text.load(std::sync::atomic::Ordering::SeqCst) {
            (text.to_owned(), false)
        } else {
            clip(text)
        };
        self.write(kind, json!({ "text": kept, "clipped": clipped }));
    }

    pub fn prompt(&self, text: &str) {
        self.turn_input(KIND_PROMPT, text);
        let mut titled = self
            .titled
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *titled {
            return;
        }
        *titled = true;
        let title = title_of(text);
        if !title.is_empty() {
            self.write(KIND_RENAMED, json!({ "title": title, "auto": true }));
        }
    }

    /// Records the internal input to a completion summary without naming the conversation.
    pub fn wakeup(&self, text: &str) {
        self.turn_input(KIND_WAKEUP, text);
    }

    pub fn turn_ended(&self, stop_reason: &str) {
        self.flush();
        self.write(KIND_TURN_ENDED, json!({ "stop_reason": stop_reason }));
    }

    /// Records the end; a session that recorded nothing stays without records.
    pub fn ended(&self) {
        self.flush();
        if self.written.load(std::sync::atomic::Ordering::SeqCst) {
            self.write(KIND_ENDED, json!({}));
        }
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
            AiEvent::Update {
                update: SessionUpdate::Compacted { summary },
            } => {
                self.flush();
                self.write(KIND_COMPACTED, json!({ "summary": summary }));
            }
            // The agent's own tools: what it reported once they finished.
            AiEvent::Update {
                update: SessionUpdate::ToolCall { call, .. },
            } => {
                if matches!(call.status.as_deref(), Some("completed" | "failed")) {
                    self.flush();
                    let input = &call.raw_input;
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
                review,
                ..
            } => {
                self.flush();
                self.write(KIND_APPROVAL, json!({ "request_id": request_id, "asked": operation, "risk": assessment.risk, "reasons": assessment.reasons, "agent_tool": agent_tool, "review": review }));
            }
            AiEvent::Approved { request_id, answer } => {
                self.write(
                    KIND_APPROVAL,
                    json!({ "request_id": request_id, "answer": answer }),
                );
            }
            AiEvent::Reviewed {
                operation,
                reviewer,
                allowed,
                confidence,
            } => {
                self.flush();
                self.write(
                    KIND_REVIEWED,
                    json!({ "operation": operation, "reviewer": reviewer, "allowed": allowed, "confidence": confidence }),
                );
            }
            AiEvent::Executed {
                operation,
                assessment,
                approved,
                outcome,
            } => {
                self.flush();
                if matches!(outcome, Outcome::Read { .. } | Outcome::Command { .. })
                    && assessment.reasons.iter().any(|r| r.code == "read_secret")
                {
                    self.protect_text();
                }
                let kind = match operation {
                    Operation::Command { .. } => KIND_COMMAND,
                    Operation::ReadFile { .. } => KIND_FILE_READ,
                    Operation::WriteFile { .. } | Operation::EditFile { .. } => KIND_FILE_WRITE,
                    Operation::ListDir { .. } => KIND_LIST_DIR,
                    Operation::ReadTerminal => KIND_TERMINAL_READ,
                    Operation::ReadNotes => KIND_NOTES_READ,
                    Operation::UpdateNotes { .. } => KIND_NOTES_WRITE,
                    Operation::AgentTool { .. } => KIND_TOOL_CALL,
                };
                let outcome = match outcome {
                    Outcome::Command { output } => {
                        json!({
                            "result": "command", "exit_code": output.exit_code, "truncated": output.truncated,
                            "stdout": output.stdout, "stderr": output.stderr,
                            "stdout_sha256": sha256(&output.stdout), "stderr_sha256": sha256(&output.stderr),
                        })
                    }
                    Outcome::Read { content, .. } | Outcome::Written { content, .. } => {
                        let mut data = serde_json::to_value(outcome).unwrap_or(Value::Null);
                        data["content"] = json!(content);
                        data
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
    fn protected_prompts_do_not_leak_into_automatic_titles() {
        let (audit, records) = auditor();
        let private = "private-operations-request-marker";
        audit.protect_text();
        audit.started(&SessionInfo::default());
        audit.prompt(private);
        audit.turn_ended("end_turn");
        let records = records.lock().unwrap();
        let rename = records
            .iter()
            .find(|record| record.kind == KIND_RENAMED)
            .unwrap();
        assert_eq!(rename.payload["title"], private);
        assert_eq!(rename.payload["private_text"], true);
        assert_eq!(rename.payload["auto"], true);
        let mut summary = rename.payload.clone();
        assert!(tool_summary(&mut summary));
        assert!(summary.get("title").is_none());
        assert!(!summary.to_string().contains(private));
        for record in records.iter() {
            let mut summary = record.payload.clone();
            tool_summary(&mut summary);
            assert!(!summary.to_string().contains(private));
        }
        let records: Vec<_> = records
            .iter()
            .enumerate()
            .map(|(index, record)| (index as i64, record.kind.clone(), record.payload.clone()))
            .collect();
        let body = conversation_body(&records).unwrap();
        assert!(!body.to_string().contains(private));
    }

    #[test]
    fn protected_ops_wakeup_is_private_and_does_not_become_a_user_prompt() {
        let (audit, records) = auditor();
        let private = "private-report-fixture";
        audit.protect_text();
        audit.write(KIND_STARTED, json!({}));
        audit.wakeup(private);
        let records = records.lock().unwrap();
        let wakeup = records
            .iter()
            .find(|record| record.kind == KIND_WAKEUP)
            .unwrap();
        assert_eq!(wakeup.payload["text"], private);
        assert_eq!(wakeup.payload["private_text"], true);
        let mut summary = wakeup.payload.clone();
        assert!(tool_summary(&mut summary));
        assert!(summary.get("text").is_none());
        let records: Vec<_> = records
            .iter()
            .enumerate()
            .map(|(index, record)| (index as i64, record.kind.clone(), record.payload.clone()))
            .collect();
        let body = conversation_body(&records).unwrap();
        assert!(!body.to_string().contains(private));
        assert!(
            transcript(
                &records
                    .into_iter()
                    .map(|(_, kind, payload)| (kind, payload))
                    .collect::<Vec<_>>()
            )
            .is_empty()
        );
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
            review: None,
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
                KIND_RENAMED,
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
        assert_eq!(records[1].payload["title"], "restart nginx");
        assert_eq!(records[2].payload["text"], "Restarting now.");
        let command = &records[5].payload;
        assert_eq!(command["risk"], "modify");
        assert_eq!(command["approved"], true);
        assert!(command["outcome"]["stdout"].as_str().unwrap().len() > CLIP);
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
        audit.started(&SessionInfo {
            agent: Some("Codex".into()),
            mode: "ask_by_risk".into(),
            ..SessionInfo::default()
        });
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
        assert_eq!(kinds, [KIND_STARTED, KIND_PROMPT, KIND_RENAMED, KIND_ENDED]);
    }

    #[test]
    fn conversations_sync_clipped_and_merge_without_repeats() {
        let records = vec![
            (
                2,
                KIND_PROMPT.to_owned(),
                json!({ "text": "x".repeat(10_000) }),
            ),
            (1, KIND_STARTED.to_owned(), json!({ "agent": "A" })),
            (3, KIND_MESSAGE.to_owned(), json!({ "text": "hi" })),
            (3, KIND_MESSAGE.to_owned(), json!({ "text": "again" })),
        ];
        let body = conversation_body(&records).unwrap();
        let list = body["records"].as_array().unwrap();
        assert_eq!(list[0]["kind"], KIND_STARTED);
        assert!(list[1]["payload"]["text"].as_str().unwrap().len() < 3_000);

        // A device that has the start and one of the two messages gets the rest.
        let local = vec![(1, KIND_STARTED.to_owned()), (3, KIND_MESSAGE.to_owned())];
        let missing = missing_records(&local, &body);
        assert_eq!(missing.len(), 2);
        assert_eq!(missing[0].1, KIND_PROMPT);
        assert_eq!(missing[1].2["text"], "again");
        assert!(
            missing_records(
                &records
                    .iter()
                    .map(|(ts, kind, _)| (*ts, kind.clone()))
                    .collect::<Vec<_>>(),
                &body
            )
            .is_empty()
        );

        // A conversation too long keeps its start and its newest records.
        let mut long: Vec<(i64, String, Value)> = (1..200)
            .map(|at| {
                (
                    at,
                    KIND_MESSAGE.to_owned(),
                    json!({ "text": format!("{at}{}", "y".repeat(1_000)) }),
                )
            })
            .collect();
        assert!(conversation_body(&long).is_none(), "no start record");
        long.push((0, KIND_STARTED.to_owned(), json!({})));
        let body = conversation_body(&long).unwrap();
        assert!(body.to_string().len() <= SYNC_MAX + 1024);
        let list = body["records"].as_array().unwrap();
        assert_eq!(list[0]["kind"], KIND_STARTED);
        assert!(
            list.last().unwrap()["payload"]["text"]
                .as_str()
                .unwrap()
                .starts_with("199")
        );
    }

    #[test]
    fn a_session_without_a_message_leaves_no_history() {
        let (audit, records) = auditor();
        audit.started(&SessionInfo {
            agent: Some("Codex".into()),
            ..SessionInfo::default()
        });
        audit.ended();
        assert!(records.lock().unwrap().is_empty());

        let (audit, records) = auditor();
        audit.started(&SessionInfo::default());
        audit.prompt("hi");
        audit.ended();
        let kinds: Vec<String> = records
            .lock()
            .unwrap()
            .iter()
            .map(|record| record.kind.clone())
            .collect();
        assert_eq!(kinds, [KIND_STARTED, KIND_PROMPT, KIND_RENAMED, KIND_ENDED]);
    }

    #[test]
    fn titles_come_from_the_first_prompt() {
        assert_eq!(
            title_of("\n  check disk usage \nthen more"),
            "check disk usage"
        );
        let long = "x".repeat(80);
        assert_eq!(title_of(&long).chars().count(), 61);
        let (audit, records) = auditor();
        audit.prompt("first question");
        audit.prompt("second");
        let records = records.lock().unwrap();
        let titles: Vec<&Value> = records
            .iter()
            .filter(|record| record.kind == KIND_RENAMED)
            .map(|record| &record.payload["title"])
            .collect();
        assert_eq!(titles, [&json!("first question")]);
    }

    #[test]
    fn a_transcript_keeps_prompts_replies_and_what_ran() {
        let records = vec![
            (KIND_STARTED.to_owned(), json!({})),
            (KIND_PROMPT.to_owned(), json!({ "text": "disk?" })),
            (KIND_MESSAGE.to_owned(), json!({ "text": "Checking." })),
            (
                KIND_COMMAND.to_owned(),
                json!({ "operation": { "op": "command", "command": "df -h" }, "outcome": { "result": "command", "exit_code": 0 } }),
            ),
            (
                KIND_FILE_WRITE.to_owned(),
                json!({ "operation": { "op": "write_file", "path": "/a" }, "outcome": { "result": "denied" } }),
            ),
            (KIND_MESSAGE.to_owned(), json!({ "text": "80% used." })),
            (KIND_PROMPT.to_owned(), json!({ "text": "thanks" })),
        ];
        let messages = transcript(&records);
        assert_eq!(messages.len(), 3);
        assert!(matches!(&messages[0], Message::User { text } if text == "disk?"));
        assert!(
            matches!(&messages[1], Message::Assistant { text, tool_calls, .. }
            if tool_calls.is_empty()
                && text == "Checking.\n\n[command `df -h`: exit 0]\n\n[file_write `/a`: refused by the user]\n\n80% used.")
        );
        assert!(matches!(&messages[2], Message::User { text } if text == "thanks"));
    }
}
