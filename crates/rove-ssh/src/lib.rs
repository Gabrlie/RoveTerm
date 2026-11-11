//! SSH terminal provider for RoveTerm.
//!
//! [`SshConnector`] authenticates to servers, verifies host keys against the vault, and opens PTY
//! shell channels as [`OpenedTerminal`](rove_session::OpenedTerminal)s. See
//! `docs/subsystems/ssh.md`.

mod connection;
mod forward;
mod hostkey;
mod keys;
mod monitor;
mod os;
mod pool;
mod progress;
mod proxy;
mod sftp;
mod socks;
mod sshconfig;

use std::sync::Arc;
use std::time::Duration;

use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use rove_session::{OpenedTerminal, TermSize};
use rove_vault::{AuthMethod, Host, Secret, Vault};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub use connection::{ConnectOptions, ExecOutput, SshConnection, Via};
pub use forward::{ForwardState, ForwardStatus, Forwards};
pub use hostkey::{
    HostKeyAnswer, HostKeyPrompt, HostKeyPrompts, HostKeyRejection, HostKeyVerifier, VaultVerifier,
};
pub use keys::{
    KeyInfo, KeyType, authorize_command, check_public_key, describe as describe_key,
    generate as generate_key,
};
pub use monitor::{CpuTimes, Disk, NetInterface, ServerStats, sample as sample_server};
pub use os::probe as probe_os;
pub use progress::{AuthKind, ConnectStep, Progress};
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
    /// Identity of the path to the server: empty for a direct connection, otherwise the proxy or
    /// jump host. Connections are shared only between targets with the same route, so equal
    /// addresses behind different jump hosts never share a connection.
    pub route: String,
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

    /// Returns the address followed by the route, such as `user@host:22 via admin@bastion:22`.
    /// Equal only for targets that reach the same server the same way; used as the label of the
    /// target's SFTP file system, which identifies transfer destinations.
    pub fn location(&self) -> String {
        if self.route.is_empty() {
            self.address()
        } else {
            format!("{} {}", self.address(), self.route)
        }
    }

    /// Builds the target of a saved host. `jumps` is its current jump chain, nearest jump host
    /// first (empty unless the host uses a jump route); see [`route_key`].
    pub fn from_host(host: &Host, jumps: &[Host]) -> Self {
        Self {
            hostname: host.hostname.clone(),
            port: host.port,
            username: host.username.clone(),
            host_id: Some(host.id.to_string()),
            title: Some(host.name.clone()),
            route: route_key(&host.route, jumps),
        }
    }
}

/// The route identity of a saved host: built from where each hop actually goes (user, address,
/// port of every jump host, and the proxy at the end), never from record ids, so editing a jump
/// host to point elsewhere changes the identity and old connections are not reused. A chain
/// shorter than the route needs yields an identity that matches nothing else.
pub fn route_key(route: &rove_vault::HostRoute, jumps: &[Host]) -> String {
    match route {
        rove_vault::HostRoute::Direct => String::new(),
        rove_vault::HostRoute::Jump { host } => match jumps.split_first() {
            Some((jump, rest)) => {
                let hop = format!(
                    "via {}@{}:{}",
                    jump.username,
                    jump.hostname.to_ascii_lowercase(),
                    jump.port
                );
                match route_key(&jump.route, rest) {
                    further if further.is_empty() => hop,
                    further => format!("{hop} > {further}"),
                }
            }
            None => format!("via unresolved {host}"),
        },
        rove_vault::HostRoute::Proxy {
            protocol,
            host,
            port,
        } => {
            let scheme = match protocol {
                rove_vault::ProxyProtocol::Http => "http",
                rove_vault::ProxyProtocol::Socks5 => "socks5",
            };
            format!("proxy:{scheme}://{}:{port}", host.to_ascii_lowercase())
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
    /// Keys from the running SSH agent, then the default identity files (`~/.ssh/id_ed25519`,
    /// `id_ecdsa`, `id_rsa`) as OpenSSH does.
    Agent {
        /// Passphrase for encrypted default identity files.
        passphrase: Option<Zeroizing<String>>,
    },
}

impl AuthSpec {
    /// Resolves credentials for `host` from its stored `secret` and an optional password typed by
    /// the user. For [`AuthMethod::Key`] the caller passes the library key as
    /// [`Secret::PrivateKey`]. For key-based methods the typed value is a passphrase and takes precedence over
    /// a stored one. Returns [`SshError::CredentialsRequired`] when the user must be asked for a
    /// password first.
    pub fn resolve(
        host: &Host,
        secret: Option<Secret>,
        typed: Option<Zeroizing<String>>,
    ) -> Result<Self, SshError> {
        match (&host.auth, secret, typed) {
            (AuthMethod::Password | AuthMethod::Interactive, _, Some(password)) => {
                Ok(Self::Password(password))
            }
            (AuthMethod::Password, Some(Secret::Password { password }), None) => {
                Ok(Self::Password(password))
            }
            (AuthMethod::Password | AuthMethod::Interactive, _, None) => {
                Err(SshError::CredentialsRequired)
            }
            (
                AuthMethod::PrivateKey | AuthMethod::Key { .. },
                Some(Secret::PrivateKey { key, passphrase }),
                typed,
            ) => Ok(Self::PrivateKey {
                key,
                passphrase: typed.or(passphrase),
            }),
            (AuthMethod::PrivateKey | AuthMethod::Key { .. }, _, _) => Err(SshError::Key(
                "no private key is stored for this host".into(),
            )),
            (AuthMethod::KeyFile { path }, secret, typed) => Ok(Self::KeyFile {
                path: path.clone(),
                passphrase: typed.or(match secret {
                    Some(Secret::Passphrase { passphrase }) => Some(passphrase),
                    _ => None,
                }),
            }),
            (AuthMethod::Agent, _, typed) => Ok(Self::Agent { passphrase: typed }),
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
    /// A private key could not be read or parsed.
    #[error("private key error: {0}")]
    Key(String),
    /// A private key is encrypted and no passphrase, or a wrong one, was given.
    #[error("private key passphrase required")]
    PassphraseRequired,
    /// The SSH agent is unreachable or failed.
    #[error("ssh agent error: {0}")]
    Agent(String),
    /// Opening or configuring a channel failed.
    #[error("channel error: {0}")]
    Channel(String),
    /// A local forward could not listen on its address.
    #[error("cannot listen: {0}")]
    ForwardBind(String),
    /// The server refused to listen for a remote forward.
    #[error("server refused remote forward: {0}")]
    ForwardRejected(String),
    /// The proxy refused or failed to open a tunnel.
    #[error("proxy error: {0}")]
    Proxy(String),
    /// The jump host could not open a tunnel to the destination.
    #[error("jump host could not reach the destination: {0}")]
    Jump(String),
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
    options: parking_lot::RwLock<ConnectOptions>,
    verifier: Arc<dyn HostKeyVerifier>,
    pool: pool::Pool,
}

impl SshConnector {
    /// Creates a connector that verifies host keys with `verifier`.
    pub fn new(config: &SshConfig, verifier: Arc<dyn HostKeyVerifier>) -> Self {
        Self {
            options: parking_lot::RwLock::new(config.options()),
            verifier,
            pool: pool::Pool::default(),
        }
    }

    /// The timing new connections get.
    pub fn timing(&self) -> ConnectOptions {
        self.options.read().clone()
    }

    /// Changes the connect timeout and keepalive of connections opened from now on (the user's
    /// connection settings). Values are clamped to the ranges [`SshConfig::validate`] accepts.
    pub fn set_timing(&self, connect_timeout_secs: u64, keepalive_secs: u64, keepalive_max: usize) {
        let mut options = self.options.write();
        options.connect_timeout = Duration::from_secs(connect_timeout_secs.clamp(1, 120));
        options.keepalive_interval = Duration::from_secs(keepalive_secs.clamp(5, 600));
        options.keepalive_max = keepalive_max.clamp(1, 10);
    }

    /// Connects directly and authenticates to `target` on a new, unshared connection.
    pub async fn connect(
        &self,
        target: &SshTarget,
        auth: &AuthSpec,
    ) -> Result<Arc<SshConnection>, SshError> {
        self.connect_via(target, auth, &Via::Direct, None).await
    }

    /// Connects to `target` along `via` on a new, unshared connection.
    pub async fn connect_via(
        &self,
        target: &SshTarget,
        auth: &AuthSpec,
        via: &Via,
        progress: Option<Progress>,
    ) -> Result<Arc<SshConnection>, SshError> {
        let options = self.options.read().clone();
        SshConnection::connect(target, auth, via, self.verifier.clone(), &options, progress)
            .await
            .map(Arc::new)
    }

    /// Returns the shared connection to `target`, connecting directly with `auth` when none is
    /// live. `auth` is ignored when a live connection is reused.
    pub async fn shared(
        &self,
        target: &SshTarget,
        auth: &AuthSpec,
    ) -> Result<Arc<SshConnection>, SshError> {
        self.shared_via(target, auth, &Via::Direct, None).await
    }

    /// Returns the shared connection to `target`, connecting along `via` with `auth` when none is
    /// live, and reporting the steps to `progress`. `auth`, `via` and `progress` are unused when
    /// a live connection is reused.
    pub async fn shared_via(
        &self,
        target: &SshTarget,
        auth: &AuthSpec,
        via: &Via,
        progress: Option<Progress>,
    ) -> Result<Arc<SshConnection>, SshError> {
        let slot = self.pool.slot(pool::PoolKey::of(target));
        let mut guard = slot.connection.lock().await;
        if let Some(connection) = guard.as_ref().filter(|connection| !connection.is_closed()) {
            connection.touch();
            return Ok(connection.clone());
        }
        let connection = self.connect_via(target, auth, via, progress).await?;
        *guard = Some(connection.clone());
        Ok(connection)
    }

    /// Returns the live shared connection to `target`, if any, without connecting. Never waits:
    /// while a connection to `target` is being made (which can wait on the user, such as a host
    /// key confirmation), it returns `None` instead of waiting behind it.
    pub async fn live(&self, target: &SshTarget) -> Option<Arc<SshConnection>> {
        let slot = self.pool.slot(pool::PoolKey::of(target));
        let guard = slot.connection.try_lock().ok()?;
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
            ctx.provide(Arc::new(Forwards::new(ctx.events().clone())))?,
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
            route: rove_vault::HostRoute::Direct,
            tags: Vec::new(),
            created_at: 0,
            updated_at: 0,
            last_connected_at: None,
            os: None,
        }
    }

    fn pw(text: &str) -> Zeroizing<String> {
        Zeroizing::new(text.into())
    }

    struct Trusting;

    #[async_trait::async_trait]
    impl HostKeyVerifier for Trusting {
        async fn verify(
            &self,
            _hostname: &str,
            _port: u16,
            _route: &str,
            _key: &rove_vault::HostKey,
        ) -> Result<(), HostKeyRejection> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn live_does_not_wait_behind_a_connection_being_made() {
        let connector = SshConnector::new(&SshConfig::default(), Arc::new(Trusting));
        let target = SshTarget {
            hostname: "203.0.113.1".into(),
            port: 22,
            username: "root".into(),
            host_id: None,
            title: None,
            route: String::new(),
        };
        // A connection attempt holds the slot (as one waiting on the user would).
        let slot = connector.pool.slot(pool::PoolKey::of(&target));
        let _connecting = slot.connection.lock().await;
        let found = tokio::time::timeout(std::time::Duration::from_secs(1), connector.live(&target))
            .await
            .expect("live waited for the connection attempt");
        assert!(found.is_none());
    }

    #[test]
    fn location_tells_routes_apart() {
        let direct = SshTarget {
            hostname: "10.0.0.5".into(),
            port: 22,
            username: "root".into(),
            host_id: None,
            title: None,
            route: String::new(),
        };
        let behind = SshTarget {
            route: "via admin@bastion:22".into(),
            ..direct.clone()
        };
        assert_eq!(direct.location(), "root@10.0.0.5:22");
        assert_eq!(behind.location(), "root@10.0.0.5:22 via admin@bastion:22");
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
            Ok(AuthSpec::Agent { passphrase: None })
        ));
    }

    #[test]
    fn resolve_prefers_typed_passphrase_for_keys() {
        let key_file = host(AuthMethod::KeyFile {
            path: "~/.ssh/id".into(),
        });
        let stored = || {
            Some(Secret::Passphrase {
                passphrase: pw("stored"),
            })
        };
        assert!(matches!(
            AuthSpec::resolve(&key_file, stored(), Some(pw("typed"))),
            Ok(AuthSpec::KeyFile { passphrase: Some(p), .. }) if p.as_str() == "typed"
        ));
        assert!(matches!(
            AuthSpec::resolve(&key_file, stored(), None),
            Ok(AuthSpec::KeyFile { passphrase: Some(p), .. }) if p.as_str() == "stored"
        ));
        let stored_key = Some(Secret::PrivateKey {
            key: pw("KEY"),
            passphrase: None,
        });
        assert!(matches!(
            AuthSpec::resolve(&host(AuthMethod::PrivateKey), stored_key, Some(pw("typed"))),
            Ok(AuthSpec::PrivateKey { passphrase: Some(p), .. }) if p.as_str() == "typed"
        ));
        assert!(matches!(
            AuthSpec::resolve(&host(AuthMethod::Agent), None, Some(pw("typed"))),
            Ok(AuthSpec::Agent { passphrase: Some(p) }) if p.as_str() == "typed"
        ));
    }

    #[test]
    fn route_identity_follows_where_the_jump_hosts_go() {
        let direct = host(AuthMethod::Agent);
        let jump_id = HostId::new();
        let target = Host {
            route: rove_vault::HostRoute::Jump { host: jump_id },
            ..host(AuthMethod::Agent)
        };
        let bastion = |hostname: &str| Host {
            id: jump_id,
            hostname: hostname.into(),
            ..host(AuthMethod::Agent)
        };
        assert_eq!(SshTarget::from_host(&direct, &[]).route, "");
        let before = SshTarget::from_host(&target, &[bastion("bastion-a")]);
        // The same jump host record edited to point at another server.
        let after = SshTarget::from_host(&target, &[bastion("bastion-b")]);
        assert_eq!(before.route, "via u@bastion-a:22");
        assert_ne!(before.route, after.route);
        // A jump host that itself goes through a proxy.
        let proxied = Host {
            route: rove_vault::HostRoute::Proxy {
                protocol: rove_vault::ProxyProtocol::Socks5,
                host: "Proxy.lan".into(),
                port: 1080,
            },
            ..bastion("bastion-a")
        };
        assert_eq!(
            SshTarget::from_host(&target, &[proxied]).route,
            "via u@bastion-a:22 > proxy:socks5://proxy.lan:1080"
        );
        assert!(
            SshTarget::from_host(&target, &[])
                .route
                .starts_with("via unresolved")
        );
    }

    #[test]
    fn address_brackets_ipv6() {
        assert_eq!(
            SshTarget::from_host(&host(AuthMethod::Agent), &[]).address(),
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
