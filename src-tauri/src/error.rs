//! Structured errors returned to the WebView.

use rove_kernel::KernelError;
use rove_session::SessionError;
use rove_ssh::{HostKeyRejection, SshError};
use rove_vault::VaultError;
use serde::Serialize;

/// Error returned by every command: a stable machine-readable `code` for the UI to localize, plus
/// an English `message` for logs and fallback display. Messages never contain secrets.
#[derive(Debug, Serialize)]
pub struct CommandError {
    /// Stable identifier, `domain.reason`.
    pub code: &'static str,
    /// Human-readable detail.
    pub message: String,
    /// Structured detail for codes that need it (host key fingerprints).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl CommandError {
    /// Builds an error without structured detail.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }
}

impl From<SessionError> for CommandError {
    fn from(error: SessionError) -> Self {
        let code = match &error {
            SessionError::NotFound(_) => "session.not_found",
            SessionError::InvalidId(_) | SessionError::InvalidSize { .. } => "input.invalid",
            SessionError::Closed => "session.closed",
            SessionError::Provider(_) => "session.provider",
        };
        Self::new(code, error.to_string())
    }
}

impl From<VaultError> for CommandError {
    fn from(error: VaultError) -> Self {
        let code = match &error {
            VaultError::Locked(_) => "vault.locked",
            VaultError::HostNotFound(_) => "host.not_found",
            VaultError::Invalid(_) => "input.invalid",
            _ => "vault.error",
        };
        Self::new(code, error.to_string())
    }
}

impl From<SshError> for CommandError {
    fn from(error: SshError) -> Self {
        let code = match &error {
            SshError::Connect(_) => "ssh.connect",
            SshError::Timeout => "ssh.timeout",
            SshError::HostKey(HostKeyRejection::Mismatch { .. }) => "ssh.host_key_mismatch",
            SshError::HostKey(HostKeyRejection::Declined) => "ssh.host_key_declined",
            SshError::HostKey(_) => "ssh.host_key",
            SshError::AuthFailed => "ssh.auth_failed",
            SshError::CredentialsRequired => "ssh.credentials_required",
            SshError::Key(_) => "ssh.key",
            SshError::Agent(_) => "ssh.agent",
            SshError::Channel(_) | SshError::Protocol(_) => "ssh.protocol",
        };
        let detail = match &error {
            SshError::HostKey(rejection) => serde_json::to_value(rejection).ok(),
            _ => None,
        };
        Self {
            code,
            message: error.to_string(),
            detail,
        }
    }
}

impl From<KernelError> for CommandError {
    fn from(error: KernelError) -> Self {
        Self::new("app.service", error.to_string())
    }
}
