//! Steps of establishing a connection, reported while it happens so the UI can show the progress
//! and where it stopped.

use std::sync::Arc;

use serde::Serialize;

/// Receives connection steps. Called on the connecting task; must not block.
pub type Progress = Arc<dyn Fn(ConnectStep) + Send + Sync>;

/// One step of establishing a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum ConnectStep {
    /// Opening the transport to `address` (`user@host:port`), through `via` when not direct.
    Connecting {
        /// Destination.
        address: String,
        /// Proxy or jump host description.
        via: Option<String>,
    },
    /// The transport is open; the SSH handshake begins.
    Handshake,
    /// The server's host key was accepted.
    HostKeyAccepted {
        /// Key algorithm.
        algorithm: String,
        /// `SHA256:` fingerprint.
        fingerprint: String,
    },
    /// The SSH agent could not be used; default keys are tried next.
    AgentUnavailable {
        /// Why.
        reason: String,
    },
    /// Offering one credential.
    Trying {
        /// Kind of credential.
        method: AuthKind,
        /// Key file, key comment or fingerprint.
        label: Option<String>,
    },
    /// The server rejected that credential.
    Rejected {
        /// Kind of credential.
        method: AuthKind,
        /// As in [`ConnectStep::Trying`].
        label: Option<String>,
    },
    /// Signed in.
    Authenticated,
}

/// Kind of credential in a [`ConnectStep`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Password authentication.
    Password,
    /// Keyboard-interactive answered with the password.
    KeyboardInteractive,
    /// A private key held by RoveTerm.
    PublicKey,
    /// A key held by the SSH agent.
    Agent,
}

/// Optional progress sink.
#[derive(Clone, Default)]
pub(crate) struct Steps(pub(crate) Option<Progress>);

impl Steps {
    pub(crate) fn emit(&self, step: ConnectStep) {
        if let Some(progress) = &self.0 {
            progress(step);
        }
    }
}
