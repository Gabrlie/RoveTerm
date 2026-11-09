//! An authenticated SSH connection and the PTY shell channels opened on it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use rove_session::{
    ExitStatus, OpenedTerminal, SessionError, SessionInfo, SessionKind, TermSize, TerminalOutput,
    TerminalSession,
};
use rove_vault::HostKey;
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, ChannelWriteHalf, Disconnect};
use russh_sftp::client::SftpSession;
use tokio::sync::mpsc;

use crate::forward::RemoteRoutes;
use crate::keys;
use crate::{AuthSpec, HostKeyRejection, HostKeyVerifier, SftpFs, SshError, SshTarget};

const OUTPUT_QUEUE: usize = 256;

/// Timing parameters for a connection.
#[derive(Debug, Clone)]
pub struct ConnectOptions {
    /// Limit for establishing the TCP connection.
    pub connect_timeout: Duration,
    /// Interval between keepalive requests.
    pub keepalive_interval: Duration,
    /// Unanswered keepalives before the connection is considered dead.
    pub keepalive_max: usize,
    /// Disconnect this long after the last channel closes; `None` keeps the connection until it
    /// is dropped.
    pub idle_timeout: Option<Duration>,
}

struct ClientHandler {
    hostname: String,
    port: u16,
    verifier: Arc<dyn HostKeyVerifier>,
    rejection: Arc<Mutex<Option<HostKeyRejection>>>,
    routes: Arc<RemoteRoutes>,
}

fn host_key(key: &russh::keys::PublicKey) -> Result<HostKey, HostKeyRejection> {
    let public_key = key
        .to_openssh()
        .map_err(|error| HostKeyRejection::Storage(format!("encode host key: {error}")))?;
    Ok(HostKey {
        algorithm: key.algorithm().as_str().to_owned(),
        public_key,
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    })
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let certificate_key;
        let key = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            PublicKeyOrCertificate::Certificate(certificate) => {
                certificate_key = russh::keys::PublicKey::from(certificate.public_key().clone());
                &certificate_key
            }
        };
        let verdict = match host_key(key) {
            Ok(key) => self.verifier.verify(&self.hostname, self.port, &key).await,
            Err(rejection) => Err(rejection),
        };
        match verdict {
            Ok(()) => Ok(true),
            Err(rejection) => {
                *self.rejection.lock() = Some(rejection);
                Ok(false)
            }
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        match self.routes.resolve(connected_address, connected_port) {
            Some(route) => {
                reply.accept().await;
                tokio::spawn(route.serve(channel));
            }
            None => {
                tracing::debug!(
                    connected_address,
                    connected_port,
                    "unexpected forwarded connection"
                );
                reply
                    .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                    .await;
            }
        }
        Ok(())
    }
}

/// An authenticated SSH connection. Cloneable handles share the same TCP connection; channels opened
/// on it are independent.
pub struct SshConnection {
    handle: Handle<ClientHandler>,
    target: SshTarget,
    channels: AtomicUsize,
    idle_generation: AtomicU64,
    retired: AtomicBool,
    idle_timeout: Option<Duration>,
    routes: Arc<RemoteRoutes>,
}

impl SshConnection {
    /// Connects to `target`, verifies the host key with `verifier`, and authenticates with `auth`.
    pub async fn connect(
        target: &SshTarget,
        auth: &AuthSpec,
        verifier: Arc<dyn HostKeyVerifier>,
        options: &ConnectOptions,
    ) -> Result<Self, SshError> {
        let address = (target.hostname.as_str(), target.port);
        let stream = tokio::time::timeout(
            options.connect_timeout,
            tokio::net::TcpStream::connect(address),
        )
        .await
        .map_err(|_| SshError::Timeout)?
        .map_err(|error| SshError::Connect(error.to_string()))?;
        if let Err(error) = stream.set_nodelay(true) {
            tracing::debug!(%error, "set_nodelay failed");
        }
        let config = Arc::new(client::Config {
            keepalive_interval: Some(options.keepalive_interval),
            keepalive_max: options.keepalive_max,
            inactivity_timeout: None,
            ..Default::default()
        });
        let rejection = Arc::new(Mutex::new(None));
        let routes = Arc::new(RemoteRoutes::default());
        let handler = ClientHandler {
            hostname: target.hostname.clone(),
            port: target.port,
            verifier,
            rejection: rejection.clone(),
            routes: routes.clone(),
        };
        let mut handle = match client::connect_stream(config, stream, handler).await {
            Ok(handle) => handle,
            Err(error) => {
                return Err(match rejection.lock().take() {
                    Some(rejection) => SshError::HostKey(rejection),
                    None => SshError::Protocol(error.to_string()),
                });
            }
        };
        authenticate(&mut handle, target, auth).await?;
        Ok(Self {
            handle,
            target: target.clone(),
            channels: AtomicUsize::new(0),
            idle_generation: AtomicU64::new(0),
            retired: AtomicBool::new(false),
            idle_timeout: options.idle_timeout,
            routes,
        })
    }

    /// Returns `true` once the underlying connection has ended.
    pub fn is_closed(&self) -> bool {
        self.retired.load(Ordering::SeqCst) || self.handle.is_closed()
    }

    /// Returns the number of open channels on this connection.
    pub fn open_channels(&self) -> usize {
        self.channels.load(Ordering::SeqCst)
    }

    /// Postpones a pending idle disconnect; called when the pool hands this connection out.
    pub(crate) fn touch(&self) {
        self.idle_generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Returns the target this connection was made to.
    pub fn target(&self) -> &SshTarget {
        &self.target
    }

    pub(crate) fn lease(self: &Arc<Self>) -> ChannelLease {
        self.channels.fetch_add(1, Ordering::SeqCst);
        self.idle_generation.fetch_add(1, Ordering::SeqCst);
        ChannelLease {
            connection: self.clone(),
        }
    }

    /// Opens a session channel with a PTY and a login shell.
    ///
    /// The returned terminal keeps `self` alive; the connection stays open while any of its
    /// terminals is open.
    pub async fn open_shell(self: &Arc<Self>, size: TermSize) -> Result<OpenedTerminal, SshError> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(channel_error)?;
        channel
            .request_pty(
                false,
                "xterm-256color",
                u32::from(size.cols),
                u32::from(size.rows),
                0,
                0,
                &[],
            )
            .await
            .map_err(channel_error)?;
        channel.request_shell(false).await.map_err(channel_error)?;
        let (mut reader, writer) = channel.split();
        let (tx, rx) = mpsc::channel(OUTPUT_QUEUE);
        tokio::spawn(async move {
            let mut status = ExitStatus::default();
            while let Some(message) = reader.wait().await {
                match message {
                    ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                        if tx
                            .send(TerminalOutput::Data(Bytes::copy_from_slice(&data)))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        status.code = i32::try_from(exit_status).ok()
                    }
                    ChannelMsg::ExitSignal { signal_name, .. } => {
                        status.signal = Some(format!("{signal_name:?}"))
                    }
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            let _receiver_may_be_gone = tx.send(TerminalOutput::Exit(status)).await;
        });
        let address = self.target.address();
        Ok(OpenedTerminal {
            session: Arc::new(SshTerminal {
                writer,
                closed: AtomicBool::new(false),
                _lease: self.lease(),
            }),
            output: rx,
            info: SessionInfo {
                kind: SessionKind::Ssh {
                    host_id: self.target.host_id.clone(),
                    address: address.clone(),
                },
                title: self.target.title.clone().unwrap_or(address),
            },
        })
    }

    /// Opens an SFTP session on a new channel. The session keeps the connection open until dropped.
    pub async fn open_sftp(self: &Arc<Self>) -> Result<SftpFs, SshError> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(channel_error)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(channel_error)?;
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure | ChannelMsg::Close) | None => {
                    return Err(SshError::Sftp(
                        "the server does not offer the sftp subsystem".into(),
                    ));
                }
                Some(_) => {}
            }
        }
        let lease = self.lease();
        let session = SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        Ok(SftpFs::new(session, self.target.address(), lease))
    }

    /// Opens a `direct-tcpip` channel to `host:port` as seen from the server. The lease keeps the
    /// connection open while the stream is in use.
    pub(crate) async fn open_direct_tcpip(
        self: &Arc<Self>,
        host: &str,
        port: u16,
        originator_address: &str,
        originator_port: u16,
    ) -> Result<(russh::Channel<client::Msg>, ChannelLease), SshError> {
        let channel = self
            .handle
            .channel_open_direct_tcpip(
                host,
                u32::from(port),
                originator_address,
                u32::from(originator_port),
            )
            .await
            .map_err(channel_error)?;
        Ok((channel, self.lease()))
    }

    /// Asks the server to listen on `address:port`. Returns the allocated port, or 0 when the
    /// server confirmed the requested port without naming it.
    pub(crate) async fn request_remote_forward(
        &self,
        address: &str,
        port: u32,
    ) -> Result<u32, SshError> {
        self.handle
            .tcpip_forward(address, port)
            .await
            .map_err(|error| SshError::ForwardRejected(format!("{address}:{port}: {error}")))
    }

    /// Asks the server to stop listening on `address:port`.
    pub(crate) async fn cancel_remote_forward(
        &self,
        address: &str,
        port: u32,
    ) -> Result<(), SshError> {
        self.handle
            .cancel_tcpip_forward(address, port)
            .await
            .map_err(|error| SshError::Protocol(error.to_string()))
    }

    pub(crate) fn remote_routes(&self) -> &RemoteRoutes {
        &self.routes
    }

    /// Ends the connection.
    pub async fn disconnect(&self) {
        if let Err(error) = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await
        {
            tracing::debug!(%error, "disconnect failed; connection already closed");
        }
    }
}

fn channel_error(error: russh::Error) -> SshError {
    SshError::Channel(error.to_string())
}

async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    target: &SshTarget,
    auth: &AuthSpec,
) -> Result<(), SshError> {
    let user = target.username.clone();
    let protocol = |error: russh::Error| SshError::Protocol(error.to_string());
    let success = match auth {
        AuthSpec::Password(password) => {
            let result = handle
                .authenticate_password(user.clone(), password.as_str())
                .await
                .map_err(protocol)?;
            result.success() || keyboard_interactive(handle, &user, password).await?
        }
        AuthSpec::PrivateKey { key, passphrase } => {
            let key = keys::decode(key, passphrase.as_deref().map(String::as_str))?;
            publickey(handle, user, key).await?
        }
        AuthSpec::KeyFile { path, passphrase } => {
            let key = keys::load(path, passphrase.as_deref().map(String::as_str))?;
            publickey(handle, user, key).await?
        }
        AuthSpec::Agent { passphrase } => {
            agent_then_defaults(handle, user, passphrase.as_deref().map(String::as_str)).await?
        }
    };
    if success {
        Ok(())
    } else {
        Err(SshError::AuthFailed)
    }
}

async fn publickey(
    handle: &mut Handle<ClientHandler>,
    user: String,
    key: russh::keys::PrivateKey,
) -> Result<bool, SshError> {
    let protocol = |error: russh::Error| SshError::Protocol(error.to_string());
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(protocol)?
        .flatten();
    let result = handle
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await
        .map_err(protocol)?;
    Ok(result.success())
}

async fn keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    password: &zeroize::Zeroizing<String>,
) -> Result<bool, SshError> {
    use russh::client::KeyboardInteractiveAuthResponse as Response;
    let protocol = |error: russh::Error| SshError::Protocol(error.to_string());
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await
        .map_err(protocol)?;
    // Servers may ask several rounds; answer every prompt with the password, at most three rounds.
    for _ in 0..3 {
        match response {
            Response::Success => return Ok(true),
            Response::Failure { .. } => return Ok(false),
            Response::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_string()).collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(protocol)?;
            }
        }
    }
    Ok(false)
}

/// Tries the SSH agent, then the default identity files. An unreachable agent is not an error
/// while a default identity remains to try. Fails with [`SshError::PassphraseRequired`] when
/// nothing succeeded and an encrypted default identity could not be opened.
async fn agent_then_defaults(
    handle: &mut Handle<ClientHandler>,
    user: String,
    passphrase: Option<&str>,
) -> Result<bool, SshError> {
    let agent_error = match agent(handle, user.clone()).await {
        Ok(true) => return Ok(true),
        Ok(false) => None,
        Err(SshError::Agent(reason)) => Some(reason),
        Err(error) => return Err(error),
    };
    let home = std::env::home_dir();
    let defaults = keys::default_keys(home.as_deref(), passphrase);
    for key in defaults.keys {
        if publickey(handle, user.clone(), key).await? {
            return Ok(true);
        }
    }
    if defaults.locked {
        return Err(SshError::PassphraseRequired);
    }
    match agent_error {
        Some(reason) => Err(SshError::Agent(reason)),
        None => Ok(false),
    }
}

#[cfg(unix)]
async fn agent(handle: &mut Handle<ClientHandler>, user: String) -> Result<bool, SshError> {
    let agent_error = |error: russh::keys::Error| SshError::Agent(error.to_string());
    let mut agent = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .map_err(agent_error)?;
    agent_try_keys(handle, user, &mut agent).await
}

#[cfg(windows)]
async fn agent(handle: &mut Handle<ClientHandler>, user: String) -> Result<bool, SshError> {
    use russh::keys::agent::client::AgentClient;
    let agent_error = |error: russh::keys::Error| SshError::Agent(error.to_string());
    match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(mut agent) => agent_try_keys(handle, user, &mut agent).await,
        Err(_) => {
            let mut agent = AgentClient::connect_pageant().await.map_err(agent_error)?;
            agent_try_keys(handle, user, &mut agent).await
        }
    }
}

async fn agent_try_keys<S>(
    handle: &mut Handle<ClientHandler>,
    user: String,
    agent: &mut russh::keys::agent::client::AgentClient<S>,
) -> Result<bool, SshError>
where
    S: russh::keys::agent::client::AgentStream + Send + Unpin + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| SshError::Agent(error.to_string()))?;
    let protocol = |error: russh::Error| SshError::Protocol(error.to_string());
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(protocol)?
        .flatten();
    for identity in identities {
        let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        let result = handle
            .authenticate_publickey_with(user.clone(), key, hash, agent)
            .await
            .map_err(|error| SshError::Agent(error.to_string()))?;
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}

struct SshTerminal {
    writer: ChannelWriteHalf<client::Msg>,
    closed: AtomicBool,
    _lease: ChannelLease,
}

/// Counts an open channel. When the last lease of a connection drops, the connection is
/// disconnected after its idle timeout unless a new channel opens first.
pub(crate) struct ChannelLease {
    connection: Arc<SshConnection>,
}

impl Drop for ChannelLease {
    fn drop(&mut self) {
        let connection = &self.connection;
        if connection.channels.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }
        let Some(idle) = connection.idle_timeout else {
            return;
        };
        let generation = connection.idle_generation.load(Ordering::SeqCst);
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let connection = connection.clone();
        runtime.spawn(async move {
            tokio::time::sleep(idle).await;
            let still_idle = connection.channels.load(Ordering::SeqCst) == 0
                && connection.idle_generation.load(Ordering::SeqCst) == generation;
            if still_idle && !connection.retired.swap(true, Ordering::SeqCst) {
                tracing::debug!(address = %connection.target.address(), "closing idle ssh connection");
                connection.disconnect().await;
            }
        });
    }
}

#[async_trait]
impl TerminalSession for SshTerminal {
    async fn write(&self, data: &[u8]) -> Result<(), SessionError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SessionError::Closed);
        }
        self.writer
            .data_bytes(Bytes::copy_from_slice(data))
            .await
            .map_err(|_| SessionError::Closed)
    }

    async fn resize(&self, size: TermSize) -> Result<(), SessionError> {
        self.writer
            .window_change(u32::from(size.cols), u32::from(size.rows), 0, 0)
            .await
            .map_err(|_| SessionError::Closed)
    }

    async fn close(&self) -> Result<(), SessionError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Err(error) = self.writer.close().await {
            tracing::debug!(%error, "closing ssh channel failed; connection already gone");
        }
        Ok(())
    }
}
