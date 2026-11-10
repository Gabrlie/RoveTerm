//! AI sessions: start an agent on a host context, send prompts, answer approvals, stop. Events
//! stream to the WebView as `ai-update` and are recorded in the journal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use rove_ai::acp::{self, McpServer};
use rove_ai::audit::Auditor;
use rove_ai::host::{ConnectionSource, Host, LocalHost, RemoteHost};
use rove_ai::mcp::McpListener;
use rove_ai::runner::{self, AgentSpec, Running};
use rove_ai::session::{AiEvent, SessionSettings, TerminalSource};
use rove_ai::{Answer, Mode, Rules};
use rove_session::{SessionId, SessionManager};
use rove_ssh::{SshConnection, SshConnector, SshTarget};
use rove_vault::{AgentAuth, AiMode};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::connect::{host_target, quick_target};
use crate::error::CommandError;
use crate::state::AppState;

/// Event carrying an AI session's [`AiEvent`]s.
pub const AI_EVENT: &str = "ai-update";
/// Longest prompt accepted.
const MAX_PROMPT: usize = 100_000;
/// Codex's ACP adapter, run through npx when it is not installed.
const CODEX_PACKAGE: &str = "@agentclientprotocol/codex-acp";
/// Agent modes that make the agent ask before its own tools change anything.
const PREFERRED_MODES: &[&str] = &["read-only", "read_only", "readonly"];

/// One running AI session.
struct Entry {
    running: Running,
    auditor: Arc<Auditor>,
    mcp: tokio::task::JoinHandle<()>,
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
    entry.running.stop().await;
    entry.mcp.abort();
    entry.auditor.ended();
}

fn ai_error(error: acp::AcpError) -> CommandError {
    let code = match &error {
        acp::AcpError::Spawn(_) => "ai.spawn",
        acp::AcpError::Closed => "ai.closed",
        acp::AcpError::AuthRequired(_) => "ai.auth_required",
        acp::AcpError::Protocol(_) => "ai.protocol",
        acp::AcpError::Rpc { .. } => "ai.agent",
    };
    CommandError::new(code, error.to_string())
}

/// Where the AI session works.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiContext {
    Local,
    Host { host_id: String },
    Quick { address: String },
}

/// The agent program found for this device.
#[derive(Debug, Clone, Serialize)]
pub struct AgentFound {
    pub program: String,
    pub args: Vec<String>,
    /// `configured`, `path` or `npx`.
    pub source: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AiStatus {
    /// `None` when no agent can be found.
    pub agent: Option<AgentFound>,
    pub has_key: bool,
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

/// The PATH a login shell would have: GUI apps on macOS and Linux start with a minimal one, which
/// misses Homebrew, npm and version-manager directories where agents are installed.
fn search_path() -> String {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let current = std::env::var("PATH").unwrap_or_default();
        #[cfg(unix)]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            let output = std::process::Command::new(&shell)
                .args(["-ilc", "printf '%s' \"$PATH\""])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output();
            let login = output
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|text| text.lines().last().unwrap_or_default().trim().to_owned())
                .filter(|path| !path.is_empty());
            let home = std::env::home_dir().unwrap_or_default();
            let mut dirs: Vec<String> = Vec::new();
            for part in login
                .iter()
                .flat_map(|path| path.split(':'))
                .chain(current.split(':'))
            {
                if !part.is_empty() && !dirs.iter().any(|dir| dir == part) {
                    dirs.push(part.to_owned());
                }
            }
            for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
                if !dirs.iter().any(|dir| dir == extra) {
                    dirs.push(extra.to_owned());
                }
            }
            for extra in [".local/bin", ".cargo/bin", ".npm-global/bin", ".bun/bin"] {
                let dir = home.join(extra).to_string_lossy().into_owned();
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
            dirs.join(":")
        }
        #[cfg(not(unix))]
        {
            current
        }
    })
    .clone()
}

/// Finds `name` on `path`, as the shell would.
fn which(name: &str, path: &str) -> Option<PathBuf> {
    let candidates: Vec<String> = if cfg!(windows) {
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        extensions
            .split(';')
            .map(|extension| format!("{name}{}", extension.to_lowercase()))
            .collect()
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(path).find_map(|dir| {
        candidates
            .iter()
            .map(|candidate| dir.join(candidate))
            .find(|file| file.is_file())
    })
}

fn find_agent(state: &AppState) -> Result<Option<AgentFound>, CommandError> {
    let prefs = state.vault.device_preferences()?.agent;
    if !prefs.command.is_empty() {
        return Ok(Some(AgentFound {
            program: prefs.command,
            args: prefs.args,
            source: "configured",
        }));
    }
    let path = search_path();
    if let Some(program) = which("codex-acp", &path) {
        return Ok(Some(AgentFound {
            program: program.to_string_lossy().into_owned(),
            args: Vec::new(),
            source: "path",
        }));
    }
    Ok(which("npx", &path).map(|npx| AgentFound {
        program: npx.to_string_lossy().into_owned(),
        args: vec!["-y".into(), CODEX_PACKAGE.into()],
        source: "npx",
    }))
}

/// Reports the agent this device would start and whether an API key is stored.
#[tauri::command]
pub async fn ai_status(state: State<'_, AppState>) -> Result<AiStatus, CommandError> {
    let agent = {
        let state = state.inner();
        find_agent(state)?
    };
    Ok(AiStatus {
        agent,
        has_key: state.vault.has_ai_key()?,
    })
}

/// Stores the API key sealed on this device, or removes it with `None`.
#[tauri::command]
pub async fn ai_set_key(
    state: State<'_, AppState>,
    key: Option<String>,
) -> Result<(), CommandError> {
    state.ensure_unlocked().await?;
    let key = key.map(zeroize::Zeroizing::new);
    Ok(state.vault.set_ai_key(key.as_deref().map(String::as_str))?)
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

/// Starts an agent working on `context`; `terminal` is the user's terminal session there, which
/// the agent may read.
#[tauri::command]
pub async fn ai_start(
    app: AppHandle,
    state: State<'_, AppState>,
    context: AiContext,
    terminal: Option<String>,
) -> Result<AiStarted, CommandError> {
    state.ensure_unlocked().await?;
    let found = find_agent(state.inner())?
        .ok_or_else(|| CommandError::new("ai.no_agent", "no agent found"))?;
    let device = state.vault.device_preferences()?;
    let prefs = state.vault.preferences()?;

    let (host, cwd): (Arc<dyn Host>, PathBuf) = match &context {
        AiContext::Local => (
            Arc::new(LocalHost),
            std::env::home_dir().unwrap_or_else(|| PathBuf::from(".")),
        ),
        AiContext::Host { host_id } => {
            let host = state.vault.host(host_id.parse()?)?;
            let target = host_target(&state, &host)?;
            let label = format!("{} ({}@{})", host.name, target.username, target.hostname);
            let source = Arc::new(Pooled {
                ssh: state.ssh.clone(),
                target,
            });
            (
                Arc::new(RemoteHost::new(label, source)),
                state.ai.workspace.clone(),
            )
        }
        AiContext::Quick { address } => {
            let target = quick_target(address)?;
            let label = target.address();
            let source = Arc::new(Pooled {
                ssh: state.ssh.clone(),
                target,
            });
            (
                Arc::new(RemoteHost::new(label, source)),
                state.ai.workspace.clone(),
            )
        }
    };
    std::fs::create_dir_all(&cwd)
        .map_err(|error| CommandError::new("app.io", error.to_string()))?;

    // Credentials: the stored key, or the agent's own sign-in.
    let key = match device.agent.auth {
        AgentAuth::Account => None,
        AgentAuth::Auto => state.vault.ai_key()?,
        AgentAuth::ApiKey => Some(
            state
                .vault
                .ai_key()?
                .ok_or_else(|| CommandError::new("ai.no_key", "no API key is stored"))?,
        ),
    };
    let mut extra = vec![("PATH".to_owned(), search_path())];
    let auth_methods: Vec<String> = match &key {
        Some(key) => {
            extra.push(("CODEX_API_KEY".into(), key.to_string()));
            extra.push(("OPENAI_API_KEY".into(), key.to_string()));
            vec!["codex-api-key".into(), "openai-api-key".into()]
        }
        None => vec!["chatgpt".into()],
    };
    let env = acp::agent_env(acp::ENV_ALLOWED, &extra);

    let id = format!("ai-{}", uuid::Uuid::now_v7());
    let listener = McpListener::bind()
        .await
        .map_err(|error| CommandError::new("app.io", error.to_string()))?;
    let exe =
        std::env::current_exe().map_err(|error| CommandError::new("app.io", error.to_string()))?;
    let mcp = vec![McpServer {
        name: "roveterm".into(),
        command: exe.to_string_lossy().into_owned(),
        args: vec!["--mcp-bridge".into()],
        env: listener
            .bridge_env()
            .map_err(|error| CommandError::new("app.io", error.to_string()))?,
    }];

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
    let spec = AgentSpec {
        program: found.program,
        args: found.args,
        env,
        cwd,
        auth_methods,
        mcp,
        preferred_modes: PREFERRED_MODES
            .iter()
            .map(|mode| (*mode).to_owned())
            .collect(),
    };
    let running = runner::start(&spec, host.clone(), settings, sink)
        .await
        .map_err(ai_error)?;
    if let Some(terminal) = terminal {
        let id: SessionId = terminal.parse()?;
        running.session().set_terminal_source(Arc::new(Screen {
            sessions: state.sessions.clone(),
            id,
        }));
    }
    let mcp = listener.serve(running.session().clone());
    let started = running.started().clone();
    auditor.started(
        started.agent.as_deref(),
        started.agent_version.as_deref(),
        &format!("{:?}", prefs.ai.mode),
    );
    state.ai.entries.lock().await.insert(
        id.clone(),
        Arc::new(Entry {
            running,
            auditor,
            mcp,
        }),
    );
    Ok(AiStarted {
        id,
        host: host.label(),
        agent: started.agent,
        agent_version: started.agent_version,
        modes: started.modes,
        current_mode: started.current_mode,
    })
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
    let result = entry.running.prompt(&text).await;
    let stop = match &result {
        Ok(reason) => reason.clone(),
        Err(error) => format!("error: {error}"),
    };
    entry.auditor.turn_ended(&stop);
    result.map_err(ai_error)
}

/// Stops the current turn of session `id`.
#[tauri::command]
pub async fn ai_cancel(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    state
        .ai
        .get(&id)
        .await?
        .running
        .cancel()
        .await
        .map_err(ai_error)
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
        .running
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
        .running
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_programs_on_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "tool.exe" } else { "tool" };
        std::fs::write(dir.path().join(name), b"").unwrap();
        let path = std::env::join_paths([dir.path()])
            .unwrap()
            .into_string()
            .unwrap();
        assert_eq!(which("tool", &path), Some(dir.path().join(name)));
        assert_eq!(which("missing", &path), None);
    }
}
