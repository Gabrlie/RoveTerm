//! Starting an agent for an AI session and driving its turns.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use crate::acp::{self, AcpError, Agent, McpServer, SessionMode};
use crate::host::Host;
use crate::session::{AiSession, EventSink, SessionSettings};

/// How to run an agent.
#[derive(Debug, Clone, Default)]
pub struct AgentSpec {
    pub program: String,
    pub args: Vec<String>,
    /// The agent's whole environment (see [`acp::agent_env`]).
    pub env: Vec<(String, String)>,
    /// The agent's working directory; for a remote host, an empty scratch directory.
    pub cwd: PathBuf,
    /// Sign-in method used when the agent asks for one.
    pub auth_method: Option<String>,
    pub mcp: Vec<McpServer>,
    /// Agent modes to switch to, most preferred first (such as `read-only`, which makes the
    /// agent ask before its own tools change anything).
    pub preferred_modes: Vec<String>,
}

/// What the UI needs to know about a started session.
#[derive(Debug, Clone, Serialize)]
pub struct Started {
    pub agent: Option<String>,
    pub agent_version: Option<String>,
    pub modes: Vec<SessionMode>,
    pub current_mode: Option<String>,
}

/// An agent working in one AI session.
pub struct Running {
    agent: Agent,
    session: Arc<AiSession>,
    acp_session: String,
    started: Started,
}

/// Starts the agent of `spec` for a session on `host`.
pub async fn start(
    spec: &AgentSpec,
    host: Arc<dyn Host>,
    settings: SessionSettings,
    sink: EventSink,
) -> Result<Running, AcpError> {
    let session = Arc::new(AiSession::new(host, settings, sink));
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
    let created = match agent.new_session(&cwd, &spec.mcp).await {
        Err(AcpError::AuthRequired(_)) if spec.auth_method.is_some() => {
            let method = spec.auth_method.as_deref().unwrap_or_default();
            agent.authenticate(method).await?;
            agent.new_session(&cwd, &spec.mcp).await?
        }
        other => other?,
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
    };
    Ok(Running {
        agent,
        session,
        acp_session: created.session_id,
        started,
    })
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
        self.agent.cancel(&self.acp_session)
    }

    /// Switches the agent to one of its [`Started::modes`].
    pub async fn set_agent_mode(&self, mode: &str) -> Result<(), AcpError> {
        self.agent.set_mode(&self.acp_session, mode).await
    }

    /// Ends the session: running commands and the agent process.
    pub async fn stop(self) {
        self.session.withdraw_approvals().await;
        self.session.stop_jobs().await;
        self.agent.shutdown().await;
    }
}
