//! Starting an agent for an AI session and driving its turns.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use crate::acp::{
    self, AcpError, Agent, AuthMethodKind, ConfigKind, ConfigOption, McpServer, NewSession,
    SessionMode,
};
use crate::session::AiSession;

/// How to run an agent.
#[derive(Debug, Clone, Default)]
pub struct AgentSpec {
    pub program: String,
    pub args: Vec<String>,
    /// The agent's whole environment (see [`acp::agent_env`]).
    pub env: Vec<(String, String)>,
    /// The agent's working directory; for a remote host, an empty scratch directory.
    pub cwd: PathBuf,
    /// Sign-in methods to use when the agent asks for one, most preferred first; the first the
    /// agent offers is used.
    pub auth_methods: Vec<String>,
    pub mcp: Vec<McpServer>,
    /// Agent modes to switch to, most preferred first (such as `read-only`, which makes the
    /// agent ask before its own tools change anything).
    pub preferred_modes: Vec<String>,
    /// The agent's own id of an earlier session to reopen, when the agent can.
    pub resume: Option<String>,
}

/// What the UI needs to know about a started session.
#[derive(Debug, Clone, Serialize)]
pub struct Started {
    pub agent: Option<String>,
    pub agent_version: Option<String>,
    pub modes: Vec<SessionMode>,
    pub current_mode: Option<String>,
    /// The session's settings the agent offers (model, reasoning effort, …).
    pub config_options: Vec<ConfigOption>,
    /// The agent's own id of the session, to reopen it later.
    pub agent_session: Option<String>,
    /// [`AgentSpec::resume`] reopened the earlier session.
    pub resumed: bool,
}

/// An agent working in one AI session.
pub struct Running {
    agent: Agent,
    session: Arc<AiSession>,
    acp_session: String,
    started: Started,
    /// The session's settings as they now are.
    options: std::sync::Mutex<Vec<ConfigOption>>,
}

/// Starts the agent of `spec` for `session`. Anything the agent connects to while starting (the
/// built-in MCP server) must already be serving `session`: agents start their MCP servers before
/// `session/new` returns.
pub async fn start(spec: &AgentSpec, session: Arc<AiSession>) -> Result<Running, AcpError> {
    let agent = acp::spawn(
        &spec.program,
        &spec.args,
        &spec.env,
        &spec.cwd,
        session.clone(),
    )
    .await?;
    open(agent, session, spec).await
}

/// Opens a session on an initialized `agent` whose client is `session`.
pub async fn open(
    agent: Agent,
    session: Arc<AiSession>,
    spec: &AgentSpec,
) -> Result<Running, AcpError> {
    let cwd = spec.cwd.to_string_lossy().into_owned();
    let resume = spec.resume.as_deref().filter(|_| agent.info().load_session);
    // The id is known before `session/load` answers, and the replay arrives before it does.
    if let Some(id) = resume {
        session.bind(id);
        session.set_replaying(true);
    }
    let opened = signed_in(&agent, spec, || async {
        match resume {
            Some(id) => agent.load_session(id, &cwd, &spec.mcp).await,
            None => agent.new_session(&cwd, &spec.mcp).await,
        }
    })
    .await;
    session.set_replaying(false);
    let (created, resumed) = match (opened, resume) {
        (Ok(created), resume) => (created, resume.is_some()),
        // The earlier session is gone: start a new one.
        (Err(AcpError::Rpc { .. }), Some(_)) => {
            session.unbind();
            (
                signed_in(&agent, spec, || agent.new_session(&cwd, &spec.mcp)).await?,
                false,
            )
        }
        (Err(error), _) => return Err(error),
    };
    session.bind(&created.session_id);
    let mut current_mode = created.current_mode.clone();
    let preferred = spec
        .preferred_modes
        .iter()
        .find(|wanted| created.modes.iter().any(|mode| &mode.id == *wanted));
    if let Some(mode) = preferred
        && current_mode.as_ref() != Some(mode)
    {
        agent.set_mode(&created.session_id, mode).await?;
        current_mode = Some(mode.clone());
    }
    let started = Started {
        agent: agent.info().name.clone(),
        agent_version: agent.info().version.clone(),
        modes: created.modes,
        current_mode,
        config_options: created.config_options,
        agent_session: Some(created.session_id.clone()),
        resumed,
    };
    tracing::debug!(agent = ?started.agent, options = ?started.config_options, "agent session settings");
    Ok(Running {
        agent,
        session,
        acp_session: created.session_id,
        options: std::sync::Mutex::new(started.config_options.clone()),
        started,
    })
}

/// Runs `open` (`session/new` or `session/load`), signing in first when the agent asks for it.
/// Only protocol-driven [`AuthMethodKind::Agent`] methods are attempted automatically; terminal
/// and env-var methods return [`AcpError::AuthRequired`] so the user can complete them explicitly.
async fn signed_in<F, Fut>(agent: &Agent, spec: &AgentSpec, open: F) -> Result<NewSession, AcpError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<NewSession, AcpError>>,
{
    match open().await {
        Err(AcpError::AuthRequired(message)) => {
            let offered = &agent.info().auth_methods;
            let method = choose_auto_auth(&spec.auth_methods, offered)
                .ok_or(AcpError::AuthRequired(message))?;
            agent.authenticate(method).await?;
            open().await
        }
        other => other,
    }
}

fn choose_auto_auth<'a>(
    wanted: &'a [String],
    offered: &[acp::AuthMethod],
) -> Option<&'a str> {
    if let Some(matched) = wanted.iter().find(|id| {
        offered
            .iter()
            .any(|method| &method.id == *id && matches!(method.kind, AuthMethodKind::Agent))
    }) {
        return Some(matched.as_str());
    }
    // Older agents that omit `authMethods` on `initialize` still accept a known method id.
    if offered.is_empty() {
        return wanted.first().map(String::as_str);
    }
    None
}

impl Running {
    pub fn started(&self) -> &Started {
        &self.started
    }

    pub fn session(&self) -> &Arc<AiSession> {
        &self.session
    }

    /// Sends `text` and waits for the turn to end; returns the stop reason.
    pub async fn prompt(&self, text: &str) -> Result<String, AcpError> {
        self.agent.prompt(&self.acp_session, text).await
    }

    /// Stops the current turn: withdraws waiting approvals and tells the agent.
    pub async fn cancel(&self) -> Result<(), AcpError> {
        self.session.withdraw_approvals().await;
        self.session.stop_jobs().await;
        self.agent.cancel(&self.acp_session)
    }

    /// Sets one of the agent's [`Started::config_options`]; returns them all as they now are.
    pub async fn set_config_option(
        &self,
        id: &str,
        value: &str,
    ) -> Result<Vec<ConfigOption>, AcpError> {
        let option = self
            .options
            .lock()
            .ok()
            .and_then(|options| options.iter().find(|option| option.id == id).cloned());
        let reported = match &option {
            Some(option) if option.legacy => {
                self.agent.set_model(&self.acp_session, value).await?;
                Vec::new()
            }
            Some(option) => {
                self.agent
                    .set_config_option(&self.acp_session, id, option.kind, value)
                    .await?
            }
            None => {
                self.agent
                    .set_config_option(&self.acp_session, id, ConfigKind::Select, value)
                    .await?
            }
        };
        let mut options = self.options.lock().map_err(|_| AcpError::Closed)?;
        if reported.is_empty() {
            // The agent did not report its settings: the change it accepted is the news.
            if let Some(changed) = options.iter_mut().find(|option| option.id == id) {
                value.clone_into(&mut changed.current);
            }
        } else {
            // Keep the older-API model choice, which the agent's own list does not include.
            let legacy: Vec<ConfigOption> = options
                .iter()
                .filter(|option| option.legacy && !reported.iter().any(|new| new.id == option.id))
                .cloned()
                .collect();
            *options = legacy.into_iter().chain(reported).collect();
        }
        Ok(options.clone())
    }

    /// Switches the agent to one of its [`Started::modes`].
    pub async fn set_agent_mode(&self, mode: &str) -> Result<(), AcpError> {
        self.agent.set_mode(&self.acp_session, mode).await
    }

    /// Ends the session: running commands and the agent process.
    pub async fn stop(&self) {
        self.session.withdraw_approvals().await;
        self.session.stop_jobs().await;
        self.agent.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::{AuthMethod, TerminalAuth};

    #[test]
    fn auto_auth_only_selects_agent_methods_and_never_terminal_methods() {
        let terminal = AuthMethod {
            id: "claude-login".into(),
            name: "Claude Login".into(),
            description: None,
            kind: AuthMethodKind::Terminal(TerminalAuth {
                command: Some("claude".into()),
                args: vec!["auth".into(), "login".into()],
                env: Vec::new(),
                label: None,
            }),
        };
        let agent = AuthMethod {
            id: "chatgpt".into(),
            name: "ChatGPT".into(),
            description: None,
            kind: AuthMethodKind::Agent,
        };
        let env_var = AuthMethod {
            id: "api-key".into(),
            name: "API Key".into(),
            description: None,
            kind: AuthMethodKind::EnvVar {
                vars: vec!["OPENAI_API_KEY".into()],
            },
        };
        assert_eq!(
            choose_auto_auth(&["claude-login".into()], std::slice::from_ref(&terminal)),
            None
        );
        assert_eq!(
            choose_auto_auth(&["api-key".into()], &[env_var]),
            None
        );
        assert_eq!(
            choose_auto_auth(&["chatgpt".into()], &[terminal, agent]),
            Some("chatgpt")
        );
        assert_eq!(
            choose_auto_auth(&["legacy-id".into()], &[]),
            Some("legacy-id")
        );
    }
}
