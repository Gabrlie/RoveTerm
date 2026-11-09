//! SSH terminal provider for RoveTerm.
//!
//! [`SshConnector`] authenticates to servers, verifies host keys against the vault, and opens PTY
//! shell channels as [`OpenedTerminal`](rove_session::OpenedTerminal)s. See
//! `docs/subsystems/ssh.md`.

mod connection;
mod hostkey;
mod pool;
mod sftp;
mod sshconfig;

use std::sync::Arc;
use std::time::Duration;

use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use rove_session::{OpenedTerminal, TermSize};
use rove_vault::{AuthMethod, Host, Secret, Vault};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub use connection::{ConnectOptions, SshConnection};
pub use hostkey::{
    HostKeyAnswer, HostKeyPrompt, HostKeyPrompts, HostKeyRejection, HostKeyVerifier, VaultVerifier,
};
pub use sftp::SftpFs;
pub use sshconfig::{
    ImportReport, QuickTarget, SshConfigHost, import_ssh_config, parse_quick_connect,
    parse_ssh_config,
};

/// Where to connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTarget {
    /// DNS name or IP address.
    pub hostname: String,
    /// TCP port.
    pub port: u16,
    /// Login user.
    pub username: String,
    /// Saved host this target comes from, for session metadata.
    pub host_id: Option<String>,
    /// Tab title; absent uses the address.
    pub title: Option<String>,
}

impl SshTarget {
    /// Returns `user@host:port`, bracketing IPv6 literals.
    pub fn address(&self) -> String {
        if self.hostname.contains(':') {
            format!("{}@[{}]:{}", self.username, self.hostname, self.port)
        } else {
            format!("{}@{}:{}", self.username, self.hostname, self.port)
        }
    }

    /// Builds the target of a saved host.
    pub fn from_host(host: &Host) -> Self {
        Self {
            hostname: host.hostname.clone(),
            port: host.port,
            username: host.username.clone(),
            host_id: Some(host.id.to_string()),
            title: Some(host.name.clone()),
        }
    }
}

/// Credentials for one connection attempt. Secret material is zeroized on drop.
pub enum AuthSpec {
    /// Password; keyboard-interactive is tried with the same password when plain password
    /// authentication fails.
    Password(Zeroizing<String>),
    /// Private key text in OpenSSH or PEM form.
    PrivateKey {
        /// Key text.
        key: Zeroizing<String>,
        /// Passphrase, if the key is encrypted.
        passphrase: Option<Zeroizing<String>>,
    },
    /// Private key file on this device.
    KeyFile {
        /// File path.
        path: String,
        /// Passphrase, if the key is encrypted.
        passphrase: Option<Zeroizing<String>>,
    },
    /// Keys from the running SSH agent.
    Agent,
}

impl AuthSpec {
    /// Resolves credentials for `host` from its stored `secret` and an optional password typed by
    /// the user. Returns [`SshError::CredentialsRequired`] when the user must be asked for a
    /// password first.
    pub fn resolve(
        host: &Host,
        secret: Option<Secret>,
        typed_password: Option<Zeroizing<String>>,
    ) -> Result<Self, SshError> {
        match (&host.auth, secret, typed_password) {
            (AuthMethod::Password | AuthMethod::Interactive, _, Some(password)) => {
                Ok(Self::Password(password))
            }
            (AuthMethod::Password, Some(Secret::Password { password }), None) => {
                Ok(Self::Password(password))
            }
            (AuthMethod::Password | AuthMethod::Interactive, _, None) => {
                Err(SshError::CredentialsRequired)
            }
            (AuthMethod::PrivateKey, Some(Secret::PrivateKey { key, passphrase }), _) => {
                Ok(Self::PrivateKey { key, passphrase })
            }
            (AuthMethod::PrivateKey, _, _) => Err(SshError::Key(
                "no private key is stored for this host".into(),
            )),
            (AuthMethod::KeyFile { path }, secret, typed) => Ok(Self::KeyFile {
                path: path.clone(),
                passphrase: match secret {
                    Some(Secret::Passphrase { passphrase }) => Some(passphrase),
                    _ => typed,
                },
            }),
            (AuthMethod::Agent, _, _) => Ok(Self::Agent),
        }
    }
}

/// Errors returned by SSH operations.
#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum SshError {
    /// TCP connection failed.
    #[error("connection failed: {0}")]
    Connect(String),
    /// TCP connection timed out.
    #[error("connection timed out")]
    Timeout,
    /// The host key was not accepted.
    #[error("host key rejected: {0:?}")]
    HostKey(HostKeyRejection),
    /// The server rejected every offered credential.
    #[error("authentication failed")]
    AuthFailed,
    /// A password must be obtained from the user before connecting.
    #[error("credentials required")]
    CredentialsRequired,
    /// A private key could not be read or decrypted.
    #[error("private key error: {0}")]
    Key(String),
    /// The SSH agent is unreachable or failed.
    #[error("ssh agent error: {0}")]
    Agent(String),
    /// Opening or configuring a channel failed.
    #[error("channel error: {0}")]
    Channel(String),
    /// The SFTP subsystem is unavailable or failed to start.
    #[error("sftp error: {0}")]
    Sftp(String),
    /// Any other protocol failure.
    #[error("ssh protocol error: {0}")]
    Protocol(String),
}

/// `[plugin.ssh]` configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SshConfig {
    /// TCP connect timeout in seconds (1–120).
    pub connect_timeout_secs: u64,
    /// Keepalive interval in seconds (5–600).
    pub keepalive_secs: u64,
    /// Unanswered keepalives before giving up (1–10).
    pub keepalive_max: usize,
    /// How long to wait for the user to confirm an unknown host key, in seconds (10–600).
    pub hostkey_prompt_secs: u64,
    /// Seconds a shared connection stays open after its last terminal closes (0–3600).
    pub idle_disconnect_secs: u64,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 15,
            keepalive_secs: 30,
            keepalive_max: 3,
            hostkey_prompt_secs: 120,
            idle_disconnect_secs: 60,
        }
    }
}

impl SshConfig {
    /// Checks every field against its documented range.
    pub fn validate(&self) -> Result<(), String> {
        let check = |name: &str, value: u64, range: std::ops::RangeInclusive<u64>| {
            if range.contains(&value) {
                Ok(())
            } else {
                Err(format!(
                    "{name} must be {}..={}, got {value}",
                    range.start(),
                    range.end()
                ))
            }
        };
        check("connect_timeout_secs", self.connect_timeout_secs, 1..=120)?;
        check("keepalive_secs", self.keepalive_secs, 5..=600)?;
        check("keepalive_max", self.keepalive_max as u64, 1..=10)?;
        check("hostkey_prompt_secs", self.hostkey_prompt_secs, 10..=600)?;
        check("idle_disconnect_secs", self.idle_disconnect_secs, 0..=3600)
    }

    fn options(&self) -> ConnectOptions {
        ConnectOptions {
            connect_timeout: Duration::from_secs(self.connect_timeout_secs),
            keepalive_interval: Duration::from_secs(self.keepalive_secs),
            keepalive_max: self.keepalive_max,
            idle_timeout: Some(Duration::from_secs(self.idle_disconnect_secs)),
        }
    }
}

/// Opens SSH terminals. Terminals to the same `user@host:port` share one connection.
pub struct SshConnector {
    options: ConnectOptions,
    verifier: Arc<dyn HostKeyVerifier>,
    pool: pool::Pool,
}

impl SshConnector {
    /// Creates a connector that verifies host keys with `verifier`.
    pub fn new(config: &SshConfig, verifier: Arc<dyn HostKeyVerifier>) -> Self {
        Self {
            options: config.options(),
            verifier,
            pool: pool::Pool::default(),
        }
    }

    /// Connects and authenticates to `target` on a new, unshared connection.
    pub async fn connect(
        &self,
        target: &SshTarget,
        auth: &AuthSpec,
    ) -> Result<Arc<SshConnection>, SshError> {
        SshConnection::connect(target, auth, self.verifier.clone(), &self.options)
            .await
            .map(Arc::new)
    }

    /// Returns the shared connection to `target`, connecting with `auth` when none is live.
    /// `auth` is ignored when a live connection is reused.
    pub async fn shared(
        &self,
        target: &SshTarget,
        auth: &AuthSpec,
    ) -> Result<Arc<SshConnection>, SshError> {
        let slot = self.pool.slot(pool::PoolKey::of(target));
        let mut guard = slot.connection.lock().await;
        if let Some(connection) = guard.as_ref().filter(|connection| !connection.is_closed()) {
            connection.touch();
            return Ok(connection.clone());
        }
        let connection = self.connect(target, auth).await?;
        *guard = Some(connection.clone());
        Ok(connection)
    }

    /// Returns the live shared connection to `target`, if any, without connecting.
    pub async fn live(&self, target: &SshTarget) -> Option<Arc<SshConnection>> {
        let slot = self.pool.slot(pool::PoolKey::of(target));
        let guard = slot.connection.lock().await;
        let connection = guard
            .as_ref()
            .filter(|connection| !connection.is_closed())?;
        connection.touch();
        Some(connection.clone())
    }

    /// Opens a PTY shell on the live shared connection to `target` without credentials. Returns
    /// `Ok(None)` when no live connection exists or it can no longer open channels; the caller then
    /// resolves credentials and uses [`SshConnector::open_terminal`].
    pub async fn open_live(
        &self,
        target: &SshTarget,
        size: TermSize,
    ) -> Result<Option<OpenedTerminal>, SshError> {
        let Some(connection) = self.live(target).await else {
            return Ok(None);
        };
        match connection.open_shell(size).await {
            Ok(opened) => Ok(Some(opened)),
            Err(SshError::Channel(reason)) => {
                tracing::debug!(%reason, "live ssh connection cannot open channels");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Opens a PTY shell of `size` on the shared connection to `target`.
    ///
    /// When opening the channel fails on a reused connection, the connection is replaced once.
    pub async fn open_terminal(
        &self,
        target: &SshTarget,
        auth: &AuthSpec,
        size: TermSize,
    ) -> Result<OpenedTerminal, SshError> {
        let connection = self.shared(target, auth).await?;
        match connection.open_shell(size).await {
            Ok(opened) => Ok(opened),
            Err(SshError::Channel(reason)) => {
                tracing::debug!(%reason, "reused ssh connection failed; reconnecting");
                self.evict(target, &connection).await;
                self.shared(target, auth).await?.open_shell(size).await
            }
            Err(error) => Err(error),
        }
    }

    /// Removes `connection` from the pool if it is still the shared connection to `target`, so the
    /// next request connects anew. Channels already open on it are unaffected.
    pub async fn evict(&self, target: &SshTarget, connection: &Arc<SshConnection>) {
        let slot = self.pool.slot(pool::PoolKey::of(target));
        let mut guard = slot.connection.lock().await;
        if guard
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, connection))
        {
            guard.take();
        }
    }

    /// Returns the number of live shared connections.
    pub fn live_connections(&self) -> usize {
        self.pool.live().len()
    }

    /// Disconnects every shared connection. Used on shutdown.
    pub async fn disconnect_all(&self) {
        for connection in self.pool.live() {
            connection.disconnect().await;
        }
    }
}

/// Registers [`HostKeyPrompts`] and [`SshConnector`]; host keys are verified against the vault.
pub struct SshPlugin;

impl Plugin for SshPlugin {
    fn name(&self) -> &'static str {
        "ssh"
    }

    fn requires(&self) -> &'static [&'static str] {
        &["vault"]
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: SshConfig = config.parse()?;
        config
            .validate()
            .map_err(|message| KernelError::plugin("ssh", message))?;
        let vault = ctx.require::<Vault>()?;
        let prompts = Arc::new(HostKeyPrompts::new(
            ctx.events().clone(),
            Duration::from_secs(config.hostkey_prompt_secs),
        ));
        let verifier = Arc::new(VaultVerifier::new(vault, prompts.clone()));
        Ok(vec![
            ctx.provide(prompts)?,
            ctx.provide(Arc::new(SshConnector::new(&config, verifier)))?,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_vault::HostId;

    fn host(auth: AuthMethod) -> Host {
        Host {
            id: HostId::new(),
            name: "n".into(),
            group: None,
            hostname: "::1".into(),
            port: 22,
            username: "u".into(),
            auth,
            tags: Vec::new(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn pw(text: &str) -> Zeroizing<String> {
        Zeroizing::new(text.into())
    }

    #[test]
    fn resolve_prefers_typed_password_and_requires_one_when_missing() {
        let stored = Some(Secret::Password {
            password: pw("stored"),
        });
        assert!(matches!(
            AuthSpec::resolve(&host(AuthMethod::Password), stored, Some(pw("typed"))),
            Ok(AuthSpec::Password(p)) if p.as_str() == "typed"
        ));
        let stored = Some(Secret::Password {
            password: pw("stored"),
        });
        assert!(matches!(
            AuthSpec::resolve(&host(AuthMethod::Password), stored, None),
            Ok(AuthSpec::Password(p)) if p.as_str() == "stored"
        ));
        assert!(matches!(
            AuthSpec::resolve(&host(AuthMethod::Interactive), None, None),
            Err(SshError::CredentialsRequired)
        ));
        assert!(matches!(
            AuthSpec::resolve(&host(AuthMethod::PrivateKey), None, None),
            Err(SshError::Key(_))
        ));
        assert!(matches!(
            AuthSpec::resolve(&host(AuthMethod::Agent), None, None),
            Ok(AuthSpec::Agent)
        ));
    }

    #[test]
    fn address_brackets_ipv6() {
        assert_eq!(
            SshTarget::from_host(&host(AuthMethod::Agent)).address(),
            "u@[::1]:22"
        );
    }

    #[test]
    fn config_ranges_are_checked() {
        assert!(SshConfig::default().validate().is_ok());
        let config = SshConfig {
            keepalive_secs: 1,
            ..SshConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
