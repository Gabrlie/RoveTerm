//! Host key verification: the vault's known-hosts table plus a user confirmation prompt.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rove_kernel::{Event, EventBus};
use rove_vault::{HostKey, KnownHostStatus, Vault};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Why a host key was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostKeyRejection {
    /// The recorded key differs: possible man-in-the-middle attack.
    Mismatch {
        /// Fingerprint of the recorded key.
        expected_fingerprint: String,
        /// Fingerprint of the presented key.
        presented_fingerprint: String,
    },
    /// The user declined an unknown key.
    Declined,
    /// Nobody answered the confirmation prompt in time.
    PromptTimeout,
    /// The known-hosts table could not be read or written.
    Storage(String),
}

/// Decides whether to trust a server's host key. Called once per new connection.
#[async_trait]
pub trait HostKeyVerifier: Send + Sync {
    /// Returns `Ok(())` to continue the handshake.
    async fn verify(
        &self,
        hostname: &str,
        port: u16,
        key: &HostKey,
    ) -> Result<(), HostKeyRejection>;
}

/// Live event: an unknown host key needs the user's decision. Answer with
/// [`HostKeyPrompts::respond`].
#[derive(Debug, Clone, Serialize)]
pub struct HostKeyPrompt {
    /// Identifier to pass back to [`HostKeyPrompts::respond`].
    pub request_id: String,
    /// Target host name.
    pub hostname: String,
    /// Target port.
    pub port: u16,
    /// The presented key.
    pub key: HostKey,
}
impl Event for HostKeyPrompt {}

/// The user's answer to a [`HostKeyPrompt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKeyAnswer {
    /// Trust the key and remember it.
    Trust,
    /// Abort the connection.
    Reject,
}

/// Pending host key confirmations.
pub struct HostKeyPrompts {
    events: EventBus,
    timeout: Duration,
    pending: Mutex<HashMap<String, oneshot::Sender<HostKeyAnswer>>>,
}

impl HostKeyPrompts {
    /// Creates the prompt registry. Unanswered prompts fail after `timeout`.
    pub fn new(events: EventBus, timeout: Duration) -> Self {
        Self {
            events,
            timeout,
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Emits a [`HostKeyPrompt`] and waits for the answer.
    pub async fn ask(
        &self,
        hostname: &str,
        port: u16,
        key: &HostKey,
    ) -> Result<HostKeyAnswer, HostKeyRejection> {
        let request_id = Uuid::now_v7().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(request_id.clone(), tx);
        self.events.emit(&HostKeyPrompt {
            request_id: request_id.clone(),
            hostname: hostname.to_owned(),
            port,
            key: key.clone(),
        });
        let answer = tokio::time::timeout(self.timeout, rx).await;
        self.pending.lock().remove(&request_id);
        match answer {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) | Err(_) => Err(HostKeyRejection::PromptTimeout),
        }
    }

    /// Delivers the user's answer. Returns `false` when the request is unknown or expired.
    pub fn respond(&self, request_id: &str, answer: HostKeyAnswer) -> bool {
        match self.pending.lock().remove(request_id) {
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    }
}

/// Verifier backed by the vault's known-hosts table: trusted keys pass, mismatching keys fail,
/// unknown keys are confirmed through [`HostKeyPrompts`] and remembered when trusted.
pub struct VaultVerifier {
    vault: Arc<Vault>,
    prompts: Arc<HostKeyPrompts>,
}

impl VaultVerifier {
    /// Creates the verifier.
    pub fn new(vault: Arc<Vault>, prompts: Arc<HostKeyPrompts>) -> Self {
        Self { vault, prompts }
    }
}

#[async_trait]
impl HostKeyVerifier for VaultVerifier {
    async fn verify(
        &self,
        hostname: &str,
        port: u16,
        key: &HostKey,
    ) -> Result<(), HostKeyRejection> {
        let storage = |error: rove_vault::VaultError| HostKeyRejection::Storage(error.to_string());
        match self
            .vault
            .check_host_key(hostname, port, key)
            .map_err(storage)?
        {
            KnownHostStatus::Trusted => Ok(()),
            KnownHostStatus::Mismatch {
                expected_fingerprint,
                ..
            } => Err(HostKeyRejection::Mismatch {
                expected_fingerprint,
                presented_fingerprint: key.fingerprint.clone(),
            }),
            KnownHostStatus::Unknown => match self.prompts.ask(hostname, port, key).await? {
                HostKeyAnswer::Trust => self
                    .vault
                    .trust_host_key(hostname, port, key)
                    .map_err(storage),
                HostKeyAnswer::Reject => Err(HostKeyRejection::Declined),
            },
        }
    }
}
