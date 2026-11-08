//! Terminal sessions for RoveTerm.
//!
//! Defines the [`TerminalSession`] seam implemented by terminal providers (local PTY, SSH) and the
//! [`SessionManager`] that owns live sessions, coalesces their output and keeps a replay buffer.
//! See `docs/subsystems/session.md`.

mod manager;
mod replay;
mod types;

use std::sync::Arc;

use async_trait::async_trait;
use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};

pub use manager::{
    OpenedTerminal, OutputChunk, OutputSink, SessionConfig, SessionExited, SessionManager,
    SessionOpened, SessionSummary,
};
pub use types::{ExitStatus, SessionId, SessionInfo, SessionKind, TermSize, TerminalOutput};

/// Control handle of one terminal, implemented by each provider.
///
/// Output does not flow through this trait; providers deliver it on the
/// [`OpenedTerminal::output`] channel.
#[async_trait]
pub trait TerminalSession: Send + Sync {
    /// Sends input bytes to the terminal.
    async fn write(&self, data: &[u8]) -> Result<(), SessionError>;

    /// Changes the terminal size.
    async fn resize(&self, size: TermSize) -> Result<(), SessionError>;

    /// Terminates the terminal and releases its resources. Idempotent: closing an already closed
    /// session succeeds.
    async fn close(&self) -> Result<(), SessionError>;
}

/// Errors returned by sessions and the session manager.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// No session with this id exists.
    #[error("session {0} not found")]
    NotFound(SessionId),
    /// A session id string could not be parsed.
    #[error("invalid session id `{0}`")]
    InvalidId(String),
    /// Terminal dimensions out of range.
    #[error("invalid terminal size {cols}x{rows}")]
    InvalidSize {
        /// Requested columns.
        cols: u16,
        /// Requested rows.
        rows: u16,
    },
    /// The session has ended and accepts no more input.
    #[error("session has ended")]
    Closed,
    /// A provider-specific failure.
    #[error("{0}")]
    Provider(String),
}

/// Registers [`SessionManager`] on the context. Configuration section: [`SessionConfig`].
pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn name(&self) -> &'static str {
        "session"
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: SessionConfig = config.parse()?;
        config
            .validate()
            .map_err(|message| KernelError::plugin("session", message))?;
        let manager = Arc::new(SessionManager::new(config, ctx.events().clone()));
        Ok(vec![ctx.provide(manager)?])
    }
}
