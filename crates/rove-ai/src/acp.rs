//! The client side of the Agent Client Protocol: newline-delimited JSON-RPC 2.0 over the agent's
//! stdin and stdout. RoveTerm starts sessions and sends prompts; the agent streams updates and asks
//! the client to read and write files, run terminals and approve its own tools, which the
//! [`Client`] implementation answers. Everything the agent sends is untrusted and checked here
//! for shape and size before it reaches the client.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

/// The ACP major version spoken here.
pub const PROTOCOL_VERSION: u64 = 1;
/// Longest message accepted from an agent.
const MAX_MESSAGE: usize = 8 * 1024 * 1024;
/// Longest text kept from one streamed chunk or tool output for the client.
const MAX_TEXT: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum AcpError {
    #[error("could not start the agent: {0}")]
    Spawn(String),
    #[error("the agent exited")]
    Closed,
    /// An error response from the agent.
    #[error("{message}")]
    Rpc { code: i64, message: String },
    /// The agent needs `authenticate` before this request.
    #[error("the agent needs to be signed in: {0}")]
    AuthRequired(String),
    #[error("the agent sent an invalid message: {0}")]
    Protocol(String),
}

/// JSON-RPC error codes.
pub mod code {
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL: i64 = -32603;
    /// ACP: authentication required.
    pub const AUTH_REQUIRED: i64 = -32000;
    /// ACP: resource (file) not found.
    pub const NOT_FOUND: i64 = -32002;
}

/// An error answered to an agent's request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientError {
    pub code: i64,
    pub message: String,
}

impl ClientError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self::new(code::INTERNAL, message)
    }
}

/// What the agent reported about a running session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionUpdate {
    /// Part of the agent's reply (Markdown).
    Message {
        text: String,
        message_id: Option<String>,
    },
    /// Part of the agent's reasoning.
    Thought {
        text: String,
    },
    /// A tool call started, or changed (`changed`), with the fields it reported.
    ToolCall {
        call: ToolCall,
        changed: bool,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    ModeChanged {
        mode_id: String,
    },
    /// The agent changed its session settings.
    ConfigOptions {
        options: Vec<ConfigOption>,
    },
    /// How much of the model's context window the conversation fills (tokens).
    Usage {
        used: u64,
        size: u64,
    },
    /// The agent's slash commands (its full list each time).
    Commands {
        commands: Vec<AgentCommand>,
    },
    /// The built-in assistant's last request: tokens in and out, and how fast they came.
    Stats {
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        first_token_ms: Option<u64>,
        tokens_per_second: Option<f64>,
    },
}

/// A command the agent offers, run by sending `/<name>` (with arguments after it) as a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentCommand {
    pub name: String,
    pub description: String,
    /// What to type after the name, when it takes input.
    pub hint: Option<String>,
}

/// Most commands kept from one list.
const MAX_COMMANDS: usize = 200;

fn parse_commands(list: &Value) -> Option<Vec<AgentCommand>> {
    let commands = list
        .as_array()?
        .iter()
        .filter_map(|entry| {
            let name = string(entry, "name")?;
            let name = name.trim_start_matches('/').to_owned();
            if name.is_empty() || name.len() > 64 || name.chars().any(char::is_whitespace) {
                return None;
            }
            let clip = |text: String| text.chars().take(500).collect::<String>();
            Some(AgentCommand {
                name,
                description: clip(string(entry, "description").unwrap_or_default()),
                hint: entry
                    .pointer("/input/hint")
                    .and_then(Value::as_str)
                    .map(|hint| clip(hint.to_owned())),
            })
        })
        .take(MAX_COMMANDS)
        .collect();
    Some(commands)
}

/// A tool call as reported by the agent; fields an update leaves out are `None` or empty.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    /// `pending`, `in_progress`, `completed` or `failed`.
    pub status: Option<String>,
    pub content: Vec<ToolContent>,
    /// Files the call touches.
    pub locations: Vec<String>,
    /// The tool's raw arguments, such as `{"command": [...]}`.
    pub raw_input: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    Text {
        text: String,
    },
    Diff {
        path: String,
        old_text: Option<String>,
        new_text: String,
    },
    Terminal {
        terminal_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

/// An option the agent offers when asking permission for one of its tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
    #[serde(rename = "optionId")]
    pub option_id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}

/// The answer to a permission request: one of the offered options, or cancelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionOutcome {
    Selected(String),
    Cancelled,
}

/// A terminal's state for `terminal/output`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TerminalState {
    pub output: String,
    pub truncated: bool,
    /// Set once the command has ended: exit code, or `None` when it was killed.
    pub exit: Option<Option<u32>>,
}

/// Answers the agent's requests. Implemented by the AI session, which applies the policy.
#[async_trait]
pub trait Client: Send + Sync {
    async fn session_update(&self, session_id: &str, update: SessionUpdate);
    async fn request_permission(
        &self,
        session_id: &str,
        call: ToolCall,
        options: Vec<PermissionOption>,
    ) -> PermissionOutcome;
    /// Reads `path`, from 1-based `line` and at most `limit` lines when given.
    async fn read_text_file(
        &self,
        session_id: &str,
        path: &str,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> Result<String, ClientError>;
    async fn write_text_file(
        &self,
        session_id: &str,
        path: &str,
        content: &str,
    ) -> Result<(), ClientError>;
    /// Starts `command` with `args`; returns the terminal id.
    async fn create_terminal(
        &self,
        session_id: &str,
        command: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: Option<&str>,
        output_limit: Option<usize>,
    ) -> Result<String, ClientError>;
    async fn terminal_output(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<TerminalState, ClientError>;
    /// Waits for the command to end; returns its exit code (`None` when killed).
    async fn wait_for_terminal_exit(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<Option<u32>, ClientError>;
    async fn kill_terminal(&self, session_id: &str, terminal_id: &str) -> Result<(), ClientError>;
    async fn release_terminal(
        &self,
        session_id: &str,
        terminal_id: &str,
    ) -> Result<(), ClientError>;
}

/// A way to sign in that the agent offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// What `initialize` told about the agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentInfo {
    pub name: Option<String>,
    pub version: Option<String>,
    pub auth_methods: Vec<AuthMethod>,
    /// The agent can reopen one of its earlier sessions (`session/load`).
    pub load_session: bool,
}

/// A setting of a session the agent offers (ACP session config options), such as its model,
/// reasoning effort or a fast mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    /// `model`, `mode`, `thought_level`, or another category the agent names.
    pub category: Option<String>,
    pub kind: ConfigKind,
    /// The chosen value; `true` or `false` for a [`ConfigKind::Boolean`].
    pub current: String,
    /// The values of a [`ConfigKind::Select`].
    pub choices: Vec<ConfigChoice>,
    /// Built from the agent's older `models` field: changed with `session/set_model`.
    #[serde(skip)]
    pub legacy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigKind {
    Select,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigChoice {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
}

/// Most options and choices kept from an agent.
const MAX_OPTIONS: usize = 16;
const MAX_CHOICES: usize = 200;

/// The select options in an agent's `configOptions`; choices may be grouped.
pub fn parse_config_options(value: Option<&Value>) -> Vec<ConfigOption> {
    fn choice(value: &Value) -> Option<ConfigChoice> {
        Some(ConfigChoice {
            value: string(value, "value")?,
            name: string(value, "name")?,
            description: string(value, "description"),
        })
    }
    let Some(options) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    options
        .iter()
        .filter_map(|option| {
            if option.get("type").and_then(Value::as_str) == Some("boolean") {
                return Some(ConfigOption {
                    id: string(option, "id")?,
                    name: string(option, "name")?,
                    category: string(option, "category"),
                    kind: ConfigKind::Boolean,
                    current: option.get("currentValue")?.as_bool()?.to_string(),
                    choices: Vec::new(),
                    legacy: false,
                });
            }
            if option.get("type").and_then(Value::as_str) != Some("select") {
                return None;
            }
            let choices: Vec<ConfigChoice> = option
                .get("options")
                .and_then(Value::as_array)?
                .iter()
                .flat_map(
                    |entry| match entry.get("options").and_then(Value::as_array) {
                        Some(group) => group.iter().filter_map(choice).collect::<Vec<_>>(),
                        None => choice(entry).into_iter().collect(),
                    },
                )
                .take(MAX_CHOICES)
                .collect();
            Some(ConfigOption {
                id: string(option, "id")?,
                name: string(option, "name")?,
                category: string(option, "category"),
                kind: ConfigKind::Select,
                current: string(option, "currentValue")?,
                choices,
                legacy: false,
            })
        })
        .take(MAX_OPTIONS)
        .collect()
}

/// A mode the agent can work in (`read-only`, `auto`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// A new session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewSession {
    pub session_id: String,
    pub current_mode: Option<String>,
    pub modes: Vec<SessionMode>,
    pub config_options: Vec<ConfigOption>,
}

/// An MCP server the agent should start for a session, speaking MCP over stdio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    /// Absolute path of the executable.
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>>;

struct Shared {
    outgoing: mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: AtomicU64,
}

impl Shared {
    fn send(&self, message: Value) -> Result<(), AcpError> {
        self.outgoing
            .send(message.to_string())
            .map_err(|_| AcpError::Closed)
    }
}

/// A running agent.
pub struct Agent {
    shared: Arc<Shared>,
    child: std::sync::Mutex<Option<tokio::process::Child>>,
    info: AgentInfo,
}

/// Starts `program` with `args` and an environment of exactly `env`, then initializes it.
pub async fn spawn(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: &std::path::Path,
    client: Arc<dyn Client>,
) -> Result<Agent, AcpError> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(name, value)| (name, value)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window for the agent.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| AcpError::Spawn(format!("{program}: {error}")))?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(AcpError::Spawn("no pipes".into()));
    };
    // The agent's diagnostics go to the log, capped per line.
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "rove_ai::agent", "{}", truncate(&line, 2000));
        }
    });
    let agent = connect(stdout, stdin, client).await?;
    if let Ok(mut slot) = agent.child.lock() {
        *slot = Some(child);
    }
    Ok(agent)
}

/// Speaks ACP over `reader` and `writer` (an agent's stdout and stdin) and initializes it.
pub async fn connect(
    reader: impl AsyncRead + Send + Unpin + 'static,
    writer: impl AsyncWrite + Send + Unpin + 'static,
    client: Arc<dyn Client>,
) -> Result<Agent, AcpError> {
    let (outgoing, mut queue) = mpsc::unbounded_channel::<String>();
    let shared = Arc::new(Shared {
        outgoing,
        pending: Mutex::new(HashMap::new()),
        next_id: AtomicU64::new(0),
    });
    let mut writer = writer;
    tokio::spawn(async move {
        while let Some(line) = queue.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err()
                || writer.write_all(b"\n").await.is_err()
                || writer.flush().await.is_err()
            {
                break;
            }
        }
    });
    let reading = shared.clone();
    tokio::spawn(async move {
        read_loop(reader, reading.clone(), client).await;
        // The agent is gone: fail everything still waiting.
        for (_, waiter) in reading.pending.lock().await.drain() {
            let _ = waiter.send(Err(AcpError::Closed));
        }
    });
    let mut agent = Agent {
        shared,
        child: std::sync::Mutex::new(None),
        info: AgentInfo::default(),
    };
    agent.info = agent.initialize().await?;
    Ok(agent)
}

fn truncate(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

async fn read_loop(reader: impl AsyncRead + Unpin, shared: Arc<Shared>, client: Arc<dyn Client>) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        // Read one line, refusing to buffer more than the limit.
        let mut too_long = false;
        loop {
            let available = match reader.fill_buf().await {
                Ok(available) => available,
                Err(_) => return,
            };
            if available.is_empty() {
                return;
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let chunk = &available[..newline.unwrap_or(available.len())];
            if line.len() + chunk.len() > MAX_MESSAGE {
                too_long = true;
            } else {
                line.extend_from_slice(chunk);
            }
            let ended = newline.is_some();
            let used = newline.map_or(available.len(), |at| at + 1);
            reader.consume(used);
            if ended {
                break;
            }
        }
        if too_long {
            tracing::warn!("dropping an oversized message from the agent");
            continue;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            tracing::warn!("dropping a malformed message from the agent");
            continue;
        };
        dispatch(message, &shared, &client).await;
    }
}

async fn dispatch(message: Value, shared: &Arc<Shared>, client: &Arc<dyn Client>) {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let id = message.get("id").cloned().filter(|id| !id.is_null());
    match (method, id) {
        // A response to one of our requests.
        (None, Some(id)) => {
            let Some(id) = id.as_u64() else {
                return;
            };
            let Some(waiter) = shared.pending.lock().await.remove(&id) else {
                return;
            };
            let result = match (message.get("result"), message.get("error")) {
                (_, Some(error)) => {
                    let code = error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or(code::INTERNAL);
                    // The detail some agents put in `data.message` (Codex: why the turn failed)
                    // says more than the generic message.
                    let text = error
                        .pointer("/data/message")
                        .and_then(Value::as_str)
                        .or_else(|| error.get("message").and_then(Value::as_str))
                        .unwrap_or("error");
                    let text = truncate(text, 2000).to_owned();
                    Err(if code == code::AUTH_REQUIRED {
                        AcpError::AuthRequired(text)
                    } else {
                        AcpError::Rpc {
                            code,
                            message: text,
                        }
                    })
                }
                (Some(result), None) => Ok(result.clone()),
                (None, None) => Ok(Value::Null),
            };
            let _ = waiter.send(result);
        }
        // A request from the agent: answer it without holding up the reader.
        (Some(method), Some(id)) => {
            let shared = shared.clone();
            let client = client.clone();
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            tokio::spawn(async move {
                let reply = match handle_request(&method, params, client.as_ref()).await {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err(error) => {
                        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": error.code, "message": error.message } })
                    }
                };
                let _ = shared.send(reply);
            });
        }
        (Some(method), None) if method == "session/update" => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
                return;
            };
            if let Some(update) = params.get("update").and_then(parse_update) {
                client.session_update(session_id, update).await;
            }
        }
        _ => {}
    }
}

fn text_of(content: &Value) -> Option<String> {
    (content.get("type").and_then(Value::as_str) == Some("text"))
        .then(|| content.get("text").and_then(Value::as_str))
        .flatten()
        .map(|text| truncate(text, MAX_TEXT).to_owned())
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| truncate(text, 4096).to_owned())
}

/// Reads a tool call or tool call update.
fn parse_tool_call(value: &Value) -> Option<ToolCall> {
    let id = string(value, "toolCallId")?;
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(64)
                .filter_map(|item| match item.get("type").and_then(Value::as_str)? {
                    "content" => item
                        .get("content")
                        .and_then(text_of)
                        .map(|text| ToolContent::Text { text }),
                    "diff" => Some(ToolContent::Diff {
                        path: string(item, "path")?,
                        old_text: item
                            .get("oldText")
                            .and_then(Value::as_str)
                            .map(|text| truncate(text, MAX_TEXT).to_owned()),
                        new_text: truncate(item.get("newText").and_then(Value::as_str)?, MAX_TEXT)
                            .to_owned(),
                    }),
                    "terminal" => Some(ToolContent::Terminal {
                        terminal_id: string(item, "terminalId")?,
                    }),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let locations = value
        .get("locations")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(64)
                .filter_map(|item| string(item, "path"))
                .collect()
        })
        .unwrap_or_default();
    let raw_input = value
        .get("rawInput")
        .filter(|input| input.to_string().len() <= MAX_TEXT)
        .cloned();
    Some(ToolCall {
        id,
        title: string(value, "title"),
        kind: string(value, "kind"),
        status: string(value, "status"),
        content,
        locations,
        raw_input,
    })
}

/// Reads a `session/update`; kinds this client does not show are dropped.
pub fn parse_update(update: &Value) -> Option<SessionUpdate> {
    match update.get("sessionUpdate").and_then(Value::as_str)? {
        "agent_message_chunk" => Some(SessionUpdate::Message {
            text: update.get("content").and_then(text_of)?,
            message_id: string(update, "messageId"),
        }),
        "agent_thought_chunk" => Some(SessionUpdate::Thought {
            text: update.get("content").and_then(text_of)?,
        }),
        "tool_call" => Some(SessionUpdate::ToolCall {
            call: parse_tool_call(update)?,
            changed: false,
        }),
        "tool_call_update" => Some(SessionUpdate::ToolCall {
            call: parse_tool_call(update)?,
            changed: true,
        }),
        "plan" => Some(SessionUpdate::Plan {
            entries: update
                .get("entries")
                .and_then(Value::as_array)?
                .iter()
                .take(100)
                .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
                .collect(),
        }),
        "current_mode_update" => Some(SessionUpdate::ModeChanged {
            mode_id: string(update, "currentModeId")?,
        }),
        "usage_update" => Some(SessionUpdate::Usage {
            used: update.get("used").and_then(Value::as_u64)?,
            size: update
                .get("size")
                .and_then(Value::as_u64)
                .filter(|size| *size > 0)?,
        }),
        "available_commands_update" => Some(SessionUpdate::Commands {
            commands: parse_commands(update.get("availableCommands")?)?,
        }),
        "config_option_update" => Some(SessionUpdate::ConfigOptions {
            options: parse_config_options(update.get("configOptions")),
        }),
        _ => None,
    }
}

fn params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, ClientError> {
    serde_json::from_value(params)
        .map_err(|error| ClientError::new(code::INVALID_PARAMS, error.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRef {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalRef {
    session_id: String,
    terminal_id: String,
}

async fn handle_request(
    method: &str,
    raw: Value,
    client: &dyn Client,
) -> Result<Value, ClientError> {
    match method {
        "session/request_permission" => {
            let session: SessionRef = params(raw.clone())?;
            let call = raw
                .get("toolCall")
                .and_then(parse_tool_call)
                .ok_or_else(|| ClientError::new(code::INVALID_PARAMS, "missing toolCall"))?;
            let options: Vec<PermissionOption> = raw
                .get("options")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .take(16)
                        .filter_map(|item| serde_json::from_value(item.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            let outcome = match client
                .request_permission(&session.session_id, call, options)
                .await
            {
                PermissionOutcome::Selected(option_id) => {
                    json!({ "outcome": "selected", "optionId": option_id })
                }
                PermissionOutcome::Cancelled => json!({ "outcome": "cancelled" }),
            };
            Ok(json!({ "outcome": outcome }))
        }
        "fs/read_text_file" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Read {
                session_id: String,
                path: String,
                line: Option<u32>,
                limit: Option<u32>,
            }
            let request: Read = params(raw)?;
            check_path(&request.path)?;
            let content = client
                .read_text_file(
                    &request.session_id,
                    &request.path,
                    request.line,
                    request.limit,
                )
                .await?;
            Ok(json!({ "content": content }))
        }
        "fs/write_text_file" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Write {
                session_id: String,
                path: String,
                content: String,
            }
            let request: Write = params(raw)?;
            check_path(&request.path)?;
            client
                .write_text_file(&request.session_id, &request.path, &request.content)
                .await?;
            Ok(json!({}))
        }
        "terminal/create" => {
            #[derive(Deserialize)]
            struct Variable {
                name: String,
                value: String,
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Create {
                session_id: String,
                command: String,
                #[serde(default)]
                args: Vec<String>,
                #[serde(default)]
                env: Vec<Variable>,
                cwd: Option<String>,
                output_byte_limit: Option<u64>,
            }
            let request: Create = params(raw)?;
            if request.command.is_empty()
                || request.command.len() > 64 * 1024
                || request.args.len() > 1024
                || request.env.len() > 256
            {
                return Err(ClientError::new(code::INVALID_PARAMS, "command too large"));
            }
            if let Some(cwd) = &request.cwd {
                check_path(cwd)?;
            }
            let env: Vec<(String, String)> = request
                .env
                .into_iter()
                .map(|variable| (variable.name, variable.value))
                .collect();
            if env.iter().any(|(name, _)| {
                name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            }) {
                return Err(ClientError::new(
                    code::INVALID_PARAMS,
                    "invalid environment variable name",
                ));
            }
            let limit = request
                .output_byte_limit
                .map(|limit| usize::try_from(limit).unwrap_or(usize::MAX));
            let id = client
                .create_terminal(
                    &request.session_id,
                    &request.command,
                    &request.args,
                    &env,
                    request.cwd.as_deref(),
                    limit,
                )
                .await?;
            Ok(json!({ "terminalId": id }))
        }
        "terminal/output" => {
            let request: TerminalRef = params(raw)?;
            let state = client
                .terminal_output(&request.session_id, &request.terminal_id)
                .await?;
            let exit = state.exit.map(|code| json!({ "exitCode": code, "signal": if code.is_none() { Some("SIGKILL") } else { None } }));
            Ok(json!({ "output": state.output, "truncated": state.truncated, "exitStatus": exit }))
        }
        "terminal/wait_for_exit" => {
            let request: TerminalRef = params(raw)?;
            let code = client
                .wait_for_terminal_exit(&request.session_id, &request.terminal_id)
                .await?;
            Ok(
                json!({ "exitCode": code, "signal": if code.is_none() { Some("SIGKILL") } else { None } }),
            )
        }
        "terminal/kill" => {
            let request: TerminalRef = params(raw)?;
            client
                .kill_terminal(&request.session_id, &request.terminal_id)
                .await?;
            Ok(json!({}))
        }
        "terminal/release" => {
            let request: TerminalRef = params(raw)?;
            client
                .release_terminal(&request.session_id, &request.terminal_id)
                .await?;
            Ok(json!({}))
        }
        _ => Err(ClientError::new(
            code::METHOD_NOT_FOUND,
            format!("unsupported method {method}"),
        )),
    }
}

/// Paths from the agent must be absolute and of reasonable length.
fn check_path(path: &str) -> Result<(), ClientError> {
    let absolute =
        path.starts_with('/') || path.get(1..3) == Some(":\\") || path.get(1..3) == Some(":/");
    if !absolute || path.len() > 4096 || path.contains('\0') {
        return Err(ClientError::new(
            code::INVALID_PARAMS,
            "the path must be absolute",
        ));
    }
    Ok(())
}

/// `mcpServers` of `session/new` and `session/load`.
fn servers_json(mcp_servers: &[McpServer]) -> Vec<Value> {
    mcp_servers
        .iter()
        .map(|server| {
            json!({
                "name": server.name,
                "command": server.command,
                "args": server.args,
                "env": server.env.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// The session `session_id` from a `session/new` or `session/load` result.
fn session_of(session_id: String, result: &Value) -> NewSession {
    let modes = result.get("modes");
    NewSession {
        session_id,
        current_mode: modes.and_then(|modes| string(modes, "currentModeId")),
        modes: modes
            .and_then(|modes| modes.get("availableModes"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .take(32)
                    .filter_map(|item| serde_json::from_value(item.clone()).ok())
                    .collect()
            })
            .unwrap_or_default(),
        config_options: with_legacy_models(
            parse_config_options(result.get("configOptions")),
            result.get("models"),
        ),
    }
}

/// Adds the model choice of an agent that reports it in the older `models` field
/// (`availableModels`, `currentModelId`) rather than as a config option.
fn with_legacy_models(mut options: Vec<ConfigOption>, models: Option<&Value>) -> Vec<ConfigOption> {
    let has_model = options
        .iter()
        .any(|option| option.category.as_deref() == Some("model") || option.id == "model");
    let Some(models) = models.filter(|_| !has_model) else {
        return options;
    };
    let choices: Vec<ConfigChoice> = models
        .get("availableModels")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|model| {
                    let value = string(model, "modelId")?;
                    Some(ConfigChoice {
                        name: string(model, "name").unwrap_or_else(|| value.clone()),
                        description: string(model, "description"),
                        value,
                    })
                })
                .take(MAX_CHOICES)
                .collect()
        })
        .unwrap_or_default();
    if let Some(current) = string(models, "currentModelId").filter(|_| !choices.is_empty()) {
        options.insert(
            0,
            ConfigOption {
                id: "model".into(),
                name: "Model".into(),
                category: Some("model".into()),
                kind: ConfigKind::Select,
                current,
                choices,
                legacy: true,
            },
        );
    }
    options
}

impl Agent {
    pub fn info(&self) -> &AgentInfo {
        &self.info
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, AcpError> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.shared.pending.lock().await.insert(id, sender);
        self.shared
            .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        receiver.await.map_err(|_| AcpError::Closed)?
    }

    async fn initialize(&self) -> Result<AgentInfo, AcpError> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true }, "terminal": true },
                    "clientInfo": { "name": "roveterm", "title": "RoveTerm", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        let version = result.get("protocolVersion").and_then(Value::as_u64);
        if version != Some(PROTOCOL_VERSION) {
            return Err(AcpError::Protocol(format!(
                "unsupported protocol version {version:?}"
            )));
        }
        let info = result.get("agentInfo");
        Ok(AgentInfo {
            name: info.and_then(|info| string(info, "title").or_else(|| string(info, "name"))),
            version: info.and_then(|info| string(info, "version")),
            auth_methods: result
                .get("authMethods")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .take(16)
                        .filter_map(|item| serde_json::from_value(item.clone()).ok())
                        .collect()
                })
                .unwrap_or_default(),
            load_session: result
                .pointer("/agentCapabilities/loadSession")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// Signs in with one of the agent's [`AgentInfo::auth_methods`].
    pub async fn authenticate(&self, method_id: &str) -> Result<(), AcpError> {
        self.request("authenticate", json!({ "methodId": method_id }))
            .await
            .map(|_| ())
    }

    /// Starts a session working in `cwd` with the given MCP servers.
    pub async fn new_session(
        &self,
        cwd: &str,
        mcp_servers: &[McpServer],
    ) -> Result<NewSession, AcpError> {
        let result = self
            .request(
                "session/new",
                json!({ "cwd": cwd, "mcpServers": servers_json(mcp_servers) }),
            )
            .await?;
        let session_id = string(&result, "sessionId")
            .ok_or_else(|| AcpError::Protocol("missing sessionId".into()))?;
        Ok(session_of(session_id, &result))
    }

    /// Reopens the agent's earlier session `session_id` (when [`AgentInfo::load_session`]). The
    /// agent replays the conversation as updates before answering.
    pub async fn load_session(
        &self,
        session_id: &str,
        cwd: &str,
        mcp_servers: &[McpServer],
    ) -> Result<NewSession, AcpError> {
        let result = self
            .request(
                "session/load",
                json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": servers_json(mcp_servers) }),
            )
            .await?;
        Ok(session_of(session_id.to_owned(), &result))
    }

    /// Sets one of the session's [`ConfigOption`]s (`value` is `true` / `false` for a boolean
    /// one); returns all of them as the agent now reports them, empty when it reports none.
    pub async fn set_config_option(
        &self,
        session_id: &str,
        config_id: &str,
        kind: ConfigKind,
        value: &str,
    ) -> Result<Vec<ConfigOption>, AcpError> {
        let value = match kind {
            ConfigKind::Boolean => Value::Bool(value == "true"),
            ConfigKind::Select => Value::String(value.to_owned()),
        };
        let result = self
            .request(
                "session/set_config_option",
                json!({ "sessionId": session_id, "configId": config_id, "value": value }),
            )
            .await?;
        Ok(parse_config_options(result.get("configOptions")))
    }

    /// Switches the model of an agent that offers it in the older `models` field.
    pub async fn set_model(&self, session_id: &str, model_id: &str) -> Result<(), AcpError> {
        self.request(
            "session/set_model",
            json!({ "sessionId": session_id, "modelId": model_id }),
        )
        .await
        .map(|_| ())
    }

    pub async fn set_mode(&self, session_id: &str, mode_id: &str) -> Result<(), AcpError> {
        self.request(
            "session/set_mode",
            json!({ "sessionId": session_id, "modeId": mode_id }),
        )
        .await
        .map(|_| ())
    }

    /// Sends a prompt and waits for the turn to end; returns the stop reason (`end_turn`,
    /// `cancelled`, …).
    pub async fn prompt(&self, session_id: &str, text: &str) -> Result<String, AcpError> {
        let result = self
            .request(
                "session/prompt",
                json!({ "sessionId": session_id, "prompt": [{ "type": "text", "text": text }] }),
            )
            .await?;
        Ok(string(&result, "stopReason").unwrap_or_else(|| "end_turn".into()))
    }

    /// Asks the agent to stop the current turn.
    pub fn cancel(&self, session_id: &str) -> Result<(), AcpError> {
        self.shared.send(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": session_id } }))
    }

    /// Stops the agent process; requests still waiting fail with [`AcpError::Closed`].
    pub async fn shutdown(&self) {
        let child = self.child.lock().ok().and_then(|mut slot| slot.take());
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }
    }
}

/// The environment for an agent: the listed variables of this process, then `extra` (which wins).
pub fn agent_env(allowed: &[&str], extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(name, _)| {
            allowed
                .iter()
                .any(|pattern| match pattern.strip_suffix('*') {
                    Some(prefix) => name.starts_with(prefix),
                    None => name == pattern,
                })
        })
        .filter(|(name, _)| !extra.iter().any(|(other, _)| other == name))
        .collect();
    env.extend(extra.iter().cloned());
    env
}

/// Variables an agent inherits from RoveTerm's environment.
pub const ENV_ALLOWED: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_*",
    "TERM",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_*",
    "SystemRoot",
    "SYSTEMROOT",
    "SystemDrive",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "COMSPEC",
    "PATHEXT",
    "WINDIR",
    "HOMEDRIVE",
    "HOMEPATH",
    "CODEX_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "https_proxy",
    "http_proxy",
    "no_proxy",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_updates() {
        let chunk = json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hi" } });
        assert_eq!(
            parse_update(&chunk),
            Some(SessionUpdate::Message {
                text: "Hi".into(),
                message_id: None
            })
        );
        let call = json!({
            "sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Edit", "kind": "edit", "status": "pending",
            "content": [{ "type": "diff", "path": "/a", "oldText": "x", "newText": "y" }, { "type": "bogus" }],
            "locations": [{ "path": "/a", "line": 3 }], "rawInput": { "command": ["ls"] }
        });
        let Some(SessionUpdate::ToolCall {
            call,
            changed: false,
        }) = parse_update(&call)
        else {
            panic!()
        };
        assert_eq!(
            call.content,
            [ToolContent::Diff {
                path: "/a".into(),
                old_text: Some("x".into()),
                new_text: "y".into()
            }]
        );
        assert_eq!(call.locations, ["/a"]);
        assert_eq!(
            parse_update(&json!({ "sessionUpdate": "usage_update" })),
            None
        );
        assert_eq!(parse_update(&json!({ "sessionUpdate": "tool_call" })), None);
    }

    #[test]
    fn agent_commands_are_read_and_checked() {
        let update = json!({
            "sessionUpdate": "available_commands_update",
            "availableCommands": [
                { "name": "review", "description": "Review changes", "input": { "hint": "what to review" } },
                { "name": "/compact", "description": "Compact" },
                { "name": "two words", "description": "dropped" },
                { "name": "", "description": "dropped" },
                { "description": "no name" },
            ],
        });
        let Some(SessionUpdate::Commands { commands }) = parse_update(&update) else {
            panic!()
        };
        assert_eq!(
            commands,
            [
                AgentCommand {
                    name: "review".into(),
                    description: "Review changes".into(),
                    hint: Some("what to review".into()),
                },
                AgentCommand {
                    name: "compact".into(),
                    description: "Compact".into(),
                    hint: None,
                },
            ]
        );
    }

    #[test]
    fn environment_is_allowlisted() {
        let env = agent_env(
            &["PATH", "LC_*"],
            &[
                ("OPENAI_API_KEY".into(), "k".into()),
                ("PATH".into(), "/x".into()),
            ],
        );
        assert!(env.contains(&("PATH".into(), "/x".into())));
        assert_eq!(env.iter().filter(|(name, _)| name == "PATH").count(), 1);
        assert!(env.iter().all(|(name, _)| name == "PATH"
            || name.starts_with("LC_")
            || name == "OPENAI_API_KEY"));
    }

    #[test]
    fn agent_paths_must_be_absolute() {
        assert!(check_path("/etc/hosts").is_ok());
        assert!(check_path("C:\\Users\\me\\a.txt").is_ok());
        assert!(check_path("relative/a.txt").is_err());
        assert!(check_path("/a\0b").is_err());
    }
}
