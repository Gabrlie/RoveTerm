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
use crate::progress::{AuthKind, ConnectStep, Progress, Steps};
use crate::{AuthSpec, HostKeyRejection, HostKeyVerifier, SftpFs, SshError, SshTarget};

const OUTPUT_QUEUE: usize = 256;

/// Result of [`SshConnection::exec`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    /// Standard output, at most the requested limit.
    pub stdout: Vec<u8>,
    /// Standard error, at most the requested limit.
    pub stderr: Vec<u8>,
    /// Exit status, when the server sent one.
    pub code: Option<i32>,
    /// Some output was discarded because it exceeded the limit.
    pub truncated: bool,
}

/// How the transport to a server is established.
pub enum Via {
    /// A TCP connection to the server.
    Direct,
    /// A TCP connection to a proxy, which then opens a tunnel to the server.
    Proxy {
        /// Proxy protocol.
        protocol: rove_vault::ProxyProtocol,
        /// Proxy host.
        host: String,
        /// Proxy port.
        port: u16,
    },
    /// A `direct-tcpip` channel on an authenticated connection to a jump host. The new connection
    /// keeps the jump connection alive while it exists.
    Jump(Arc<SshConnection>),
}

impl Via {
    /// Describes a proxy or jump host for progress reports; `None` when direct.
    fn describe(&self) -> Option<String> {
        match self {
            Self::Direct => None,
            Self::Proxy {
                protocol,
                host,
                port,
            } => {
                let scheme = match protocol {
                    rove_vault::ProxyProtocol::Http => "http",
                    rove_vault::ProxyProtocol::Socks5 => "socks5",
                };
                Some(format!("{scheme}://{host}:{port}"))
            }
            Self::Jump(jump) => Some(jump.target().address()),
        }
    }
}

/// The byte stream an SSH session runs over.
trait Transport: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Transport for T {}

async fn tcp(host: &str, port: u16) -> Result<tokio::net::TcpStream, SshError> {
    let stream = tokio::net::TcpStream::connect((host, port))
        .await
        .map_err(|error| SshError::Connect(error.to_string()))?;
    if let Err(error) = stream.set_nodelay(true) {
        tracing::debug!(%error, "set_nodelay failed");
    }
    Ok(stream)
}

/// Opens the transport to `target` along `via`. A jump returns the lease that keeps the jump
/// connection open.
async fn open_transport(
    target: &SshTarget,
    via: &Via,
) -> Result<(Box<dyn Transport>, Option<ChannelLease>), SshError> {
    match via {
        Via::Direct => Ok((Box::new(tcp(&target.hostname, target.port).await?), None)),
        Via::Proxy {
            protocol,
            host,
            port,
        } => {
            let mut stream = tcp(host, *port).await?;
            match protocol {
                rove_vault::ProxyProtocol::Http => {
                    crate::proxy::http_connect(&mut stream, &target.hostname, target.port).await?
                }
                rove_vault::ProxyProtocol::Socks5 => {
                    crate::proxy::socks5_connect(&mut stream, &target.hostname, target.port).await?
                }
            }
            Ok((Box::new(stream), None))
        }
        Via::Jump(jump) => {
            let (channel, lease) = jump
                .open_direct_tcpip(&target.hostname, target.port, "127.0.0.1", 0)
                .await
                .map_err(|error| match error {
                    SshError::Channel(reason) => SshError::Jump(reason),
                    other => other,
                })?;
            Ok((Box::new(channel.into_stream()), Some(lease)))
        }
    }
}

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
    route: String,
    verifier: Arc<dyn HostKeyVerifier>,
    rejection: Arc<Mutex<Option<HostKeyRejection>>>,
    routes: Arc<RemoteRoutes>,
    steps: Steps,
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
            Ok(key) => {
                self.verifier
                    .verify(&self.hostname, self.port, &self.route, &key)
                    .await
            }
            Err(rejection) => Err(rejection),
        };
        match verdict {
            Ok(()) => {
                self.steps.emit(ConnectStep::HostKeyAccepted {
                    algorithm: key.algorithm().as_str().to_owned(),
                    fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                });
                Ok(true)
            }
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
    /// Keeps the jump connection open while this connection exists.
    _tunnel: Option<ChannelLease>,
}

impl SshConnection {
    /// Connects to `target` along `via`, verifies the host key with `verifier`, and authenticates
    /// with `auth`. `connect_timeout` covers opening the transport, including proxy and jump
    /// handshakes.
    pub async fn connect(
        target: &SshTarget,
        auth: &AuthSpec,
        via: &Via,
        verifier: Arc<dyn HostKeyVerifier>,
        options: &ConnectOptions,
        progress: Option<Progress>,
    ) -> Result<Self, SshError> {
        let steps = Steps(progress);
        steps.emit(ConnectStep::Connecting {
            address: target.address(),
            via: via.describe(),
        });
        let (stream, tunnel) =
            tokio::time::timeout(options.connect_timeout, open_transport(target, via))
                .await
                .map_err(|_| SshError::Timeout)??;
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
            route: target.route.clone(),
            verifier,
            rejection: rejection.clone(),
            routes: routes.clone(),
            steps: steps.clone(),
        };
        steps.emit(ConnectStep::Handshake);
        let mut handle = match client::connect_stream(config, stream, handler).await {
            Ok(handle) => handle,
            Err(error) => {
                return Err(match rejection.lock().take() {
                    Some(rejection) => SshError::HostKey(rejection),
                    None => SshError::Protocol(error.to_string()),
                });
            }
        };
        authenticate(&mut handle, target, auth, &steps).await?;
        Ok(Self {
            handle,
            target: target.clone(),
            channels: AtomicUsize::new(0),
            idle_generation: AtomicU64::new(0),
            retired: AtomicBool::new(false),
            idle_timeout: options.idle_timeout,
            routes,
            _tunnel: tunnel,
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

    /// The connections this one runs through, first jump host first, ending with this
    /// connection. A direct or proxied connection is its own whole chain.
    pub fn chain(self: &Arc<Self>) -> Vec<Arc<SshConnection>> {
        let mut chain = vec![self.clone()];
        let mut current = self.clone();
        while let Some(next) = current
            ._tunnel
            .as_ref()
            .map(|lease| lease.connection.clone())
        {
            chain.push(next.clone());
            current = next;
        }
        chain.reverse();
        chain
    }

    /// Time for one protocol round trip (`keepalive@openssh.com` and its reply), in
    /// microseconds.
    pub async fn round_trip(&self) -> Result<u64, SshError> {
        let started = std::time::Instant::now();
        self.handle
            .send_ping()
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;
        Ok(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX))
    }

    /// Latency of each segment of the path, in microseconds: one entry per hop of [`chain`],
    /// the first from this machine to the first server, each later one from the previous jump
    /// host to the next server. A proxy is not a separate segment. Round trips are measured
    /// concurrently; a segment is the difference between consecutive round trips (at least 0).
    ///
    /// [`chain`]: Self::chain
    pub async fn hop_latency(self: &Arc<Self>) -> Result<Vec<u64>, SshError> {
        let pings: Vec<_> = self
            .chain()
            .into_iter()
            .map(|connection| tokio::spawn(async move { connection.round_trip().await }))
            .collect();
        let mut segments = Vec::with_capacity(pings.len());
        let mut reached = 0;
        for ping in pings {
            let round_trip = ping
                .await
                .map_err(|error| SshError::Channel(error.to_string()))??;
            segments.push(round_trip.saturating_sub(reached));
            reached = reached.max(round_trip);
        }
        Ok(segments)
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
        Ok(SftpFs::new(session, self.target.location(), lease))
    }

    /// Runs `command` on the server without a PTY and collects its output. Output beyond `limit`
    /// bytes per stream is discarded; [`ExecOutput::truncated`] tells. The caller bounds the time.
    pub async fn exec(
        self: &Arc<Self>,
        command: &str,
        limit: usize,
    ) -> Result<ExecOutput, SshError> {
        self.exec_controlled(
            command,
            limit,
            tokio_util::sync::CancellationToken::new(),
            None,
        )
        .await
    }

    /// Runs one exec channel and waits for remote exit evidence after requesting termination.
    pub async fn exec_controlled(
        self: &Arc<Self>,
        command: &str,
        limit: usize,
        cancel: tokio_util::sync::CancellationToken,
        timeout: Option<Duration>,
    ) -> Result<ExecOutput, SshError> {
        let deadline = async {
            match timeout {
                Some(time) => tokio::time::sleep(time).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(deadline);
        let mut channel = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(SshError::Channel("command cancelled before execution".into())),
            () = &mut deadline => return Err(SshError::Channel("command timed out before execution".into())),
            channel = self.handle.channel_open_session() => channel.map_err(channel_error)?,
        };
        let _lease = self.lease();
        channel.exec(true, command).await.map_err(channel_error)?;
        let mut output = ExecOutput::default();
        // Appends what fits; returns `true` when something was dropped.
        let append = |buffer: &mut Vec<u8>, data: &[u8]| {
            let room = limit.saturating_sub(buffer.len());
            buffer.extend_from_slice(&data[..data.len().min(room)]);
            data.len() > room
        };
        let mut truncated = false;
        let mut confirmed = false;
        let mut interrupted = None;
        loop {
            let message = tokio::select! {
                biased;
                () = cancel.cancelled() => { interrupted = Some("cancelled"); break; },
                () = &mut deadline => { interrupted = Some("timed out"); break; },
                message = channel.wait() => match message { Some(message) => message, None => break },
            };
            match message {
                ChannelMsg::Data { data } => truncated |= append(&mut output.stdout, &data),
                ChannelMsg::ExtendedData { data, ext: 1 } => {
                    truncated |= append(&mut output.stderr, &data)
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    confirmed = true;
                    output.code = i32::try_from(exit_status).ok();
                }
                ChannelMsg::ExitSignal { .. } => confirmed = true,
                ChannelMsg::Failure => {
                    return Err(SshError::Channel(
                        "the server refused to run the command".into(),
                    ));
                }
                _ => {}
            }
        }
        if let Some(reason) = interrupted {
            for signal in [russh::Sig::TERM, russh::Sig::KILL] {
                if confirmed {
                    break;
                }
                let result = tokio::time::timeout(Duration::from_secs(2), async {
                    channel.signal(signal).await.ok()?;
                    while let Some(message) = channel.wait().await {
                        match message {
                            ChannelMsg::ExitStatus { .. } | ChannelMsg::ExitSignal { .. } => {
                                return Some(true);
                            }
                            ChannelMsg::Close => return None,
                            _ => {}
                        }
                    }
                    None
                })
                .await;
                match result {
                    Ok(Some(true)) => confirmed = true,
                    Ok(None) => break,
                    _ => {}
                }
            }
            let _ = tokio::time::timeout(Duration::from_secs(1), channel.close()).await;
            return Err(SshError::Channel(if confirmed {
                format!("{reason}: remote command exited")
            } else {
                format!("{reason}: could not confirm that the remote command stopped")
            }));
        }
        if timeout.is_some() && !confirmed {
            return Err(SshError::Channel(
                "could not confirm that the remote command stopped".into(),
            ));
        }
        output.truncated = truncated;
        Ok(output)
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
    steps: &Steps,
) -> Result<(), SshError> {
    let user = target.username.clone();
    let protocol = |error: russh::Error| SshError::Protocol(error.to_string());
    let success = match auth {
        AuthSpec::Password(password) => {
            steps.emit(ConnectStep::Trying {
                method: AuthKind::Password,
                label: None,
            });
            let result = handle
                .authenticate_password(user.clone(), password.as_str())
                .await
                .map_err(protocol)?;
            if result.success() {
                true
            } else {
                steps.emit(ConnectStep::Rejected {
                    method: AuthKind::Password,
                    label: None,
                });
                keyboard_interactive(handle, &user, password, steps).await?
            }
        }
        AuthSpec::PrivateKey { key, passphrase } => {
            let key = keys::decode(key, passphrase.as_deref().map(String::as_str))?;
            publickey(handle, user, key, None, steps).await?
        }
        AuthSpec::KeyFile { path, passphrase } => {
            let key = keys::load(path, passphrase.as_deref().map(String::as_str))?;
            publickey(handle, user, key, Some(path.clone()), steps).await?
        }
        AuthSpec::Agent { passphrase } => {
            agent_then_defaults(
                handle,
                user,
                passphrase.as_deref().map(String::as_str),
                steps,
            )
            .await?
        }
    };
    if success {
        steps.emit(ConnectStep::Authenticated);
        Ok(())
    } else {
        Err(SshError::AuthFailed)
    }
}

/// Offers one private key. `label` names it in progress reports; `None` uses its comment.
async fn publickey(
    handle: &mut Handle<ClientHandler>,
    user: String,
    key: russh::keys::PrivateKey,
    label: Option<String>,
    steps: &Steps,
) -> Result<bool, SshError> {
    let protocol = |error: russh::Error| SshError::Protocol(error.to_string());
    let label = label.unwrap_or_else(|| key_label(key.public_key()));
    steps.emit(ConnectStep::Trying {
        method: AuthKind::PublicKey,
        label: Some(label.clone()),
    });
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(protocol)?
        .flatten();
    let result = handle
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await
        .map_err(protocol)?;
    if !result.success() {
        steps.emit(ConnectStep::Rejected {
            method: AuthKind::PublicKey,
            label: Some(label),
        });
    }
    Ok(result.success())
}

/// Names a public key by its comment, else its algorithm and fingerprint.
fn key_label(key: &russh::keys::PublicKey) -> String {
    let comment = AsRef::<str>::as_ref(key.comment()).trim();
    if comment.is_empty() {
        format!(
            "{} {}",
            key.algorithm().as_str(),
            key.fingerprint(HashAlg::Sha256)
        )
    } else {
        comment.to_owned()
    }
}

async fn keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    password: &zeroize::Zeroizing<String>,
    steps: &Steps,
) -> Result<bool, SshError> {
    use russh::client::KeyboardInteractiveAuthResponse as Response;
    let protocol = |error: russh::Error| SshError::Protocol(error.to_string());
    steps.emit(ConnectStep::Trying {
        method: AuthKind::KeyboardInteractive,
        label: None,
    });
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await
        .map_err(protocol)?;
    // Servers may ask several rounds; answer every prompt with the password, at most three rounds.
    for _ in 0..3 {
        match response {
            Response::Success => return Ok(true),
            Response::Failure { .. } => break,
            Response::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_string()).collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(protocol)?;
            }
        }
    }
    steps.emit(ConnectStep::Rejected {
        method: AuthKind::KeyboardInteractive,
        label: None,
    });
    Ok(false)
}

/// Tries the SSH agent, then the default identity files. An unreachable agent is not an error
/// while a default identity remains to try. Fails with [`SshError::PassphraseRequired`] when
/// nothing succeeded and an encrypted default identity could not be opened.
async fn agent_then_defaults(
    handle: &mut Handle<ClientHandler>,
    user: String,
    passphrase: Option<&str>,
    steps: &Steps,
) -> Result<bool, SshError> {
    let agent_error = match agent(handle, user.clone(), steps).await {
        Ok(true) => return Ok(true),
        Ok(false) => None,
        Err(SshError::Agent(reason)) => {
            steps.emit(ConnectStep::AgentUnavailable {
                reason: reason.clone(),
            });
            Some(reason)
        }
        Err(error) => return Err(error),
    };
    let home = std::env::home_dir();
    let defaults = keys::default_keys(home.as_deref(), passphrase);
    for (path, key) in defaults.keys {
        let label = Some(path.display().to_string());
        if publickey(handle, user.clone(), key, label, steps).await? {
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
async fn agent(
    handle: &mut Handle<ClientHandler>,
    user: String,
    steps: &Steps,
) -> Result<bool, SshError> {
    let agent_error = |error: russh::keys::Error| SshError::Agent(error.to_string());
    let mut agent = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .map_err(agent_error)?;
    agent_try_keys(handle, user, &mut agent, steps).await
}

#[cfg(windows)]
async fn agent(
    handle: &mut Handle<ClientHandler>,
    user: String,
    steps: &Steps,
) -> Result<bool, SshError> {
    use russh::keys::agent::client::AgentClient;
    let agent_error = |error: russh::keys::Error| SshError::Agent(error.to_string());
    match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(mut agent) => agent_try_keys(handle, user, &mut agent, steps).await,
        Err(_) => {
            let mut agent = AgentClient::connect_pageant().await.map_err(agent_error)?;
            agent_try_keys(handle, user, &mut agent, steps).await
        }
    }
}

async fn agent_try_keys<S>(
    handle: &mut Handle<ClientHandler>,
    user: String,
    agent: &mut russh::keys::agent::client::AgentClient<S>,
    steps: &Steps,
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
        let label = Some(key_label(&key));
        steps.emit(ConnectStep::Trying {
            method: AuthKind::Agent,
            label: label.clone(),
        });
        let result = handle
            .authenticate_publickey_with(user.clone(), key, hash, agent)
            .await
            .map_err(|error| SshError::Agent(error.to_string()))?;
        if result.success() {
            return Ok(true);
        }
        steps.emit(ConnectStep::Rejected {
            method: AuthKind::Agent,
            label,
        });
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
