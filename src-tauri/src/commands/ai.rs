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
use tauri::{AppHandle, Emitter, Manager, State};
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
    builtin: Option<Arc<InteractiveAssistant>>,
    backend: Arc<dyn AgentBackend>,
    auditor: Arc<Auditor>,
    /// The built-in MCP server of an ACP agent.
    mcp: Option<tokio::task::JoinHandle<()>>,
    /// The fleet lead conversation that started this sub-agent.
    parent: Option<String>,
}

/// A model change takes the write lock between complete turns.
struct InteractiveAssistant {
    assistant: tokio::sync::RwLock<Assistant>,
    session: Arc<AiSession>,
    provider: Arc<std::sync::RwLock<ProviderPrefs>>,
    info: std::sync::Mutex<SessionInfo>,
    ops: Option<Arc<crate::ops_tools::OpsTools>>,
}

#[async_trait]
impl AgentBackend for InteractiveAssistant {
    async fn prompt(&self, text: &str) -> Result<String, BackendError> {
        Ok(self.assistant.read().await.prompt(text).await?)
    }
    async fn cancel(&self) {
        self.assistant.read().await.cancel().await;
    }
    async fn stop(&self) {
        self.cancel().await;
        self.session.stop_jobs().await;
        let _idle = self.assistant.write().await;
    }
    fn session(&self) -> &Arc<AiSession> {
        &self.session
    }
    async fn set_config_option(
        &self,
        id: &str,
        value: &str,
    ) -> Result<Vec<acp::ConfigOption>, BackendError> {
        self.assistant
            .try_write()
            .map_err(|_| BackendError::Busy)?
            .set_config_option(id, value)
            .await
    }
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

fn audit_sink(
    vault: Arc<rove_vault::Vault>,
    journal: Arc<dyn rove_journal::JournalStore>,
) -> audit::RecordSink {
    Arc::new(move |mut record| {
        let mut summary = record.payload.clone();
        let result = if audit::tool_summary(&mut summary) {
            let reference = uuid::Uuid::now_v7();
            let session = record.session_id.clone().unwrap_or_default();
            let sealed = vault.seal_audit(&session, reference, &record.payload);
            record.payload = summary;
            match sealed {
                Ok(sealed) => {
                    record.payload["body_ref"] = serde_json::json!(reference);
                    journal.append_private(record, &session, &reference.to_string(), &sealed)
                }
                Err(error) => {
                    tracing::error!(%error, "could not seal audit detail");
                    record.payload["detail_unavailable"] = true.into();
                    journal.append(record).map(|_| ())
                }
            }
        } else {
            journal.append(record).map(|_| ())
        };
        if let Err(error) = result {
            tracing::error!(%error, "could not persist audit record");
        }
    })
}

fn lead_checkpoint_sink(
    app: AppHandle,
    session: String,
    audit: crate::ops_tools::AuditGuard,
) -> rove_ai::assistant::CheckpointSink {
    Arc::new(move |checkpoint| {
        audit.check()?;
        let result = (|| {
            let state = app.state::<AppState>();
            let value = serde_json::to_value(checkpoint)
                .map_err(|_| "checkpoint encoding failed".to_owned())?;
            let reference = crate::ops_runtime::save_private(&state, &session, &value)
                .map_err(|_| "checkpoint persistence failed".to_owned())?;
            state
                .journal
                .append(rove_journal::NewRecord {
                    kind: "ops/lead_checkpoint".into(),
                    session_id: Some(session.clone()),
                    payload: serde_json::to_value(reference)
                        .map_err(|_| "checkpoint reference failed".to_owned())?,
                })
                .map_err(|_| "checkpoint persistence failed".to_owned())?;
            Ok(())
        })();
        if result.is_err() {
            audit.fail();
            app.state::<AppState>().ops.halt_on_storage_failure();
        }
        result
    })
}

fn reviewed_lead_checkpoint(
    mut checkpoint: rove_ai::assistant::AssistantCheckpoint,
    confirmed: bool,
    tasks: &serde_json::Value,
) -> Result<rove_ai::assistant::AssistantCheckpoint, CommandError> {
    if checkpoint.pending_tool.is_some() {
        if !confirmed {
            return Err(CommandError::new(
                "ops.recovery_required",
                "review the saved task states before resuming the interrupted operations lead",
            ));
        }
        checkpoint.resolve_interrupted();
        checkpoint.messages.push(llm::Message::User {
            text: format!(
                "The user has reviewed the interrupted operations lead's saved task states. The prior tool invocation must not be replayed. Inspect these persisted tasks and continue existing task IDs before creating any new work.\n{}",
                rove_ai::assistant::untrusted("operations-recovery-state", &tasks.to_string()),
            ),
        });
    }
    Ok(checkpoint)
}

fn latest_lead_checkpoint(
    state: &AppState,
    session: &str,
) -> Result<Option<rove_ai::assistant::AssistantCheckpoint>, CommandError> {
    let record = state
        .journal
        .read(&rove_journal::JournalQuery {
            session_id: Some(session.to_owned()),
            kind: Some("ops/lead_checkpoint".into()),
            limit: Some(1),
            newest_first: true,
            ..Default::default()
        })
        .map_err(|_| CommandError::new("ops.store", "checkpoint could not be read"))?;
    record
        .first()
        .map(|record| {
            let reference: rove_ops::PrivateRef = serde_json::from_value(record.payload.clone())
                .map_err(|_| CommandError::new("ops.checkpoint", "invalid checkpoint reference"))?;
            crate::ops_runtime::read_private(state, &reference)
        })
        .transpose()
}

#[tauri::command]
pub async fn ai_audit_detail(
    state: State<'_, AppState>,
    id: String,
    reference: uuid::Uuid,
) -> Result<serde_json::Value, CommandError> {
    check_session_id(&id)?;
    state.ensure_unlocked().await?;
    let sealed = state
        .journal
        .private_body(&id, &reference.to_string())
        .map_err(|e| CommandError::new("journal.read", e.to_string()))?;
    Ok(state.vault.open_audit(&id, reference, &sealed)?)
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
    Host {
        host_id: String,
    },
    Quick {
        address: String,
    },
    /// All saved hosts: the fleet lead, which hands work to a sub-agent per host.
    Fleet,
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

/// The judge of the auto mode as the preferences choose it, with its confidence threshold: the
/// session's model (for local agents, the default provider's), a chosen provider's model, or Jev
/// with its key. `None` when it cannot work (no provider, no key): the user is asked instead.
fn reviewer_for(
    state: &AppState,
    prefs: &rove_vault::Preferences,
    agent: &AgentRef,
) -> Option<(Arc<dyn rove_ai::review::Reviewer>, f64)> {
    let threshold = f64::from(prefs.ai.review_confidence) / 100.0;
    // A judge on `provider`'s model `model` (its default when `None`).
    let on_model = |provider: &ProviderPrefs, model: Option<&str>| {
        let key = state
            .vault
            .ai_key(&format!("builtin:{}", provider.id))
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut endpoint = endpoint(provider);
        if let Some(model) = model {
            model.clone_into(&mut endpoint.model);
        }
        Client::new(endpoint, &key, rove_ai::review::TIMEOUT)
            .map_err(|error| tracing::warn!(%error, "the model judge is unavailable"))
            .ok()
            .map(|client| {
                Arc::new(rove_ai::review::ModelReviewer::new(client))
                    as Arc<dyn rove_ai::review::Reviewer>
            })
    };
    let reviewer: Arc<dyn rove_ai::review::Reviewer> = match prefs.ai.reviewer {
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
            let Some(provider) = prefs
                .ai
                .providers
                .iter()
                .find(|provider| Some(&provider.id) == wanted.as_ref())
                .or_else(|| prefs.ai.providers.first())
            else {
                tracing::warn!(
                    "no model provider for the auto mode's judge; the user is asked instead"
                );
                return None;
            };
            on_model(provider, None)?
        }
        rove_vault::ReviewerKind::Chosen => {
            let chosen = prefs.ai.review_model.as_ref()?;
            let Some(provider) = prefs
                .ai
                .providers
                .iter()
                .find(|provider| provider.id == chosen.provider)
            else {
                tracing::warn!(
                    "the auto mode's judge names a removed provider; the user is asked instead"
                );
                return None;
            };
            on_model(provider, Some(&chosen.model))?
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

/// A remote host's connection from the app's pool.
enum Pooled {
    /// A saved host: the live connection, else a new one made in the background with the
    /// host's stored credentials (never asking for a password or passphrase). It is shared, so a
    /// terminal opened later reuses it.
    Saved {
        app: AppHandle,
        host: rove_vault::HostId,
    },
    /// A quick-connect address has no stored credentials: only its live connection.
    Live {
        ssh: Arc<SshConnector>,
        target: SshTarget,
    },
}

#[async_trait]
impl ConnectionSource for Pooled {
    async fn connection(&self) -> Result<Arc<SshConnection>, String> {
        let (app, host) = match self {
            Self::Live { ssh, target } => {
                return ssh.live(target).await.ok_or_else(|| {
                    "it is not connected; ask the user to open a terminal to it".to_owned()
                });
            }
            Self::Saved { app, host } => (app, *host),
        };
        let state = app.state::<AppState>();
        let source = crate::connect::Source::Host {
            host_id: host,
            typed: None,
            remember: false,
        };
        crate::connect::with_connection(&state, &source, None, |connection| async move {
            Ok(connection)
        })
        .await
        .map_err(|error| match error.code {
            "ssh.credentials_required" | "ssh.passphrase_required" => {
                "it needs a password or key passphrase that is not saved; ask the user to connect to it once in a terminal and save the password".to_owned()
            }
            _ => error.message,
        })
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

pub(crate) fn mode(mode: AiMode) -> Mode {
    match mode {
        AiMode::AskByRisk => Mode::AskByRisk,
        AiMode::AskAll => Mode::AskAll,
        AiMode::Auto => Mode::Auto,
    }
}

/// The notes of a saved host, in the vault.
struct VaultNotes {
    vault: Arc<rove_vault::Vault>,
    host: rove_vault::HostId,
}

#[async_trait]
impl rove_ai::session::NotesSource for VaultNotes {
    async fn read(&self) -> Result<String, String> {
        self.vault
            .host_notes(self.host)
            .map_err(|error| error.to_string())
    }

    async fn write(&self, text: &str) -> Result<(), String> {
        self.vault
            .set_host_notes(self.host, text)
            .map_err(|error| error.to_string())
    }
}

/// Lets `session` keep the notes of `context`'s host, when it is a saved host.
pub(crate) fn attach_notes(state: &AppState, session: &AiSession, context: &AiContext) {
    if let AiContext::Host { host_id } = context
        && let Ok(host) = host_id.parse()
    {
        session.set_notes_source(Arc::new(VaultNotes {
            vault: state.vault.clone(),
            host,
        }));
    }
}

/// The session's host and what the assistant is told about it.
fn host_of(
    app: &AppHandle,
    state: &AppState,
    context: &AiContext,
) -> Result<(Arc<dyn Host>, HostMeta, PathBuf), CommandError> {
    Ok(match context {
        AiContext::Local => {
            let meta = HostMeta {
                name: "this computer".into(),
                address: "local".into(),
                system: Some(std::env::consts::OS.into()),
                notes: None,
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
                notes: state
                    .vault
                    .host_notes(host.id)
                    .ok()
                    .filter(|notes| !notes.is_empty()),
            };
            let source = Arc::new(Pooled::Saved {
                app: app.clone(),
                host: host.id,
            });
            (
                Arc::new(RemoteHost::new(label, source)),
                meta,
                state.ai.workspace.clone(),
            )
        }
        // The fleet lead works through sub-agents, never on a host itself.
        AiContext::Fleet => (
            Arc::new(LocalHost) as Arc<dyn Host>,
            HostMeta {
                name: "all saved hosts".into(),
                address: "fleet".into(),
                system: None,
                notes: None,
            },
            state.ai.workspace.clone(),
        ),
        AiContext::Quick { address } => {
            let target = quick_target(address)?;
            let label = target.address();
            let meta = HostMeta {
                name: label.clone(),
                address: label.clone(),
                system: None,
                notes: None,
            };
            let source = Arc::new(Pooled::Live {
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
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri binds individually named IPC fields"
)]
pub async fn ai_start(
    app: AppHandle,
    state: State<'_, AppState>,
    context: AiContext,
    agent: String,
    terminal: Option<String>,
    resume: Option<String>,
    model: Option<String>,
    resume_confirmed: Option<bool>,
) -> Result<AiStarted, CommandError> {
    let agent_text = agent;
    let agent = agent_ref(&agent_text)?;
    state.ensure_unlocked().await?;
    let prefs = state.vault.preferences()?;
    let (host, meta, cwd) = host_of(&app, state.inner(), &context)?;
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
            if info.context != serde_json::to_value(&context).ok()
                || (info.agent_ref.as_deref() != Some(agent_text.as_str())
                    && !(model.is_some()
                        && matches!(&agent, AgentRef::Builtin(_))
                        && info
                            .agent_ref
                            .as_deref()
                            .is_some_and(|a| a.starts_with("builtin:"))))
            {
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
        audit_sink(state.vault.clone(), state.journal.clone()),
        id.clone(),
        host.label(),
    ));
    if matches!(context, AiContext::Fleet)
        || earlier.as_ref().is_some_and(|(_, _, rows, _)| {
            rows.iter().any(|row| row.payload["private_text"] == true)
        })
    {
        auditor.protect_text();
    }
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
    attach_notes(state.inner(), &session, &context);
    if let Some(terminal) = terminal {
        let id: SessionId = terminal.parse()?;
        session.set_terminal_source(Arc::new(Screen {
            sessions: state.sessions.clone(),
            id,
        }));
    }

    let mut builtin = None;
    let mut operations = None;
    let mut agent_session = None;
    let (backend, mcp, started): (Arc<dyn AgentBackend>, _, AiStarted) = match agent {
        AgentRef::Builtin(provider_id) => {
            let provider = prefs
                .ai
                .providers
                .iter()
                .find(|provider| provider.id == provider_id)
                .ok_or_else(|| CommandError::new("ai.no_agent", "no such provider"))?;
            let mut provider = provider.clone();
            let requested = model.as_deref().or_else(|| {
                earlier
                    .as_ref()
                    .and_then(|(_, info, _, _)| info.agent_version.as_deref())
            });
            if let Some(model) = requested {
                select_model(&mut provider, model)?;
            }
            let mut current_prefs = prefs.clone();
            if let Some(active) = current_prefs
                .ai
                .providers
                .iter_mut()
                .find(|p| p.id == provider.id)
            {
                *active = provider.clone();
            }
            session.set_reviewer(reviewer_for(
                state.inner(),
                &current_prefs,
                &agent_ref(&agent_text)?,
            ));
            let selection = Arc::new(std::sync::RwLock::new(provider.clone()));
            let key = state
                .vault
                .ai_key(&format!("builtin:{provider_id}"))?
                .unwrap_or_default();
            let idle = Duration::from_secs(u64::from(prefs.ai.request_timeout_secs));
            let client = Client::new(endpoint(&provider), &key, idle)
                .map_err(|error| CommandError::new("ai.model", error.to_string()))?
                .with_cache_key(&id);
            let assistant = if matches!(context, AiContext::Fleet) {
                let tools = Arc::new(crate::ops_tools::OpsTools::new(
                    app.clone(),
                    id.clone(),
                    selection.clone(),
                    session.clone(),
                    None,
                ));
                operations = Some(tools.clone());
                let audit = tools.audit_guard();
                Assistant::with_toolbox(client, session, crate::ops_tools::PROMPT.to_owned(), tools)
                    .with_checkpoint_sink(lead_checkpoint_sink(app.clone(), id.clone(), audit))
            } else {
                Assistant::new(client, session, &meta)
            }
            .with_models(&provider.models)
            .with_max_steps(prefs.ai.max_steps as usize)
            .with_contexts(provider.contexts.clone(), provider.context_window);
            let checkpoint = if matches!(context, AiContext::Fleet) && earlier.is_some() {
                latest_lead_checkpoint(state.inner(), &id)?
            } else {
                None
            };
            if let Some(checkpoint) = checkpoint {
                let tasks = if checkpoint.pending_tool.is_some() {
                    serde_json::to_value(
                        state
                            .ops
                            .list()
                            .map_err(crate::ops_runtime::error)?
                            .into_iter()
                            .filter(|task| task.conversation == id)
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|_| CommandError::new("ops.store", "invalid task state"))?
                } else {
                    serde_json::Value::Null
                };
                let checkpoint =
                    reviewed_lead_checkpoint(checkpoint, resume_confirmed == Some(true), &tasks)?;
                assistant
                    .restore_checkpoint(checkpoint)
                    .await
                    .map_err(|error| CommandError::new("ops.checkpoint", error.to_string()))?;
            } else if let Some((_, _, records, _)) = &earlier {
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
            let active = Arc::new(InteractiveAssistant {
                session: assistant.session().clone(),
                ops: operations,
                assistant: tokio::sync::RwLock::new(assistant),
                provider: selection,
                info: std::sync::Mutex::new(SessionInfo {
                    agent: started.agent.clone(),
                    agent_version: started.agent_version.clone(),
                    mode: serde_json::to_value(prefs.ai.mode)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    agent_ref: Some(agent_text.clone()),
                    context: serde_json::to_value(&context).ok(),
                    agent_session: None,
                    resumed: earlier.is_some(),
                    parent: None,
                }),
            });
            builtin = Some(active.clone());
            (active as Arc<dyn AgentBackend>, None, started)
        }
        AgentRef::Acp(_) if matches!(context, AiContext::Fleet) => {
            return Err(CommandError::new(
                "input.invalid",
                "the fleet lead is RoveTerm's own assistant",
            ));
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
        parent: None,
    });
    state.ai.entries.lock().await.insert(
        id,
        Arc::new(Entry {
            builtin,
            backend,
            auditor,
            mcp,
            parent: None,
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
    let (host, _meta, _cwd) = host_of(app, state, &context)?;
    let id = format!("ai-{}", uuid::Uuid::now_v7());
    let auditor = Arc::new(Auditor::new(
        audit_sink(state.vault.clone(), state.journal.clone()),
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
    attach_notes(state, &session, &context);
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
        parent: None,
    });
    state.ai.entries.lock().await.insert(
        id.clone(),
        Arc::new(Entry {
            builtin: None,
            backend: Arc::new(ExternalBackend(session.clone())),
            auditor,
            mcp: None,
            parent: None,
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

/// A sub-agent of the fleet lead started on a host (`{parent, id, host, task}`) or ended (`{id,
/// ended: true}`).
pub const AI_SUBTASK_EVENT: &str = "ai-subtask";

/// Starts a sub-agent for fleet lead `parent` on saved host `host_id`: RoveTerm's assistant with
/// `provider`, the host's tools and notes, the user's approval rules and judge, recorded as its
/// own conversation (with `parent`) and announced to the UI. Returns its id and assistant.
pub(crate) async fn start_ops_worker(
    app: &AppHandle,
    parent: &str,
    host_id: rove_vault::HostId,
    provider: &ProviderPrefs,
    lead_mode: Mode,
    unattended: Option<bool>,
    spec: crate::ops_runtime::WorkerSpec,
) -> Result<crate::ops_runtime::OpsWorker, CommandError> {
    let state = app.state::<AppState>();
    let state = state.inner();
    let prefs = state.vault.preferences()?;
    let requested_model = provider.model.clone();
    let mut provider = prefs
        .ai
        .providers
        .iter()
        .find(|current| current.id == provider.id)
        .cloned()
        .ok_or_else(|| CommandError::new("ops.failed", "the model provider was removed"))?;
    select_model(&mut provider, &requested_model)?;
    let context = AiContext::Host {
        host_id: host_id.to_string(),
    };
    let (host, meta, _cwd) = host_of(app, state, &context)?;
    let id = spec.session;
    let auditor = Arc::new(Auditor::new(
        audit_sink(state.vault.clone(), state.journal.clone()),
        id.clone(),
        host.label(),
    ));
    auditor.protect_text();
    let observer = spec.on_event;
    let sink = {
        let app = app.clone();
        let auditor = auditor.clone();
        let id = id.clone();
        Arc::new(move |event: AiEvent| {
            observer(&event);
            auditor.event(&event);
            if let Err(error) = app.emit(AI_EVENT, serde_json::json!({ "id": id, "event": event }))
            {
                tracing::warn!(%error, "emitting an AI event failed");
            }
        })
    };
    // The lead's mode now; a scheduled run has nobody to ask.
    let settings = SessionSettings {
        mode: if unattended.is_some() {
            Mode::Unattended
        } else {
            lead_mode
        },
        rules: Rules {
            read_only: prefs.ai.read_only.clone(),
            dangerous: prefs.ai.dangerous.clone(),
        },
        ..SessionSettings::default()
    };
    let session = Arc::new(AiSession::new(host.clone(), settings, sink));
    if !spec.can_modify {
        session.restrict_read_only();
    }
    attach_notes(state, &session, &context);
    let agent_ref = AgentRef::Builtin(provider.id.clone());
    // A scheduled run that may not change anything has no judge: every change is refused.
    if unattended != Some(false) {
        let mut reviewer_prefs = prefs.clone();
        if let Some(p) = reviewer_prefs
            .ai
            .providers
            .iter_mut()
            .find(|p| p.id == provider.id)
        {
            p.model.clone_from(&provider.model);
        }
        session.set_reviewer(reviewer_for(state, &reviewer_prefs, &agent_ref));
    }
    let key = state
        .vault
        .ai_key(&format!("builtin:{}", provider.id))?
        .unwrap_or_default();
    let idle = Duration::from_secs(u64::from(prefs.ai.request_timeout_secs));
    let client = Client::new(endpoint(&provider), &key, idle)
        .map_err(|error| CommandError::new("ai.model", error.to_string()))?
        .with_cache_key(&id);
    let verification = Arc::new(crate::ops_verifier::Verification::new(spec.acceptance));
    let toolbox = Arc::new(crate::ops_verifier::VerifiedTools::new(
        Arc::new(rove_ai::mcp::SessionTools(session.clone())),
        verification.clone(),
    ));
    let assistant = Assistant::with_host_toolbox(client, session, &meta, toolbox)
        .with_models(&provider.models)
        .with_max_steps(prefs.ai.max_steps as usize)
        .with_contexts(provider.contexts.clone(), provider.context_window)
        .with_checkpoint_sink(spec.persist);
    let resumed = spec.checkpoint.is_some();
    if let Some(checkpoint) = spec.checkpoint {
        assistant
            .restore_checkpoint(checkpoint)
            .await
            .map_err(|error| CommandError::new("ops.checkpoint", error.to_string()))?;
    }
    let assistant = Arc::new(assistant);
    let mode_value = serde_json::to_value(if unattended.is_some() {
        Mode::Unattended
    } else {
        lead_mode
    })
    .ok()
    .and_then(|mode| mode.as_str().map(str::to_owned))
    .unwrap_or_default();
    auditor.started(&SessionInfo {
        agent: Some(provider.name.clone()),
        agent_version: Some(provider.model.clone()),
        mode: mode_value,
        agent_ref: Some(format!("builtin:{}", provider.id)),
        context: serde_json::to_value(&context).ok(),
        agent_session: None,
        resumed,
        parent: Some(parent.to_owned()),
    });
    state.ai.entries.lock().await.insert(
        id.clone(),
        Arc::new(Entry {
            builtin: None,
            backend: assistant.clone(),
            auditor,
            mcp: None,
            parent: Some(parent.to_owned()),
        }),
    );
    if let Err(error) = app.emit(
        AI_SUBTASK_EVENT,
        serde_json::json!({ "parent": parent, "id": id, "host": host.label(), "task": spec.task }),
    ) {
        tracing::warn!(%error, "announcing a sub-agent failed");
    }
    Ok(crate::ops_runtime::OpsWorker {
        id,
        assistant,
        verification,
    })
}

/// The current conversation policy, for resumed operations workers.
pub(crate) async fn ops_live_mode(app: &AppHandle, parent: &str) -> Option<Mode> {
    let state = app.state::<AppState>();
    let entry = state.ai.entries.lock().await.get(parent).cloned()?;
    Some(entry.backend.session().mode().await)
}

/// The provider the fleet lead uses: the default agent when it is a provider, else the first.
pub(crate) fn fleet_provider(prefs: &rove_vault::Preferences) -> Option<ProviderPrefs> {
    let wanted = prefs
        .ai
        .default_agent
        .as_deref()
        .and_then(|agent| agent.strip_prefix("builtin:"));
    prefs
        .ai
        .providers
        .iter()
        .find(|provider| Some(provider.id.as_str()) == wanted)
        .or_else(|| prefs.ai.providers.first())
        .cloned()
}

/// Runs scheduled task `schedule` once: a fleet lead conversation of its own, titled with the
/// task's name, whose sub-agents run unattended (making changes only when the task allows and
/// the judge is sure). Returns the conversation's id, and its report (the lead's last reply) or
/// why it failed.
pub(crate) async fn run_scheduled(
    app: &AppHandle,
    schedule: &rove_vault::Schedule,
    when: &str,
    due: i64,
) -> (Option<String>, Result<String, String>) {
    let state = app.state::<AppState>();
    let Ok(prefs) = state.vault.preferences() else {
        return (None, Err("the preferences could not be read".into()));
    };
    let Some(provider) = fleet_provider(&prefs) else {
        return (None, Err("no model provider is set up".into()));
    };
    let id = format!("ai-{}", uuid::Uuid::now_v7());
    let context = AiContext::Fleet;
    let auditor = Arc::new(Auditor::new(
        audit_sink(state.vault.clone(), state.journal.clone()),
        id.clone(),
        "fleet",
    ));
    auditor.set_titled();
    auditor.protect_text();
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
    let session = Arc::new(AiSession::new(
        Arc::new(LocalHost),
        SessionSettings {
            mode: Mode::Unattended,
            ..SessionSettings::default()
        },
        sink,
    ));
    let key = state
        .vault
        .ai_key(&format!("builtin:{}", provider.id))
        .ok()
        .flatten()
        .unwrap_or_default();
    let idle = Duration::from_secs(u64::from(prefs.ai.request_timeout_secs));
    let client = match Client::new(endpoint(&provider), &key, idle) {
        Ok(client) => client.with_cache_key(&id),
        Err(error) => return (None, Err(error.to_string())),
    };
    let tools = Arc::new(crate::ops_tools::OpsTools::new(
        app.clone(),
        id.clone(),
        Arc::new(std::sync::RwLock::new(provider.clone())),
        session.clone(),
        Some(schedule.allow_changes),
    ));
    let request_id = match rove_ops::OpsRequestId::new(format!("schedule:{}:{due}", schedule.id)) {
        Ok(id) => id,
        Err(error) => return (None, Err(error.to_string())),
    };
    let _request = match tools.begin_request(
        request_id,
        rove_ops::TaskSource::Schedule {
            schedule: schedule.id.to_string(),
            due,
        },
        schedule.allow_changes,
    ) {
        Ok(guard) => guard,
        Err(error) => return (None, Err(error)),
    };
    let audit = tools.audit_guard();
    let assistant = Arc::new(
        Assistant::with_toolbox(client, session, crate::ops_tools::PROMPT.to_owned(), tools)
            .with_checkpoint_sink(lead_checkpoint_sink(app.clone(), id.clone(), audit))
            .with_models(&provider.models)
            .with_max_steps(prefs.ai.max_steps as usize)
            .with_contexts(provider.contexts.clone(), provider.context_window),
    );
    auditor.started(&SessionInfo {
        agent: Some(provider.name.clone()),
        agent_version: Some(provider.model.clone()),
        mode: "unattended".into(),
        agent_ref: Some(format!("builtin:{}", provider.id)),
        context: serde_json::to_value(&context).ok(),
        agent_session: None,
        resumed: false,
        parent: None,
    });
    state.ai.entries.lock().await.insert(
        id.clone(),
        Arc::new(Entry {
            builtin: None,
            backend: assistant.clone(),
            auditor: auditor.clone(),
            mcp: None,
            parent: None,
        }),
    );
    let changes = if schedule.allow_changes {
        "Changes the user's judge is sure of are made; other changes are refused and listed."
    } else {
        "This run only checks and reports: every change is refused and listed."
    };
    let prompt = format!(
        "Scheduled task \"{}\" ({when}). Nobody is watching this run. {changes}\n\n{}\n\nFinish with a report: a one-line summary, then per server what you found and what was changed, then what needs the user's decision.",
        schedule.name, schedule.instruction
    );
    auditor.prompt(&prompt);
    state.ai.recorder.record(audit::renamed(
        &id,
        &format!("{} · {when}", schedule.name),
        false,
    ));
    let result = assistant.prompt(&prompt).await;
    let mut operations_failure = None;
    let mut reports = Vec::new();
    if matches!(&result, Ok(reason) if reason == "end_turn") {
        let mut updates = state.ops.subscribe();
        loop {
            let tasks = match state.ops.list() {
                Ok(tasks) => tasks
                    .into_iter()
                    .filter(|task| task.conversation == id)
                    .collect::<Vec<_>>(),
                Err(error) => {
                    operations_failure = Some(error.to_string());
                    break;
                }
            };
            if tasks.iter().any(|task| {
                matches!(
                    task.state,
                    rove_ops::OpsTaskState::Queued
                        | rove_ops::OpsTaskState::Running
                        | rove_ops::OpsTaskState::WaitingApproval
                )
            }) {
                if state.ops.is_closed() || updates.changed().await.is_err() {
                    operations_failure = Some("operations stopped before completion".into());
                    break;
                }
                continue;
            }
            for task in tasks {
                if task.state != rove_ops::OpsTaskState::Completed {
                    operations_failure =
                        Some(format!("operations task {} is {:?}", task.id, task.state));
                }
                let evidence = task.report.into_iter().chain(
                    task.subtasks
                        .iter()
                        .filter_map(|sub| sub.evidence.last().cloned()),
                );
                for reference in evidence {
                    match state
                        .journal
                        .private_body(&reference.session, &reference.reference.to_string())
                        .map_err(|error| error.to_string())
                        .and_then(|sealed| {
                            state
                                .vault
                                .open_audit(&reference.session, reference.reference, &sealed)
                                .map_err(|error| error.to_string())
                        }) {
                        Ok(report) => reports.push(report.to_string()),
                        Err(error) => operations_failure = Some(error),
                    }
                }
            }
            break;
        }
    }
    let ended = match &result {
        Ok(reason) => reason.clone(),
        Err(error) => format!("error: {error}"),
    };
    auditor.turn_ended(&ended);
    let mut report = assistant.last_reply().await;
    if !reports.is_empty() {
        report = reports.join("\n\n");
        assistant.session().emit(AiEvent::Update {
            update: acp::SessionUpdate::Message {
                text: report.clone(),
                message_id: Some("ops-final-report".into()),
            },
        });
    }
    let entry = state.ai.entries.lock().await.remove(&id);
    if let Some(entry) = entry {
        stop(&entry).await;
    }
    (
        Some(id),
        match result {
            Ok(reason) if reason == "end_turn" => match operations_failure {
                Some(error) => Err(error),
                None => Ok(report),
            },
            Ok(reason) => Err(reason),
            Err(error) => Err(error.to_string()),
        },
    )
}

/// Ends sub-agent `id` (its task is done, failed or was cancelled).
pub(crate) async fn end_subagent(app: &AppHandle, state: &AppState, id: &str) {
    let entry = state.ai.entries.lock().await.remove(id);
    if let Some(entry) = entry {
        stop(&entry).await;
    }
    if let Err(error) = app.emit(
        AI_SUBTASK_EVENT,
        serde_json::json!({ "id": id, "ended": true }),
    ) {
        tracing::warn!(%error, "announcing the end of a sub-agent failed");
    }
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
    request_id: Option<String>,
    allow_changes: Option<bool>,
) -> Result<String, CommandError> {
    if text.trim().is_empty() || text.len() > MAX_PROMPT {
        return Err(CommandError::new(
            "input.invalid",
            "the message is empty or too long",
        ));
    }
    let entry = state.ai.get(&id).await?;
    let _request = if let Some(tools) = entry
        .builtin
        .as_ref()
        .and_then(|active| active.ops.as_ref())
    {
        let request = rove_ops::OpsRequestId::new(
            request_id.unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
        )
        .map_err(|e| CommandError::new("input.invalid", e.to_string()))?;
        Some(
            tools
                .begin_request(
                    request,
                    rove_ops::TaskSource::User,
                    allow_changes.unwrap_or(false),
                )
                .map_err(|e| CommandError::new("ai.busy", e))?,
        )
    } else {
        None
    };
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
    let entry = state.ai.get(&id).await?;
    entry.backend.cancel().await;
    if let Some(active) = &entry.builtin {
        let _idle = active.assistant.write().await;
        state
            .ops
            .cancel_conversation(&id)
            .map_err(|e| CommandError::new("ops.store", e.to_string()))?;
    } else {
        state
            .ops
            .cancel_conversation(&id)
            .map_err(|e| CommandError::new("ops.store", e.to_string()))?;
    }
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

/// Changes the approval mode of session `id`, and of the sub-agents it is running.
#[tauri::command]
pub async fn ai_set_mode(
    state: State<'_, AppState>,
    id: String,
    mode: AiMode,
) -> Result<(), CommandError> {
    let mode = self::mode(mode);
    let affected: Vec<Arc<Entry>> = {
        let entries = state.ai.entries.lock().await;
        if !entries.contains_key(&id) {
            drop(entries);
            // The same error as an unknown session.
            state.ai.get(&id).await?;
            return Ok(());
        }
        entries
            .iter()
            .filter(|(key, entry)| **key == id || entry.parent.as_deref() == Some(id.as_str()))
            .map(|(_, entry)| entry.clone())
            .collect()
    };
    for entry in affected {
        entry.backend.session().set_mode(mode).await;
    }
    Ok(())
}

/// Ends session `id`: its running commands and the agent.
#[tauri::command]
pub async fn ai_stop(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let entry = state.ai.entries.lock().await.remove(&id);
    if let Some(entry) = entry {
        stop(&entry).await;
    }
    state
        .ops
        .cancel_conversation(&id)
        .map_err(|e| CommandError::new("ops.store", e.to_string()))?;
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
    state
        .ops
        .cancel_conversation(&id)
        .map_err(|e| CommandError::new("ops.store", e.to_string()))?;
    state
        .ops
        .wait_conversation_idle(&id)
        .await
        .map_err(|e| CommandError::new("ops.store", e.to_string()))?;
    let tasks = state
        .ops
        .list()
        .map_err(|e| CommandError::new("ops.store", e.to_string()))?;
    state
        .ops
        .forget_conversation(&id)
        .map_err(|e| CommandError::new("ops.store", e.to_string()))?;
    for task in tasks.into_iter().filter(|task| task.conversation == id) {
        for session in task.subtasks.into_iter().filter_map(|sub| sub.session) {
            state
                .journal
                .delete_session(&session, "ops/")
                .map_err(|e| CommandError::new("journal.io", e.to_string()))?;
        }
    }
    state
        .journal
        .delete_session(&id, "ops/")
        .map_err(|e| CommandError::new("journal.io", e.to_string()))?;
    let recorder = state.ai.recorder.clone();
    let journal = state.journal.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Records still queued for it are written first, then removed with the rest.
        recorder.flush();
        journal.transaction(&mut || delete_conversation(journal.as_ref(), &id))
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
    pub role: &'static str,
    pub children: Vec<AiSessionSummary>,
    pub model: Option<String>,
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
const MAX_RECORDS: u32 = u32::MAX;

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
    for record in &mut records {
        if matches!(
            record.kind.as_str(),
            audit::KIND_MESSAGE | audit::KIND_PROMPT | audit::KIND_THOUGHT | audit::KIND_COMPACTED
        ) && let Some(reference) = record.payload["body_ref"].as_str()
            && let Ok(reference) = reference.parse::<uuid::Uuid>()
            && let Some(session) = &record.session_id
        {
            let sealed = state
                .journal
                .private_body(session, &reference.to_string())
                .map_err(|e| CommandError::new("journal.read", e.to_string()))?;
            record.payload = state.vault.open_audit(session, reference, &sealed)?;
        }
    }
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
    role: Option<String>,
) -> Result<Vec<AiSessionSummary>, CommandError> {
    let starts = newest(state.inner(), audit::KIND_STARTED, u32::MAX).await?;
    let names = newest(state.inner(), audit::KIND_RENAMED, u32::MAX).await?;
    Ok(history_summaries(starts, names, role.as_deref()))
}

fn history_summaries(
    mut starts: Vec<rove_journal::JournalRecord>,
    mut names: Vec<rove_journal::JournalRecord>,
    role: Option<&str>,
) -> Vec<AiSessionSummary> {
    starts.sort_by_key(|row| std::cmp::Reverse((row.ts, row.seq)));
    names.sort_by_key(|row| std::cmp::Reverse((row.ts, row.seq)));
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
    let mut children: HashMap<String, Vec<AiSessionSummary>> = HashMap::new();
    let mut sessions: Vec<_> = starts
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
            let kind = match record
                .payload
                .pointer("/context/kind")
                .and_then(|v| v.as_str())
            {
                Some("fleet") => "fleet",
                Some("host" | "local" | "quick") => "assistant",
                _ => "unknown",
            };
            let summary = AiSessionSummary {
                children: Vec::new(),
                role: kind,
                model: text("agent_version"),
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
            };
            if let Some(parent) = record.payload["parent"].as_str() {
                children.entry(parent.to_owned()).or_default().push(summary);
                return None;
            }
            role.is_none_or(|r| r == kind).then_some(summary)
        })
        .collect();
    sessions.truncate(MAX_SESSIONS as usize);
    for session in &mut sessions {
        session.children = children.remove(&session.id).unwrap_or_default();
    }
    sessions
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

fn select_model(provider: &mut ProviderPrefs, model: &str) -> Result<(), CommandError> {
    if provider.model != model {
        let index = provider
            .models
            .iter()
            .position(|entry| entry == model)
            .ok_or_else(|| {
                CommandError::new("ai.model", "model not configured for this provider")
            })?;
        std::mem::swap(&mut provider.model, &mut provider.models[index]);
    }
    Ok(())
}

/// Changes the built-in model and the clients used for subsequent delegated work.
#[tauri::command]
pub async fn ai_select_model(
    state: State<'_, AppState>,
    id: String,
    provider: String,
    model: String,
) -> Result<Vec<acp::ConfigOption>, CommandError> {
    let entry = state.ai.get(&id).await?;
    let active = entry
        .builtin
        .as_ref()
        .ok_or_else(|| CommandError::new("ai.model", "not an interactive assistant"))?;
    let mut prefs = state.vault.preferences()?;
    let target = prefs
        .ai
        .providers
        .iter_mut()
        .find(|p| p.id == provider)
        .ok_or_else(|| CommandError::new("ai.model", "unknown provider"))?;
    select_model(target, &model)?;
    let target = target.clone();
    let key = state
        .vault
        .ai_key(&format!("builtin:{provider}"))?
        .unwrap_or_default();
    let client = Client::new(
        endpoint(&target),
        &key,
        Duration::from_secs(prefs.ai.request_timeout_secs.into()),
    )
    .map_err(|e| CommandError::new("ai.model", e.to_string()))?
    .with_cache_key(&id);
    let reviewer = reviewer_for(&state, &prefs, &AgentRef::Builtin(provider.clone()));
    let mut assistant = active
        .assistant
        .try_write()
        .map_err(|_| backend_error(BackendError::Busy))?;
    let mut selected = active.provider.write().unwrap_or_else(|e| e.into_inner());
    let models = std::iter::once(model.clone())
        .chain(target.models.iter().cloned())
        .collect();
    assistant.retarget(
        client,
        models,
        target.contexts.clone(),
        target.context_window,
        selected.id != provider,
    );
    active.session.set_reviewer(reviewer);
    let mut info = active.info.lock().unwrap_or_else(|e| e.into_inner());
    info.agent_ref = Some(format!("builtin:{provider}"));
    info.agent = Some(target.name.clone());
    info.agent_version = Some(model);
    entry.auditor.model_changed(&info);
    *selected = target;
    Ok(assistant.config_options())
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
    let entry = state.ai.get(&id).await?;
    if option == "model"
        && let Some(active) = &entry.builtin
    {
        let provider = active
            .provider
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .id
            .clone();
        return ai_select_model(state, id, provider, value).await;
    }
    let options = entry
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

#[cfg(test)]
mod history_tests {
    use super::*;
    use rove_journal::{JournalQuery, JournalRecord, JournalStore, NewRecord, SqliteJournal};
    use serde_json::json;

    fn interrupted_lead() -> rove_ai::assistant::AssistantCheckpoint {
        serde_json::from_value(json!({
            "version":1,"format":"chat","model":"model","turns":1,"last_used":0,
            "pending_tool":"pending-start",
            "messages":[{"role":"assistant","text":"queued a task","tool_calls":[{
                "id":"pending-start","name":"start_tasks","arguments":"{}"
            }],"opaque":null}]
        }))
        .unwrap()
    }

    #[test]
    fn interrupted_lead_requires_review_before_restoring() {
        let checkpoint = interrupted_lead();
        let error = reviewed_lead_checkpoint(checkpoint.clone(), false, &json!([])).unwrap_err();
        assert_eq!(error.code, "ops.recovery_required");
        let mut clean = checkpoint;
        clean.resolve_interrupted();
        assert_eq!(
            reviewed_lead_checkpoint(clean.clone(), false, &json!([])).unwrap(),
            clean,
        );
    }

    #[test]
    fn reviewed_lead_preserves_history_and_adds_durable_task_state() {
        let checkpoint = reviewed_lead_checkpoint(
            interrupted_lead(),
            true,
            &json!([{"id":"existing-task","state":"interrupted"}]),
        )
        .unwrap();
        assert!(checkpoint.pending_tool.is_none());
        assert!(
            matches!(checkpoint.messages.first(),Some(llm::Message::Assistant{text,..}) if text=="queued a task")
        );
        assert!(checkpoint.messages.iter().any(|message| matches!(message,llm::Message::Tool{call_id,output,..} if call_id=="pending-start" && output.contains("do not replay"))));
        assert!(
            matches!(checkpoint.messages.last(),Some(llm::Message::User{text}) if text.contains("existing-task") && text.contains("operations-recovery-state"))
        );
    }

    #[test]
    fn roles_are_filtered_before_limit_and_children_follow_parent() {
        let start = |id: &str, ts: i64, kind: Option<&str>, parent: Option<&str>, model: &str| {
            JournalRecord {
                seq: ts,
                ts,
                kind: audit::KIND_STARTED.into(),
                session_id: Some(id.into()),
                payload: json!({"context":kind.map(|kind|json!({"kind":kind})), "parent":parent, "agent_version":model,"agent_ref":"builtin:b"}),
            }
        };
        let mut rows: Vec<_> = (0..120)
            .map(|i| start(&format!("ai-{i}"), 10 + i, Some("local"), None, "a"))
            .collect();
        rows.extend([
            start("ai-old", 0, None, None, ""),
            start("ai-fleet", 1, Some("fleet"), None, "before"),
            start("ai-fleet", 2, Some("fleet"), None, "after"),
            start("ai-child", 3, Some("host"), Some("ai-fleet"), "after"),
        ]);
        let fleet = history_summaries(rows.clone(), vec![], Some("fleet"));
        assert_eq!(fleet.len(), 1);
        assert_eq!(fleet[0].model.as_deref(), Some("after"));
        assert_eq!(fleet[0].children[0].id, "ai-child");
        assert_eq!(
            history_summaries(rows.clone(), vec![], Some("assistant")).len(),
            100
        );
        assert_eq!(
            history_summaries(rows, vec![], Some("unknown"))[0].id,
            "ai-old"
        );
    }

    #[test]
    fn full_tool_content_is_encrypted_local_and_deleted_with_the_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(SqliteJournal::open(&dir.path().join("journal.db")).unwrap());
        let vault = Arc::new(
            rove_vault::Vault::open(
                &dir.path().join("vault.db"),
                Arc::new(rove_vault::MemoryKeyCache::default()),
            )
            .unwrap(),
        );
        vault.try_unlock();
        let sink = audit_sink(vault.clone(), journal.clone());
        let session = "ai-privacy";
        let body = "unique-local-tool-body".repeat(2000);
        sink(NewRecord {
            kind: audit::KIND_STARTED.into(),
            session_id: Some(session.into()),
            payload: json!({}),
        });
        for (kind, field) in [
            (audit::KIND_FILE_READ, "content"),
            (audit::KIND_FILE_WRITE, "content"),
            (audit::KIND_TERMINAL_READ, "content"),
            (audit::KIND_COMMAND, "stdout"),
        ] {
            sink(NewRecord {
                kind: kind.into(),
                session_id: Some(session.into()),
                payload: json!({"outcome":{field:&body}}),
            });
        }
        let rows = journal.read(&JournalQuery::default()).unwrap();
        assert_eq!(rows.len(), 5);
        let mut refs = Vec::new();
        for row in &rows[1..] {
            let reference: uuid::Uuid =
                serde_json::from_value(row.payload["body_ref"].clone()).unwrap();
            let sealed = journal
                .private_body(session, &reference.to_string())
                .unwrap();
            let opened = vault.open_audit(session, reference, &sealed).unwrap();
            assert_eq!(
                opened["outcome"]
                    .as_object()
                    .unwrap()
                    .values()
                    .next()
                    .unwrap()
                    .as_str(),
                Some(body.as_str())
            );
            refs.push(reference);
        }
        let projection = audit::conversation_body(
            &rows
                .into_iter()
                .map(|r| (r.ts, r.kind, r.payload))
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .to_string();
        assert!(!projection.contains("unique-local-tool-body") && !projection.contains("body_ref"));
        for file in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let bytes = std::fs::read(file.path()).unwrap();
            assert!(!bytes.windows(22).any(|w| w == b"unique-local-tool-body"));
        }
        delete_conversation(journal.as_ref(), session).unwrap();
        for reference in refs {
            assert!(
                journal
                    .private_body(session, &reference.to_string())
                    .is_err()
            );
        }
    }
}
