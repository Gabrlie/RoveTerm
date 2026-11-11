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
use rove_ai::audit::{self, Auditor, SessionInfo};
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
/// An agent the user runs on their own started (`{id, context, agent, host}`) or ended (`{id,
/// ended: true}`) a conversation on a host.
pub const AI_EXTERNAL_EVENT: &str = "ai-external";
/// Longest prompt accepted.
const MAX_PROMPT: usize = 100_000;
/// Agent modes that make an ACP agent ask before its own tools change anything.
const PREFERRED_MODES: &[&str] = &["read-only", "read_only", "readonly", "default", "plan"];

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
        BackendError::Invalid(message) => CommandError::new("input.invalid", message),
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Offered in the panel; a detected agent the user turned off is listed for the settings only.
    pub enabled: bool,
    /// Added by the user's own command rather than detected.
    pub custom: bool,
}

/// The agents of this device, with the default.
#[derive(Debug, Serialize)]
pub struct AgentList {
    pub agents: Vec<AgentChoice>,
    pub default: Option<String>,
    /// The Jev reviewer's key is stored.
    pub jev_key: bool,
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
    /// Settings of the session the user can change (model, reasoning effort, …).
    pub config_options: Vec<acp::ConfigOption>,
    /// An earlier session was continued; for an ACP agent, whether it reopened its own session.
    pub resumed: bool,
}

/// How an ACP agent runs.
struct AcpAgent {
    name: String,
    program: String,
    args: Vec<String>,
    known: Option<&'static detect::KnownAgent>,
    via_npx: bool,
    enabled: bool,
    custom: bool,
}

/// The ACP agents of this device: detected (with whether the user turned them off), then added
/// by command.
fn acp_agents(vault: &rove_vault::Vault) -> Result<Vec<(String, AcpAgent)>, CommandError> {
    let agents = vault.device_preferences()?.agents;
    let mut list: Vec<(String, AcpAgent)> = detect::detect(&detect::search_path())
        .into_iter()
        .map(|found| {
            let known = detect::known(&found.id);
            let enabled = !agents.disabled.contains(&found.id);
            (
                found.id,
                AcpAgent {
                    name: found.name,
                    program: found.program,
                    args: found.args,
                    known,
                    via_npx: found.via_npx,
                    enabled,
                    custom: false,
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
                enabled: true,
                custom: true,
            },
        ));
    }
    Ok(list)
}

/// An enabled ACP agent of this device, with the environment and sign-in methods it starts
/// with: its stored key (per the sign-in setting) goes into the agent's key variables.
async fn acp_launch(
    state: &AppState,
    agent_id: &str,
) -> Result<(AcpAgent, Vec<(String, String)>, Vec<String>), CommandError> {
    let vault = state.vault.clone();
    let found = tauri::async_runtime::spawn_blocking(move || acp_agents(&vault))
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))??
        .into_iter()
        .find(|(id, agent)| id == agent_id && agent.enabled)
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
    Ok((found, extra, auth_methods))
}

/// Vault key of the Jev reviewer.
const JEV_KEY: &str = "review:jev";

/// The reviewer of the review mode as the preferences choose it, with its confidence threshold:
/// the session's model (for local agents, the default provider's), or Jev with its key. `None`
/// when it is off or cannot work (no provider, no key), so the user is asked as before.
fn reviewer_for(
    state: &AppState,
    prefs: &rove_vault::Preferences,
    agent: &AgentRef,
) -> Option<(Arc<dyn rove_ai::review::Reviewer>, f64)> {
    let threshold = f64::from(prefs.ai.review_confidence) / 100.0;
    let reviewer: Arc<dyn rove_ai::review::Reviewer> = match prefs.ai.reviewer {
        rove_vault::ReviewerKind::Off => return None,
        rove_vault::ReviewerKind::Model => {
            let wanted = match agent {
                AgentRef::Builtin(id) => Some(id.clone()),
                AgentRef::Acp(_) => prefs
                    .ai
                    .default_agent
                    .as_deref()
                    .and_then(|agent| agent.strip_prefix("builtin:"))
                    .map(str::to_owned),
            };
            let provider = prefs
                .ai
                .providers
                .iter()
                .find(|provider| Some(&provider.id) == wanted.as_ref())
                .or_else(|| prefs.ai.providers.first())?;
            let key = state
                .vault
                .ai_key(&format!("builtin:{}", provider.id))
                .ok()
                .flatten()
                .unwrap_or_default();
            let client = Client::new(endpoint(provider), &key, rove_ai::review::TIMEOUT)
                .map_err(|error| tracing::warn!(%error, "the model reviewer is unavailable"))
                .ok()?;
            Arc::new(rove_ai::review::ModelReviewer::new(client))
        }
        rove_vault::ReviewerKind::Jev => {
            let Some(key) = state.vault.ai_key(JEV_KEY).ok().flatten() else {
                tracing::warn!("the Jev reviewer has no key; the user is asked instead");
                return None;
            };
            let reviewer = rove_ai::review::JevReviewer::new(rove_ai::review::JEV_URL, &key)
                .map_err(|error| tracing::warn!(%error, "the Jev reviewer is unavailable"))
                .ok()?;
            Arc::new(reviewer)
        }
    };
    Some((reviewer, threshold))
}

/// Stores the Jev reviewer's API key sealed on this device (synced like the agents' keys), or
/// removes it with `None`.
#[tauri::command]
pub async fn ai_set_review_key(
    state: State<'_, AppState>,
    key: Option<String>,
) -> Result<(), CommandError> {
    state.ensure_unlocked().await?;
    if key.is_none() {
        tracing::info!(agent = JEV_KEY, "AI key removed by the user");
    }
    let key = key.map(Zeroizing::new);
    Ok(state
        .vault
        .set_ai_key(JEV_KEY, key.as_deref().map(String::as_str))?)
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
                enabled: true,
                custom: false,
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
            enabled: found.enabled,
            custom: found.custom,
        });
    }
    Ok(AgentList {
        agents,
        default: prefs.ai.default_agent,
        jev_key: keys.iter().any(|key| key == JEV_KEY),
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
    if key.is_none() {
        tracing::info!(agent = %id, "AI key removed by the user");
    }
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
        effort: Some(match provider.effort {
            ReasoningEffort::Minimal => llm::Effort::Minimal,
            ReasoningEffort::Low => llm::Effort::Low,
            ReasoningEffort::Medium => llm::Effort::Medium,
            ReasoningEffort::High => llm::Effort::High,
            ReasoningEffort::XHigh => llm::Effort::XHigh,
            ReasoningEffort::Max => llm::Effort::Max,
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

/// The models provider `provider` offers, to choose from; `key` is the typed key, else the one
/// stored for it.
#[tauri::command]
pub async fn ai_provider_models(
    state: State<'_, AppState>,
    provider: ProviderPrefs,
    key: Option<String>,
) -> Result<Vec<llm::ListedModel>, CommandError> {
    let key = match key.filter(|key| !key.trim().is_empty()) {
        Some(key) => Zeroizing::new(key.trim().to_owned()),
        // A locked vault reads as no key: the list may not need one.
        None => state
            .vault
            .ai_key(&format!("builtin:{}", provider.id))
            .ok()
            .flatten()
            .unwrap_or_default(),
    };
    // The model is not chosen yet: only the address and format matter here.
    let mut endpoint = endpoint(&provider);
    endpoint.model.clear();
    let client = Client::new(endpoint, &key, Duration::from_secs(30))
        .map_err(|error| CommandError::new("ai.model", error.to_string()))?;
    client
        .list_models()
        .await
        .map_err(|error| CommandError::new("ai.model", error.to_string()))
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
    resume: Option<String>,
) -> Result<AiStarted, CommandError> {
    let agent_text = agent;
    let agent = agent_ref(&agent_text)?;
    state.ensure_unlocked().await?;
    let prefs = state.vault.preferences()?;
    let (host, meta, cwd) = host_of(state.inner(), &context)?;
    std::fs::create_dir_all(&cwd)
        .map_err(|error| CommandError::new("app.io", error.to_string()))?;

    // Resuming continues the same session id, with the agent it had.
    let earlier = match &resume {
        Some(id) => {
            check_session_id(id)?;
            if state.ai.entries.lock().await.contains_key(id) {
                return Err(CommandError::new("ai.busy", "the conversation is open"));
            }
            let records = session_records(state.inner(), id.clone()).await?;
            let info = records
                .iter()
                .rev()
                .find(|record| record.kind == audit::KIND_STARTED)
                .and_then(|record| {
                    serde_json::from_value::<SessionInfo>(record.payload.clone()).ok()
                })
                .ok_or_else(|| CommandError::new("input.invalid", "no such conversation"))?;
            if info.agent_ref.as_deref() != Some(agent_text.as_str()) {
                return Err(CommandError::new(
                    "input.invalid",
                    "the conversation was with another agent",
                ));
            }
            let titled = records
                .iter()
                .any(|record| record.kind == audit::KIND_RENAMED);
            Some((id.clone(), info, records, titled))
        }
        None => None,
    };
    let id = earlier.as_ref().map_or_else(
        || format!("ai-{}", uuid::Uuid::now_v7()),
        |(id, ..)| id.clone(),
    );
    let auditor = Arc::new(Auditor::new(
        {
            let recorder = state.ai.recorder.clone();
            Arc::new(move |record| recorder.record(record))
        },
        id.clone(),
        host.label(),
    ));
    if earlier.as_ref().is_some_and(|(.., titled)| *titled) {
        auditor.set_titled();
    }
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
    session.set_reviewer(reviewer_for(state.inner(), &prefs, &agent));
    if let Some(terminal) = terminal {
        let id: SessionId = terminal.parse()?;
        session.set_terminal_source(Arc::new(Screen {
            sessions: state.sessions.clone(),
            id,
        }));
    }

    let mut agent_session = None;
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
            let idle = Duration::from_secs(u64::from(prefs.ai.request_timeout_secs));
            let client = Client::new(endpoint(provider), &key, idle)
                .map_err(|error| CommandError::new("ai.model", error.to_string()))?
                .with_cache_key(&id);
            let assistant = Assistant::new(client, session, &meta)
                .with_models(&provider.models)
                .with_max_steps(prefs.ai.max_steps as usize)
                .with_contexts(provider.contexts.clone(), provider.context_window);
            if let Some((_, _, records, _)) = &earlier {
                let records: Vec<(String, serde_json::Value)> = records
                    .iter()
                    .map(|record| (record.kind.clone(), record.payload.clone()))
                    .collect();
                assistant.seed(audit::transcript(&records)).await;
            }
            let started = AiStarted {
                id: id.clone(),
                host: host.label(),
                agent: Some(provider.name.clone()),
                agent_version: Some(provider.model.clone()),
                modes: Vec::new(),
                current_mode: None,
                config_options: assistant.config_options(),
                resumed: earlier.is_some(),
            };
            (Arc::new(assistant) as Arc<dyn AgentBackend>, None, started)
        }
        AgentRef::Acp(agent_id) => {
            let (found, extra, auth_methods) = acp_launch(state.inner(), &agent_id).await?;
            let listener = McpListener::bind()
                .await
                .map_err(|error| CommandError::new("app.io", error.to_string()))?;
            let exe = std::env::current_exe()
                .map_err(|error| CommandError::new("app.io", error.to_string()))?;
            let mcp_server = McpServer {
                name: rove_ai::mcp::SERVER_NAME.into(),
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
                resume: earlier
                    .as_ref()
                    .and_then(|(_, info, ..)| info.agent_session.clone()),
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
                // Settings the user switches here; the mode is RoveTerm's approval mode.
                config_options: info
                    .config_options
                    .into_iter()
                    .filter(|option| option.category.as_deref() != Some("mode"))
                    .collect(),
                resumed: info.resumed,
            };
            agent_session = info.agent_session;
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
    auditor.started(&SessionInfo {
        agent: started.agent.clone(),
        agent_version: started.agent_version.clone(),
        mode: mode_value,
        agent_ref: Some(agent_text),
        context: serde_json::to_value(&context).ok(),
        agent_session,
        resumed: earlier.is_some(),
    });
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

/// The backend of a conversation an agent the user runs drives through RoveTerm's MCP server:
/// there is nothing to prompt here; stopping withdraws its approvals and stops its commands.
struct ExternalBackend(Arc<AiSession>);

#[async_trait]
impl AgentBackend for ExternalBackend {
    async fn prompt(&self, _text: &str) -> Result<String, BackendError> {
        Err(BackendError::Invalid(
            "this conversation is driven by an agent outside RoveTerm".into(),
        ))
    }

    async fn cancel(&self) {
        self.0.withdraw_approvals().await;
    }

    async fn stop(&self) {
        self.0.withdraw_approvals().await;
        self.0.stop_jobs().await;
    }

    fn session(&self) -> &Arc<AiSession> {
        &self.0
    }

    async fn set_config_option(
        &self,
        _id: &str,
        _value: &str,
    ) -> Result<Vec<acp::ConfigOption>, BackendError> {
        Err(BackendError::Invalid(
            "this conversation has no settings here".into(),
        ))
    }
}

/// Starts a conversation for `client` (an agent the user runs, by the name it gave) on saved
/// host `host_id`: a session with the user's approval rules and reviewer, recorded like any
/// other, and announced to the UI. Returns its id and session.
pub(crate) async fn start_external(
    app: &AppHandle,
    state: &AppState,
    client: &str,
    host_id: rove_vault::HostId,
) -> Result<(String, Arc<AiSession>), CommandError> {
    state.ensure_unlocked().await?;
    let prefs = state.vault.preferences()?;
    let context = AiContext::Host {
        host_id: host_id.to_string(),
    };
    let (host, _meta, _cwd) = host_of(state, &context)?;
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
    session.set_reviewer(reviewer_for(
        state,
        &prefs,
        &AgentRef::Acp("external".into()),
    ));
    let mode_value = serde_json::to_value(prefs.ai.mode)
        .ok()
        .and_then(|mode| mode.as_str().map(str::to_owned))
        .unwrap_or_default();
    // No agent reference: the conversation cannot be continued from RoveTerm.
    auditor.started(&SessionInfo {
        agent: Some(client.to_owned()),
        agent_version: None,
        mode: mode_value,
        agent_ref: None,
        context: serde_json::to_value(&context).ok(),
        agent_session: None,
        resumed: false,
    });
    state.ai.entries.lock().await.insert(
        id.clone(),
        Arc::new(Entry {
            backend: Arc::new(ExternalBackend(session.clone())),
            auditor,
            mcp: None,
        }),
    );
    tracing::info!(session = %id, agent = client, "an outside agent started working on a host");
    if let Err(error) = app.emit(
        AI_EXTERNAL_EVENT,
        serde_json::json!({ "id": id, "context": context, "agent": client, "host": host.label() }),
    ) {
        tracing::warn!(%error, "announcing an outside agent's conversation failed");
    }
    Ok((id, session))
}

/// Ends conversation `id` of an agent the user runs (its connection closed).
pub(crate) async fn end_external(app: &AppHandle, state: &AppState, id: &str) {
    let entry = state.ai.entries.lock().await.remove(id);
    if let Some(entry) = entry {
        stop(&entry).await;
    }
    if let Err(error) = app.emit(
        AI_EXTERNAL_EVENT,
        serde_json::json!({ "id": id, "ended": true }),
    ) {
        tracing::warn!(%error, "announcing the end of an outside agent's conversation failed");
    }
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

/// Deletes conversation `id` at the user's request: stops it when it runs, removes its `ai/*`
/// records from the journal and leaves an [`audit::KIND_DELETED`] record, which sync passes on
/// so the other devices delete it too.
#[tauri::command]
pub async fn ai_delete(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    check_session_id(&id)?;
    let entry = state.ai.entries.lock().await.remove(&id);
    if let Some(entry) = entry {
        stop(&entry).await;
    }
    let recorder = state.ai.recorder.clone();
    let journal = state.journal.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Records still queued for it are written first, then removed with the rest.
        recorder.flush();
        delete_conversation(journal.as_ref(), &id)
    })
    .await
    .map_err(|error| CommandError::new("app.task", error.to_string()))?
    .map_err(|error| CommandError::new("journal.io", error.to_string()))
}

/// Removes conversation `id`'s records and marks it deleted (once).
pub(crate) fn delete_conversation(
    journal: &dyn rove_journal::JournalStore,
    id: &str,
) -> Result<(), rove_journal::JournalError> {
    journal.delete_session(id, "ai/")?;
    if !deleted_conversations(journal)?.contains(id) {
        journal.append(rove_journal::NewRecord {
            kind: audit::KIND_DELETED.to_owned(),
            session_id: None,
            payload: serde_json::json!({ "session": id }),
        })?;
    }
    Ok(())
}

/// Conversations the user deleted (on this device or another).
pub(crate) fn deleted_conversations(
    journal: &dyn rove_journal::JournalStore,
) -> Result<std::collections::HashSet<String>, rove_journal::JournalError> {
    Ok(journal
        .read(&rove_journal::JournalQuery {
            kind: Some(audit::KIND_DELETED.to_owned()),
            limit: Some(10_000),
            ..rove_journal::JournalQuery::default()
        })?
        .into_iter()
        .filter_map(|record| {
            record
                .payload
                .get("session")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect())
}

/// A past AI session, from the journal.
#[derive(Debug, Serialize)]
pub struct AiSessionSummary {
    pub id: String,
    /// The latest start, Unix milliseconds.
    pub started: i64,
    pub host: Option<String>,
    pub agent: Option<String>,
    /// The title given by the user, or taken from the first prompt.
    pub title: Option<String>,
    /// The agent, to resume with (absent for sessions recorded before resuming existed).
    pub agent_ref: Option<String>,
    /// The host context, to resume on.
    pub context: Option<serde_json::Value>,
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

/// Refuses anything but an AI session id.
fn check_session_id(id: &str) -> Result<(), CommandError> {
    if !id.starts_with("ai-") || id.len() > 64 {
        return Err(CommandError::new("input.invalid", "not an AI session id"));
    }
    Ok(())
}

/// The `ai/*` records of session `id`, oldest first (by time: records synced from another
/// device are appended later but keep their own time).
async fn session_records(
    state: &AppState,
    id: String,
) -> Result<Vec<rove_journal::JournalRecord>, CommandError> {
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
    let mut records: Vec<rove_journal::JournalRecord> = records
        .into_iter()
        .filter(|record| record.kind.starts_with("ai/"))
        .collect();
    records.sort_by_key(|record| record.ts);
    Ok(records)
}

/// The newest records of `kind`.
async fn newest(
    state: &AppState,
    kind: &str,
    limit: u32,
) -> Result<Vec<rove_journal::JournalRecord>, CommandError> {
    let journal = state.journal.clone();
    let kind = kind.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        journal.read(&rove_journal::JournalQuery {
            kind: Some(kind),
            limit: Some(limit),
            newest_first: true,
            ..rove_journal::JournalQuery::default()
        })
    })
    .await
    .map_err(|error| CommandError::new("app.task", error.to_string()))?
    .map_err(|error| CommandError::new("journal.read", error.to_string()))
}

/// Lists recent AI sessions, newest first; a resumed session is listed once, by its latest start.
#[tauri::command]
pub async fn ai_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<AiSessionSummary>, CommandError> {
    let starts = newest(state.inner(), audit::KIND_STARTED, MAX_SESSIONS * 4).await?;
    let names = newest(state.inner(), audit::KIND_RENAMED, MAX_SESSIONS * 20).await?;
    let mut titles: HashMap<String, String> = HashMap::new();
    for record in names {
        let (Some(id), Some(title)) = (
            record.session_id,
            record.payload.get("title").and_then(|value| value.as_str()),
        ) else {
            continue;
        };
        // Newest first: the first title seen is the current one.
        titles.entry(id).or_insert_with(|| title.to_owned());
    }
    let mut seen = std::collections::HashSet::new();
    Ok(starts
        .into_iter()
        .filter_map(|record| {
            let id = record.session_id?;
            if !seen.insert(id.clone()) {
                return None;
            }
            let text = |key: &str| {
                record
                    .payload
                    .get(key)
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            };
            Some(AiSessionSummary {
                title: titles.remove(&id),
                started: record.ts,
                host: text("host"),
                agent: text("agent"),
                agent_ref: text("agent_ref"),
                context: record
                    .payload
                    .get("context")
                    .filter(|value| !value.is_null())
                    .cloned(),
                id,
            })
        })
        .collect::<Vec<_>>())
    .map(|mut sessions| {
        // Synced conversations are appended late but keep their own time.
        sessions.sort_by_key(|session| std::cmp::Reverse(session.started));
        sessions.truncate(MAX_SESSIONS as usize);
        sessions
    })
}

/// The audit records of AI session `id`, oldest first.
#[tauri::command]
pub async fn ai_records(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<AiRecord>, CommandError> {
    check_session_id(&id)?;
    Ok(session_records(state.inner(), id)
        .await?
        .into_iter()
        .map(|record| AiRecord {
            ts: record.ts,
            kind: record.kind,
            payload: record.payload,
        })
        .collect())
}

/// Names AI session `id` (running or not).
#[tauri::command]
pub async fn ai_rename(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<String, CommandError> {
    check_session_id(&id)?;
    let title = title.trim().to_owned();
    if title.is_empty()
        || title.chars().count() > audit::MAX_TITLE
        || title.chars().any(char::is_control)
    {
        return Err(CommandError::new(
            "input.invalid",
            "the title is empty or too long",
        ));
    }
    if let Ok(entry) = state.ai.get(&id).await {
        entry.auditor.set_titled();
    }
    state.ai.recorder.record(audit::renamed(&id, &title, false));
    Ok(title)
}

/// Sets setting `option` of session `id` (such as `model`) to `value`; returns the settings as
/// they now are.
#[tauri::command]
pub async fn ai_set_option(
    state: State<'_, AppState>,
    id: String,
    option: String,
    value: String,
) -> Result<Vec<acp::ConfigOption>, CommandError> {
    if option.is_empty() || option.len() > 100 || value.is_empty() || value.len() > 500 {
        return Err(CommandError::new("input.invalid", "not a setting"));
    }
    let options = state
        .ai
        .get(&id)
        .await?
        .backend
        .set_config_option(&option, &value)
        .await
        .map_err(backend_error)?;
    Ok(options
        .into_iter()
        .filter(|option| option.category.as_deref() != Some("mode"))
        .collect())
}

/// Plan usage of a local agent, as far as it reports it.
#[derive(Debug, Clone, Serialize)]
pub struct AiQuota {
    pub windows: Vec<rove_ai::quota::Window>,
    /// How full the context of the agent's latest session is, when its logs say (Codex).
    pub context: Option<rove_ai::quota::ContextUse>,
    /// When it was read (Unix milliseconds).
    pub as_of: i64,
}

/// How long a read quota is reused before reading it again.
const QUOTA_REUSE: Duration = Duration::from_secs(300);
/// Longest the Claude Code probe may take to start, and to answer `/usage`.
const QUOTA_PROBE: Duration = Duration::from_secs(30);

static QUOTAS: std::sync::Mutex<Option<HashMap<String, (std::time::Instant, AiQuota)>>> =
    std::sync::Mutex::new(None);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// The rate-limit windows of local agent `agent` (Codex: from its session logs, with the context
/// of its latest session; Claude Code: from its `/usage` report, which starts it briefly);
/// `None` for agents that report none. A recent Claude Code answer is reused unless `refresh`.
#[tauri::command]
pub async fn ai_quota(
    state: State<'_, AppState>,
    agent: String,
    refresh: bool,
) -> Result<Option<AiQuota>, CommandError> {
    let AgentRef::Acp(agent_id) = agent_ref(&agent)? else {
        return Ok(None);
    };
    if agent_id == "codex" {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::home_dir().map(|home| home.join(".codex")))
            .ok_or_else(|| CommandError::new("app.io", "no home directory"))?;
        // Reading the logs is cheap, and the context changes with every turn: never reused.
        let (windows, context) = tauri::async_runtime::spawn_blocking(move || {
            (
                rove_ai::quota::codex_quota(&home, now_ms()).unwrap_or_default(),
                rove_ai::quota::codex_latest_context(&home),
            )
        })
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))?;
        return Ok(Some(AiQuota {
            windows,
            context,
            as_of: now_ms(),
        }));
    }
    if !refresh
        && let Some((at, quota)) = QUOTAS
            .lock()
            .ok()
            .and_then(|cache| cache.as_ref()?.get(&agent_id).cloned())
        && at.elapsed() < QUOTA_REUSE
    {
        return Ok(Some(quota));
    }
    let windows = match agent_id.as_str() {
        "claude" => claude_usage(state.inner(), &agent_id).await?,
        _ => return Ok(None),
    };
    let quota = AiQuota {
        windows,
        context: None,
        as_of: now_ms(),
    };
    if let Ok(mut cache) = QUOTAS.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(agent_id, (std::time::Instant::now(), quota.clone()));
    }
    Ok(Some(quota))
}

/// Starts Claude Code on its own, without tools, asks its local `/usage` command (which does not
/// reach the model) and reads the limits from the report.
async fn claude_usage(
    state: &AppState,
    agent_id: &str,
) -> Result<Vec<rove_ai::quota::Window>, CommandError> {
    let (found, extra, auth_methods) = acp_launch(state, agent_id).await?;
    let report = Arc::new(std::sync::Mutex::new(String::new()));
    let sink = {
        let report = report.clone();
        Arc::new(move |event: AiEvent| {
            if let AiEvent::Update {
                update: acp::SessionUpdate::Message { text, .. },
            } = event
                && let Ok(mut report) = report.lock()
            {
                report.push_str(&text);
            }
        })
    };
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings::default(),
        sink,
    ));
    let spec = AgentSpec {
        program: found.program,
        args: found.args,
        env: acp::agent_env(acp::ENV_ALLOWED, &extra),
        cwd: std::env::home_dir().unwrap_or_else(std::env::temp_dir),
        auth_methods,
        mcp: Vec::new(),
        preferred_modes: Vec::new(),
        resume: None,
    };
    let running = tokio::time::timeout(QUOTA_PROBE, runner::start(&spec, session))
        .await
        .map_err(|_| CommandError::new("ai.timeout", "the agent did not start in time"))?
        .map_err(acp_error)?;
    let answered = tokio::time::timeout(QUOTA_PROBE, running.prompt("/usage")).await;
    running.stop().await;
    match answered {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => return Err(acp_error(error)),
        Err(_) => return Err(CommandError::new("ai.timeout", "no usage report in time")),
    }
    let text = report.lock().map(|text| text.clone()).unwrap_or_default();
    Ok(rove_ai::quota::claude_limits(&text))
}
