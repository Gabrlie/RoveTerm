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
use tokio::sync::mpsc;

use crate::{AuthSpec, HostKeyRejection, HostKeyVerifier, SshError, SshTarget};

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
        let handler = ClientHandler {
            hostname: target.hostname.clone(),
            port: target.port,
            verifier,
            rejection: rejection.clone(),
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

    fn lease(self: &Arc<Self>) -> ChannelLease {
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
            let key =
                russh::keys::decode_secret_key(key, passphrase.as_deref().map(String::as_str))
                    .map_err(|error| SshError::Key(error.to_string()))?;
            publickey(handle, user, key).await?
        }
        AuthSpec::KeyFile { path, passphrase } => {
            let key = russh::keys::load_secret_key(path, passphrase.as_deref().map(String::as_str))
                .map_err(|error| SshError::Key(error.to_string()))?;
            publickey(handle, user, key).await?
        }
        AuthSpec::Agent => agent(handle, user).await?,
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
struct ChannelLease {
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
