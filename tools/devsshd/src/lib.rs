//! Development SSH server: password and public-key authentication, PTY shells backed by
//! `rove-pty`, `exec` requests run through `sh -c`, an SFTP subsystem on the real file system, and
//! local (`direct-tcpip`) and remote (`tcpip-forward`) port forwarding.
//!
//! Intended for integration tests and local debugging only. It runs commands as the current user
//! and must never listen on a public interface. Remote forwards always bind to loopback.

mod sftp;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rove_pty::{LocalRequest, LocalTerminals, PtyConfig};
use rove_session::{TermSize, TerminalOutput, TerminalSession};
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{
    Auth, ChannelOpenHandle, Handle, Msg, Server as _, Session, VerifiedPublicKey,
};
use russh::{Channel, ChannelId, Pty};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::AbortHandle;

/// Server settings.
pub struct DevConfig {
    /// Accepted user name.
    pub user: String,
    /// Accepted password; `None` disables password authentication.
    pub password: Option<String>,
    /// Accepted public keys.
    pub authorized_keys: Vec<PublicKey>,
    /// Server host key.
    pub host_key: PrivateKey,
    /// Shell for PTY sessions; `None` uses the platform default.
    pub shell: Option<String>,
    /// Directory relative SFTP paths resolve against; `None` uses the current directory.
    pub sftp_root: Option<PathBuf>,
    /// Simulates an SSH server that ignores exec channel signals.
    pub ignore_exec_signals: bool,
}

impl DevConfig {
    /// `demo` / `demo` with a freshly generated Ed25519 host key.
    pub fn demo() -> Self {
        Self {
            user: "demo".into(),
            password: Some("demo".into()),
            authorized_keys: Vec::new(),
            host_key: generate_key(),
            shell: Some("/bin/sh".into()),
            sftp_root: None,
            ignore_exec_signals: false,
        }
    }
}

/// Generates an Ed25519 key.
pub fn generate_key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
        .expect("ed25519 key generation")
}

/// A running server. Dropping it shuts the listener down.
pub struct DevServer {
    addr: SocketAddr,
    connections: Arc<AtomicUsize>,
    shutdown: russh::server::RunningServerHandle,
}

impl DevServer {
    /// Binds `bind` (use port 0 for an ephemeral port) and serves in a background task.
    pub async fn start(bind: &str, config: DevConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let connections = Arc::new(AtomicUsize::new(0));
        let russh_config = Arc::new(russh::server::Config {
            keys: vec![config.host_key.clone()],
            auth_rejection_time: std::time::Duration::from_millis(50),
            auth_rejection_time_initial: Some(std::time::Duration::ZERO),
            inactivity_timeout: Some(std::time::Duration::from_secs(3600)),
            ..Default::default()
        });
        let mut server = Server {
            config: Arc::new(config),
            terminals: Arc::new(LocalTerminals::new(PtyConfig::default())),
            connections: connections.clone(),
        };
        let (handle_tx, handle_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let running = server.run_on_socket(russh_config, &listener);
            let _receiver_waits_for_this = handle_tx.send(running.handle());
            if let Err(error) = running.await {
                tracing::error!(%error, "devsshd stopped");
            }
        });
        let shutdown = handle_rx
            .await
            .map_err(|_| std::io::Error::other("devsshd task ended before starting"))?;
        Ok(Self {
            addr,
            connections,
            shutdown,
        })
    }

    /// Returns the bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns the number of TCP connections accepted so far.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for DevServer {
    fn drop(&mut self) {
        self.shutdown.shutdown("devsshd shutting down".into());
    }
}

#[derive(Clone)]
struct Server {
    config: Arc<DevConfig>,
    terminals: Arc<LocalTerminals>,
    connections: Arc<AtomicUsize>,
}

impl russh::server::Server for Server {
    type Handler = Client;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> Client {
        self.connections.fetch_add(1, Ordering::SeqCst);
        Client {
            config: self.config.clone(),
            terminals: self.terminals.clone(),
            channels: HashMap::new(),
            unclaimed: HashMap::new(),
            forwards: HashMap::new(),
            exec_signals: HashMap::new(),
        }
    }
}

#[derive(Default)]
struct ChannelState {
    size: TermSize,
    session: Option<Arc<dyn TerminalSession>>,
}

struct Client {
    config: Arc<DevConfig>,
    terminals: Arc<LocalTerminals>,
    channels: HashMap<ChannelId, ChannelState>,
    /// Session channels kept until a request decides their use; only `sftp` consumes them.
    unclaimed: HashMap<ChannelId, Channel<Msg>>,
    exec_signals: HashMap<ChannelId, tokio::sync::mpsc::UnboundedSender<russh::Sig>>,
    /// Remote forward listeners by `(address, port)` as requested by the client.
    forwards: HashMap<(String, u32), AbortHandle>,
}

impl Drop for Client {
    fn drop(&mut self) {
        for forward in self.forwards.values() {
            forward.abort();
        }
        for state in self.channels.values_mut() {
            if let Some(session) = state.session.take() {
                tokio::spawn(async move {
                    let _already_closed_is_fine = session.close().await;
                });
            }
        }
    }
}

fn clamp_size(cols: u32, rows: u32) -> TermSize {
    let clamp = |value: u32| {
        u16::try_from(value.clamp(1, u32::from(TermSize::MAX))).unwrap_or(TermSize::MAX)
    };
    TermSize::new(clamp(cols), clamp(rows)).unwrap_or_default()
}

async fn finish(handle: &Handle, channel: ChannelId, code: u32) {
    let _ = handle.exit_status_request(channel, code).await;
    let _ = handle.eof(channel).await;
    let _ = handle.close(channel).await;
}

impl russh::server::Handler for Client {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let accepted =
            user == self.config.user && self.config.password.as_deref() == Some(password);
        Ok(if accepted {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let accepted = user == self.config.user
            && self
                .config
                .authorized_keys
                .iter()
                .any(|authorized| authorized.key_data() == key.public_key.key_data());
        Ok(if accepted {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), ChannelState::default());
        self.unclaimed.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Ok(port) = u16::try_from(port_to_connect) else {
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        match TcpStream::connect((host_to_connect, port)).await {
            Ok(stream) => {
                reply.accept().await;
                tokio::spawn(bridge(channel, stream));
            }
            Err(error) => {
                tracing::debug!(%error, "devsshd direct-tcpip connect failed");
                reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            }
        }
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let Ok(requested) = u16::try_from(*port) else {
            return Ok(false);
        };
        let listener = match TcpListener::bind(("127.0.0.1", requested)).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::debug!(%error, "devsshd tcpip-forward bind failed");
                return Ok(false);
            }
        };
        let bound = match listener.local_addr() {
            Ok(addr) => u32::from(addr.port()),
            Err(_) => return Ok(false),
        };
        let handle = session.handle();
        let connected_address = address.to_owned();
        let task = tokio::spawn(async move {
            while let Ok((stream, peer)) = listener.accept().await {
                let channel = handle
                    .channel_open_forwarded_tcpip(
                        connected_address.clone(),
                        bound,
                        peer.ip().to_string(),
                        u32::from(peer.port()),
                    )
                    .await;
                match channel {
                    Ok(channel) => {
                        tokio::spawn(bridge(channel, stream));
                    }
                    Err(error) => tracing::debug!(%error, "devsshd forwarded-tcpip open failed"),
                }
            }
        });
        self.forwards
            .insert((address.to_owned(), bound), task.abort_handle());
        *port = bound;
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(match self.forwards.remove(&(address.to_owned(), port)) {
            Some(task) => {
                task.abort();
                true
            }
            None => false,
        })
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        match (name, self.unclaimed.remove(&channel)) {
            ("sftp", Some(stream)) => {
                let root = match &self.config.sftp_root {
                    Some(root) => root.clone(),
                    None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                };
                session.channel_success(channel)?;
                russh_sftp::server::run(stream.into_stream(), sftp::SftpHandler::new(root)).await;
                Ok(())
            }
            _ => session.channel_failure(channel),
        }
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.unclaimed.remove(&channel);
        self.channels.entry(channel).or_default().size = clamp_size(cols, rows);
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.unclaimed.remove(&channel);
        let size = self.channels.entry(channel).or_default().size;
        let request = LocalRequest {
            shell: self.config.shell.clone(),
            cwd: None,
            size: Some(size),
        };
        let mut opened = match self.terminals.open(request) {
            Ok(opened) => opened,
            Err(error) => {
                tracing::warn!(%error, "devsshd could not start a shell");
                return session.channel_failure(channel);
            }
        };
        self.channels.entry(channel).or_default().session = Some(opened.session.clone());
        let handle = session.handle();
        tokio::spawn(async move {
            while let Some(output) = opened.output.recv().await {
                match output {
                    TerminalOutput::Data(data) => {
                        if handle.data(channel, data).await.is_err() {
                            return;
                        }
                    }
                    TerminalOutput::Exit(status) => {
                        let code = status
                            .code
                            .and_then(|code| u32::try_from(code).ok())
                            .unwrap_or(1);
                        finish(&handle, channel, code).await;
                        return;
                    }
                }
            }
        });
        session.channel_success(channel)
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.unclaimed.remove(&channel);
        let command = String::from_utf8_lossy(data).into_owned();
        let handle = session.handle();
        let (signals, mut pending) = tokio::sync::mpsc::unbounded_channel();
        self.exec_signals.insert(channel, signals);
        tokio::spawn(async move {
            let mut process = tokio::process::Command::new("sh");
            process
                .arg("-c")
                .arg(command)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            #[cfg(unix)]
            process.process_group(0);
            let output = match process.spawn() {
                Ok(child) => {
                    #[cfg(unix)]
                    let group = child.id();
                    let wait = child.wait_with_output();
                    tokio::pin!(wait);
                    loop {
                        tokio::select! {
                            output = &mut wait => break output,
                            signal = pending.recv() => {
                                #[cfg(unix)]
                                if let Some(group) = group {
                                    let signal = match signal { Some(russh::Sig::TERM) => "-TERM", Some(russh::Sig::KILL) | None => "-KILL", _ => continue };
                                    let _ = tokio::process::Command::new("/bin/kill").args([signal, "--", &format!("-{group}")]).status().await;
                                }
                                if signal.is_none() { break wait.await; }
                            }
                        }
                    }
                }
                Err(error) => Err(error),
            };
            let code = match output {
                Ok(output) => {
                    let _ = handle.data(channel, output.stdout).await;
                    if !output.stderr.is_empty() {
                        let _ = handle.extended_data(channel, 1, output.stderr).await;
                    }
                    output
                        .status
                        .code()
                        .and_then(|code| u32::try_from(code).ok())
                        .unwrap_or(1)
                }
                Err(error) => {
                    let _ = handle.data(channel, format!("devsshd: {error}\n")).await;
                    127
                }
            };
            finish(&handle, channel, code).await;
        });
        session.channel_success(channel)
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(terminal) = self
            .channels
            .get(&channel)
            .and_then(|state| state.session.clone())
        {
            let _ended_shell_ignores_input = terminal.write(data).await;
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let size = clamp_size(cols, rows);
        let state = self.channels.entry(channel).or_default();
        state.size = size;
        if let Some(terminal) = state.session.clone() {
            let _ended_shell_ignores_resize = terminal.resize(size).await;
        }
        Ok(())
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: russh::Sig,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if !self.config.ignore_exec_signals
            && let Some(sender) = self.exec_signals.get(&channel)
        {
            let _ = sender.send(signal);
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.unclaimed.remove(&channel);
        self.exec_signals.remove(&channel);
        if let Some(terminal) = self
            .channels
            .remove(&channel)
            .and_then(|state| state.session)
        {
            let _already_closed_is_fine = terminal.close().await;
        }
        Ok(())
    }
}

/// Copies bytes both ways between a forwarding channel and a TCP stream until either side closes.
async fn bridge(channel: Channel<Msg>, mut stream: TcpStream) {
    let mut channel = channel.into_stream();
    if let Err(error) = tokio::io::copy_bidirectional(&mut channel, &mut stream).await {
        tracing::debug!(%error, "devsshd forwarding stream ended");
    }
}
