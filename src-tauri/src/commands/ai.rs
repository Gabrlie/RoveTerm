//! AI sessions: start an agent (the built-in assistant with a model provider, or a local ACP
//! agent) on a host context, send prompts, answer approvals, stop. Events stream to the WebView
//! as `ai-update` and are recorded in the journal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rove_ai::acp::{self, McpServer};
use rove_ai::assistant::{Assistant, HostMeta};
use rove_ai::audit::Auditor;
use rove_ai::backend::{AgentBackend, AgentRef, BackendError};
use rove_ai::detect;
use rove_ai::host::{ConnectionSource, Host, LocalHost, RemoteHost};
use rove_ai::llm::{self, Client, Endpoint};
use rove_ai::mcp::McpListener;
use rove_ai::runner::{self, AgentSpec};
use rove_ai::session::{AiEvent, AiSession, SessionSettings, TerminalSource};
use rove_ai::{Answer, Mode, Rules};
use rove_session::{SessionId, SessionManager};
use rove_ssh::{SshConnection, SshConnector, SshTarget};
use rove_vault::{AgentAuth, AiMode, ProviderFormat, ProviderPrefs, ReasoningEffort};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use zeroize::Zeroizing;

use crate::connect::{host_target, quick_target};
use crate::error::CommandError;
use crate::state::AppState;

/// Event carrying an AI session's [`AiEvent`]s.
pub const AI_EVENT: &str = "ai-update";
/// Longest prompt accepted.
const MAX_PROMPT: usize = 100_000;
/// Agent modes that make an ACP agent ask before its own tools change anything.
const PREFERRED_MODES: &[&str] = &["read-only", "read_only", "readonly", "default", "plan"];
/// How long a model may stay silent before its reply counts as stalled.
const MODEL_IDLE: Duration = Duration::from_secs(180);

/// One running AI session.
struct Entry {
    backend: Arc<dyn AgentBackend>,
    auditor: Arc<Auditor>,
    /// The built-in MCP server of an ACP agent.
    mcp: Option<tokio::task::JoinHandle<()>>,
}

/// AI sessions of this app.
pub struct AiSessions {
    entries: tokio::sync::Mutex<HashMap<String, Arc<Entry>>>,
    /// Working directory of agents whose host is remote: an empty scratch directory.
    workspace: PathBuf,
    recorder: rove_journal::Recorder,
}

impl AiSessions {
    pub fn new(
        data_dir: &Path,
        journal: Arc<dyn rove_journal::JournalStore>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            entries: tokio::sync::Mutex::new(HashMap::new()),
            workspace: data_dir.join("ai-workspace"),
            recorder: rove_journal::Recorder::start(journal)?,
        })
    }

    async fn get(&self, id: &str) -> Result<Arc<Entry>, CommandError> {
        self.entries
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| CommandError::new("ai.not_found", "no such AI session"))
    }

    /// Stops every session; used on exit.
    pub async fn stop_all(&self) {
        let entries: Vec<Arc<Entry>> = self
            .entries
            .lock()
            .await
            .drain()
            .map(|(_, entry)| entry)
            .collect();
        for entry in entries {
            stop(&entry).await;
        }
    }
}

async fn stop(entry: &Entry) {
    entry.backend.stop().await;
    if let Some(mcp) = &entry.mcp {
        mcp.abort();
    }
    entry.auditor.ended();
}

fn backend_error(error: BackendError) -> CommandError {
    match error {
        BackendError::Acp(error) => acp_error(error),
        BackendError::Model(message) => CommandError::new("ai.model", message),
        BackendError::Busy => CommandError::new("ai.busy", "a turn is already running"),
    }
}

fn acp_error(error: acp::AcpError) -> CommandError {
    let code = match &error {
        acp::AcpError::Spawn(_) => "ai.spawn",
        acp::AcpError::Closed => "ai.closed",
        acp::AcpError::AuthRequired(_) => "ai.auth_required",
        acp::AcpError::Protocol(_) => "ai.protocol",
        acp::AcpError::Rpc { .. } => "ai.agent",
    };
    CommandError::new(code, error.to_string())
}

/// Parses `builtin:<id>` or `acp:<id>` from the WebView.
fn agent_ref(text: &str) -> Result<AgentRef, CommandError> {
    let invalid = || CommandError::new("input.invalid", "not an agent reference");
    let (kind, id) = text.split_once(':').ok_or_else(invalid)?;
    if id.is_empty()
        || id.len() > 48
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(invalid());
    }
    match kind {
        "builtin" => Ok(AgentRef::Builtin(id.to_owned())),
        "acp" => Ok(AgentRef::Acp(id.to_owned())),
        _ => Err(invalid()),
    }
}

/// Where the AI session works.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiContext {
    Local,
    Host { host_id: String },
    Quick { address: String },
}

/// An agent the user can talk to, as the panel lists it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentChoice {
    /// `builtin:<provider id>` or `acp:<agent id>`.
    pub agent: String,
    /// `builtin` or `acp`.
    pub kind: &'static str,
    pub name: String,
    /// The model, or the program that runs.
    pub detail: String,
    /// Runs through npx (downloaded on first use).
    pub via_npx: bool,
    /// An API key is stored on this device.
    pub has_key: bool,
}

/// The agents of this device, with the default.
#[derive(Debug, Serialize)]
pub struct AgentList {
    pub agents: Vec<AgentChoice>,
    pub default: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AiStarted {
    pub id: String,
    /// The host the session works on.
    pub host: String,
    pub agent: Option<String>,
    pub agent_version: Option<String>,
    pub modes: Vec<acp::SessionMode>,
    pub current_mode: Option<String>,
}

/// How an ACP agent runs.
struct AcpAgent {
    name: String,
    program: String,
    args: Vec<String>,
    known: Option<&'static detect::KnownAgent>,
    via_npx: bool,
}

/// The ACP agents of this device: detected (unless turned off), then added by command.
fn acp_agents(vault: &rove_vault::Vault) -> Result<Vec<(String, AcpAgent)>, CommandError> {
    let agents = vault.device_preferences()?.agents;
    let mut list: Vec<(String, AcpAgent)> = detect::detect(&detect::search_path())
        .into_iter()
        .filter(|found| !agents.disabled.contains(&found.id))
        .map(|found| {
            let known = detect::known(&found.id);
            (
                found.id,
                AcpAgent {
                    name: found.name,
                    program: found.program,
                    args: found.args,
                    known,
                    via_npx: found.via_npx,
                },
            )
        })
        .collect();
    for custom in agents.custom {
        list.push((
            custom.id,
            AcpAgent {
                name: custom.name,
                program: custom.command,
                args: custom.args,
                known: None,
                via_npx: false,
            },
        ));
    }
    Ok(list)
}

/// Lists the agents of this device: the built-in assistant with each provider, then ACP agents.
#[tauri::command]
pub async fn ai_agents(state: State<'_, AppState>) -> Result<AgentList, CommandError> {
    let prefs = state.vault.preferences()?;
    let keys = state.vault.ai_key_ids()?;
    let mut agents: Vec<AgentChoice> = prefs
        .ai
        .providers
        .iter()
        .map(|provider| {
            let agent = format!("builtin:{}", provider.id);
            AgentChoice {
                has_key: keys.contains(&agent),
                agent,
                kind: "builtin",
                name: provider.name.clone(),
                detail: provider.model.clone(),
                via_npx: false,
            }
        })
        .collect();
    // Looking at the PATH may run a login shell once: off the async threads.
    let vault = state.vault.clone();
    let found = tauri::async_runtime::spawn_blocking(move || acp_agents(&vault))
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))??;
    for (id, found) in found {
        let agent = format!("acp:{id}");
        let detail = [found.program, found.args.join(" ")]
            .join(" ")
            .trim()
            .to_owned();
        agents.push(AgentChoice {
            has_key: keys.contains(&agent),
            agent,
            kind: "acp",
            name: found.name,
            detail,
            via_npx: found.via_npx,
        });
    }
    Ok(AgentList {
        agents,
        default: prefs.ai.default_agent,
    })
}

/// Stores the API key of agent `agent` (`builtin:<id>` or `acp:<id>`) sealed on this device, or
/// removes it with `None`.
#[tauri::command]
pub async fn ai_set_key(
    state: State<'_, AppState>,
    agent: String,
    key: Option<String>,
) -> Result<(), CommandError> {
    let agent = agent_ref(&agent)?;
    let id = match agent {
        AgentRef::Builtin(id) => format!("builtin:{id}"),
        AgentRef::Acp(id) => format!("acp:{id}"),
    };
    state.ensure_unlocked().await?;
    let key = key.map(Zeroizing::new);
    Ok(state
        .vault
        .set_ai_key(&id, key.as_deref().map(String::as_str))?)
}

fn endpoint(provider: &ProviderPrefs) -> Endpoint {
    Endpoint {
        format: match provider.format {
            ProviderFormat::Chat => llm::Format::Chat,
            ProviderFormat::Responses => llm::Format::Responses,
            ProviderFormat::Anthropic => llm::Format::Anthropic,
        },
        base_url: provider.base_url.clone(),
        model: provider.model.clone(),
        max_tokens: provider.max_tokens,
        effort: provider.effort.map(|effort| match effort {
            ReasoningEffort::Low => llm::Effort::Low,
            ReasoningEffort::Medium => llm::Effort::Medium,
            ReasoningEffort::High => llm::Effort::High,
        }),
    }
}

/// Checks `provider` with a tiny request: its address, its model and `key` (the stored key of a
/// saved provider when `None`). Returns the model's reply.
#[tauri::command]
pub async fn ai_provider_test(
    state: State<'_, AppState>,
    provider: ProviderPrefs,
    key: Option<String>,
) -> Result<String, CommandError> {
    let mut prefs = rove_vault::Preferences::default();
    prefs.ai.providers = vec![provider];
    let provider = prefs.validate()?.ai.providers.remove(0);
    let key = match key.filter(|key| !key.trim().is_empty()) {
        Some(key) => Zeroizing::new(key.trim().to_owned()),
        None => {
            state.ensure_unlocked().await?;
            state
                .vault
                .ai_key(&format!("builtin:{}", provider.id))?
                .unwrap_or_default()
        }
    };
    let client = Client::new(endpoint(&provider), &key, Duration::from_secs(30))
        .map_err(|error| CommandError::new("ai.model", error.to_string()))?;
    let reply = client
        .complete(
            "Reply with the single word OK.",
            &[llm::Message::User {
                text: "ping".into(),
            }],
            &[],
            &mut |_| {},
        )
        .await
        .map_err(|error| CommandError::new("ai.model", error.to_string()))?;
    Ok(reply.text.chars().take(200).collect())
}

/// A remote host's live connection, from the app's pool.
struct Pooled {
    ssh: Arc<SshConnector>,
    target: SshTarget,
}

#[async_trait]
impl ConnectionSource for Pooled {
    async fn connection(&self) -> Option<Arc<SshConnection>> {
        self.ssh.live(&self.target).await
    }
}

/// The user's terminal, read from its replay buffer.
struct Screen {
    sessions: Arc<SessionManager>,
    id: SessionId,
}

#[async_trait]
impl TerminalSource for Screen {
    async fn recent(&self, lines: usize) -> Option<String> {
        let output = self.sessions.snapshot(self.id).ok()?;
        Some(rove_ai::screen::recent_text(&output, lines))
    }
}

fn mode(mode: AiMode) -> Mode {
    match mode {
        AiMode::AskByRisk => Mode::AskByRisk,
        AiMode::AskAll => Mode::AskAll,
        AiMode::Auto => Mode::Auto,
    }
}

/// The session's host and what the assistant is told about it.
fn host_of(
    state: &AppState,
    context: &AiContext,
) -> Result<(Arc<dyn Host>, HostMeta, PathBuf), CommandError> {
    Ok(match context {
        AiContext::Local => {
            let meta = HostMeta {
                name: "this computer".into(),
                address: "local".into(),
                system: Some(std::env::consts::OS.into()),
            };
            (
                Arc::new(LocalHost),
                meta,
                std::env::home_dir().unwrap_or_else(|| PathBuf::from(".")),
            )
        }
        AiContext::Host { host_id } => {
            let host = state.vault.host(host_id.parse()?)?;
            let target = host_target(state, &host)?;
            let label = format!("{} ({}@{})", host.name, target.username, target.hostname);
            let meta = HostMeta {
                name: host.name.clone(),
                address: format!("{}@{}:{}", target.username, target.hostname, target.port),
                system: host.os.as_ref().map(|os| {
                    if os.name.is_empty() {
                        os.id.clone()
                    } else {
                        os.name.clone()
                    }
                }),
            };
            let source = Arc::new(Pooled {
                ssh: state.ssh.clone(),
                target,
            });
            (
                Arc::new(RemoteHost::new(label, source)),
                meta,
                state.ai.workspace.clone(),
            )
        }
        AiContext::Quick { address } => {
            let target = quick_target(address)?;
            let label = target.address();
            let meta = HostMeta {
                name: label.clone(),
                address: label.clone(),
                system: None,
            };
            let source = Arc::new(Pooled {
                ssh: state.ssh.clone(),
                target,
            });
            (
                Arc::new(RemoteHost::new(label, source)),
                meta,
                state.ai.workspace.clone(),
            )
        }
    })
}

/// Starts `agent` (`builtin:<provider id>` or `acp:<agent id>`) working on `context`; `terminal`
/// is the user's terminal session there, which the agent may read.
#[tauri::command]
pub async fn ai_start(
    app: AppHandle,
    state: State<'_, AppState>,
    context: AiContext,
    agent: String,
    terminal: Option<String>,
) -> Result<AiStarted, CommandError> {
    let agent = agent_ref(&agent)?;
    state.ensure_unlocked().await?;
    let prefs = state.vault.preferences()?;
    let (host, meta, cwd) = host_of(state.inner(), &context)?;
    std::fs::create_dir_all(&cwd)
        .map_err(|error| CommandError::new("app.io", error.to_string()))?;

    let id = format!("ai-{}", uuid::Uuid::now_v7());
    let auditor = Arc::new(Auditor::new(
        {
            let recorder = state.ai.recorder.clone();
            Arc::new(move |record| recorder.record(record))
        },
        id.clone(),
        host.label(),
    ));
    let sink = {
        let app = app.clone();
        let auditor = auditor.clone();
        let id = id.clone();
        Arc::new(move |event: AiEvent| {
            auditor.event(&event);
            if let Err(error) = app.emit(AI_EVENT, serde_json::json!({ "id": id, "event": event }))
            {
                tracing::warn!(%error, "emitting an AI event failed");
            }
        })
    };
    let settings = SessionSettings {
        mode: mode(prefs.ai.mode),
        rules: Rules {
            read_only: prefs.ai.read_only.clone(),
            dangerous: prefs.ai.dangerous.clone(),
        },
        ..SessionSettings::default()
    };
    let session = Arc::new(AiSession::new(host.clone(), settings, sink));
    if let Some(terminal) = terminal {
        let id: SessionId = terminal.parse()?;
        session.set_terminal_source(Arc::new(Screen {
            sessions: state.sessions.clone(),
            id,
        }));
    }

    let (backend, mcp, started): (Arc<dyn AgentBackend>, _, AiStarted) = match agent {
        AgentRef::Builtin(provider_id) => {
            let provider = prefs
                .ai
                .providers
                .iter()
                .find(|provider| provider.id == provider_id)
                .ok_or_else(|| CommandError::new("ai.no_agent", "no such provider"))?;
            let key = state
                .vault
                .ai_key(&format!("builtin:{provider_id}"))?
                .unwrap_or_default();
            let client = Client::new(endpoint(provider), &key, MODEL_IDLE)
                .map_err(|error| CommandError::new("ai.model", error.to_string()))?;
            let assistant = Arc::new(Assistant::new(client, session, &meta));
            let started = AiStarted {
                id: id.clone(),
                host: host.label(),
                agent: Some(provider.name.clone()),
                agent_version: Some(provider.model.clone()),
                modes: Vec::new(),
                current_mode: None,
            };
            (assistant as Arc<dyn AgentBackend>, None, started)
        }
        AgentRef::Acp(agent_id) => {
            let vault = state.vault.clone();
            let found = tauri::async_runtime::spawn_blocking(move || acp_agents(&vault))
                .await
                .map_err(|error| CommandError::new("app.task", error.to_string()))??
                .into_iter()
                .find(|(id, _)| *id == agent_id)
                .map(|(_, agent)| agent)
                .ok_or_else(|| CommandError::new("ai.no_agent", "no such agent"))?;
            let device = state.vault.device_preferences()?;
            let key = match device.agents.auth {
                AgentAuth::Account => None,
                AgentAuth::Auto => state.vault.ai_key(&format!("acp:{agent_id}"))?,
                AgentAuth::ApiKey => Some(
                    state
                        .vault
                        .ai_key(&format!("acp:{agent_id}"))?
                        .ok_or_else(|| CommandError::new("ai.no_key", "no API key is stored"))?,
                ),
            };
            let mut extra = vec![("PATH".to_owned(), detect::search_path())];
            let auth_methods: Vec<String> = match (&key, found.known) {
                (Some(key), Some(known)) => {
                    for var in known.key_vars {
                        extra.push(((*var).to_owned(), key.to_string()));
                    }
                    known
                        .key_methods
                        .iter()
                        .map(|method| (*method).to_owned())
                        .collect()
                }
                (None, Some(known)) => known
                    .account_methods
                    .iter()
                    .map(|method| (*method).to_owned())
                    .collect(),
                (_, None) => Vec::new(),
            };
            let listener = McpListener::bind()
                .await
                .map_err(|error| CommandError::new("app.io", error.to_string()))?;
            let exe = std::env::current_exe()
                .map_err(|error| CommandError::new("app.io", error.to_string()))?;
            let mcp_server = McpServer {
                name: "roveterm".into(),
                command: exe.to_string_lossy().into_owned(),
                args: vec!["--mcp-bridge".into()],
                env: listener
                    .bridge_env()
                    .map_err(|error| CommandError::new("app.io", error.to_string()))?,
            };
            let spec = AgentSpec {
                program: found.program,
                args: found.args,
                env: acp::agent_env(acp::ENV_ALLOWED, &extra),
                cwd,
                auth_methods,
                mcp: vec![mcp_server],
                preferred_modes: PREFERRED_MODES
                    .iter()
                    .map(|mode| (*mode).to_owned())
                    .collect(),
            };
            // Serving first: the agent starts the MCP bridge before its session exists.
            let mcp = listener.serve(session.clone());
            let running = match runner::start(&spec, session).await {
                Ok(running) => running,
                Err(error) => {
                    mcp.abort();
                    return Err(acp_error(error));
                }
            };
            let info = running.started().clone();
            let started = AiStarted {
                id: id.clone(),
                host: host.label(),
                agent: info.agent.or(Some(found.name)),
                agent_version: info.agent_version,
                modes: info.modes,
                current_mode: info.current_mode,
            };
            (
                Arc::new(running) as Arc<dyn AgentBackend>,
                Some(mcp),
                started,
            )
        }
    };
    let mode_value = serde_json::to_value(prefs.ai.mode)
        .ok()
        .and_then(|mode| mode.as_str().map(str::to_owned))
        .unwrap_or_default();
    auditor.started(
        started.agent.as_deref(),
        started.agent_version.as_deref(),
        &mode_value,
    );
    state.ai.entries.lock().await.insert(
        id,
        Arc::new(Entry {
            backend,
            auditor,
            mcp,
        }),
    );
    Ok(started)
}

/// Sends `text` to session `id` and waits for the turn to end; returns the stop reason.
#[tauri::command]
pub async fn ai_prompt(
    state: State<'_, AppState>,
    id: String,
    text: String,
) -> Result<String, CommandError> {
    if text.trim().is_empty() || text.len() > MAX_PROMPT {
        return Err(CommandError::new(
            "input.invalid",
            "the message is empty or too long",
        ));
    }
    let entry = state.ai.get(&id).await?;
    entry.auditor.prompt(&text);
    let result = entry.backend.prompt(&text).await;
    let stop = match &result {
        Ok(reason) => reason.clone(),
        Err(error) => format!("error: {error}"),
    };
    entry.auditor.turn_ended(&stop);
    result.map_err(backend_error)
}

/// Stops the current turn of session `id`.
#[tauri::command]
pub async fn ai_cancel(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    state.ai.get(&id).await?.backend.cancel().await;
    Ok(())
}

/// Answers approval `request_id` of session `id`.
#[tauri::command]
pub async fn ai_answer(
    state: State<'_, AppState>,
    id: String,
    request_id: u64,
    answer: Answer,
) -> Result<bool, CommandError> {
    Ok(state
        .ai
        .get(&id)
        .await?
        .backend
        .session()
        .answer(request_id, answer)
        .await)
}

/// Changes the approval mode of session `id`.
#[tauri::command]
pub async fn ai_set_mode(
    state: State<'_, AppState>,
    id: String,
    mode: AiMode,
) -> Result<(), CommandError> {
    state
        .ai
        .get(&id)
        .await?
        .backend
        .session()
        .set_mode(self::mode(mode))
        .await;
    Ok(())
}

/// Ends session `id`: its running commands and the agent.
#[tauri::command]
pub async fn ai_stop(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let entry = state.ai.entries.lock().await.remove(&id);
    if let Some(entry) = entry {
        stop(&entry).await;
    }
    Ok(())
}

/// A past AI session, from the journal.
#[derive(Debug, Serialize)]
pub struct AiSessionSummary {
    pub id: String,
    /// Start time, Unix milliseconds.
    pub started: i64,
    pub host: Option<String>,
    pub agent: Option<String>,
}

/// One audit record of an AI session.
#[derive(Debug, Serialize)]
pub struct AiRecord {
    pub ts: i64,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Most sessions listed.
const MAX_SESSIONS: u32 = 100;
/// Most records read of one session.
const MAX_RECORDS: u32 = 5000;

/// Lists recent AI sessions, newest first.
#[tauri::command]
pub async fn ai_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<AiSessionSummary>, CommandError> {
    let journal = state.journal.clone();
    let records = tauri::async_runtime::spawn_blocking(move || {
        journal.read(&rove_journal::JournalQuery {
            kind: Some(rove_ai::audit::KIND_STARTED.to_owned()),
            limit: Some(MAX_SESSIONS),
            newest_first: true,
            ..rove_journal::JournalQuery::default()
        })
    })
    .await
    .map_err(|error| CommandError::new("app.task", error.to_string()))?
    .map_err(|error| CommandError::new("journal.read", error.to_string()))?;
    Ok(records
        .into_iter()
        .filter_map(|record| {
            Some(AiSessionSummary {
                id: record.session_id?,
                started: record.ts,
                host: record
                    .payload
                    .get("host")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned),
                agent: record
                    .payload
                    .get("agent")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned),
            })
        })
        .collect())
}

/// The audit records of AI session `id`, oldest first.
#[tauri::command]
pub async fn ai_records(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<AiRecord>, CommandError> {
    if !id.starts_with("ai-") || id.len() > 64 {
        return Err(CommandError::new("input.invalid", "not an AI session id"));
    }
    let journal = state.journal.clone();
    let records = tauri::async_runtime::spawn_blocking(move || {
        journal.read(&rove_journal::JournalQuery {
            session_id: Some(id),
            limit: Some(MAX_RECORDS),
            ..rove_journal::JournalQuery::default()
        })
    })
    .await
    .map_err(|error| CommandError::new("app.task", error.to_string()))?
    .map_err(|error| CommandError::new("journal.read", error.to_string()))?;
    Ok(records
        .into_iter()
        .filter(|record| record.kind.starts_with("ai/"))
        .map(|record| AiRecord {
            ts: record.ts,
            kind: record.kind,
            payload: record.payload,
        })
        .collect())
}
