//! What the app drives in an AI session, whichever agent answers: the built-in assistant or an
//! ACP agent. A future agent (such as one already running on the server) is another
//! implementation, without changing the app or the UI.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::acp::ConfigOption;
use crate::assistant::{Assistant, AssistantError};
use crate::runner::Running;
use crate::session::AiSession;

/// Which agent answers in a session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum AgentRef {
    /// The built-in assistant with the provider of this id.
    Builtin(String),
    /// The ACP agent of this id (a known agent's id, or `custom-…`).
    Acp(String),
}

/// A failed turn.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum BackendError {
    #[error(transparent)]
    Acp(#[from] crate::acp::AcpError),
    #[error("{0}")]
    Model(String),
    #[error("a turn is already running")]
    Busy,
    /// A setting or value the agent does not offer.
    #[error("{0}")]
    Invalid(String),
}

impl From<AssistantError> for BackendError {
    fn from(error: AssistantError) -> Self {
        match error {
            AssistantError::Busy => Self::Busy,
            AssistantError::Llm(error) => Self::Model(error.to_string()),
            AssistantError::UnknownModel(_) => Self::Invalid(error.to_string()),
        }
    }
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Runs one turn; returns why it ended.
    async fn prompt(&self, text: &str) -> Result<String, BackendError>;
    /// Stops the running turn.
    async fn cancel(&self);
    /// Ends the session.
    async fn stop(&self);
    /// The session the agent works through.
    fn session(&self) -> &Arc<AiSession>;
    /// Sets one of the session's settings (such as `model`); returns them all as they now are.
    async fn set_config_option(
        &self,
        id: &str,
        value: &str,
    ) -> Result<Vec<ConfigOption>, BackendError>;
}

#[async_trait]
impl AgentBackend for Running {
    async fn prompt(&self, text: &str) -> Result<String, BackendError> {
        Ok(Running::prompt(self, text).await?)
    }

    async fn cancel(&self) {
        let _ = Running::cancel(self).await;
    }

    async fn stop(&self) {
        Running::stop(self).await;
    }

    fn session(&self) -> &Arc<AiSession> {
        Running::session(self)
    }

    async fn set_config_option(
        &self,
        id: &str,
        value: &str,
    ) -> Result<Vec<ConfigOption>, BackendError> {
        Ok(Running::set_config_option(self, id, value).await?)
    }
}

#[async_trait]
impl AgentBackend for Assistant {
    async fn prompt(&self, text: &str) -> Result<String, BackendError> {
        Ok(Assistant::prompt(self, text).await?)
    }

    async fn cancel(&self) {
        Assistant::cancel(self).await;
    }

    async fn stop(&self) {
        Assistant::cancel(self).await;
        self.session().stop_jobs().await;
    }

    fn session(&self) -> &Arc<AiSession> {
        Assistant::session(self)
    }

    async fn set_config_option(
        &self,
        id: &str,
        value: &str,
    ) -> Result<Vec<ConfigOption>, BackendError> {
        match id {
            "model" => self.set_model(value)?,
            "effort" => self.set_effort(value)?,
            _ => return Err(BackendError::Invalid(format!("no setting {id}"))),
        }
        Ok(self.config_options())
    }
}
