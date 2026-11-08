//! Development SSH server: password and public-key authentication, PTY shells backed by
//! `rove-pty`, and `exec` requests run through `sh -c`.
//!
//! Intended for integration tests and local debugging only. It runs commands as the current user
//! and must never listen on a public interface.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rove_pty::{LocalRequest, LocalTerminals, PtyConfig};
use rove_session::{TermSize, TerminalOutput, TerminalSession};
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{
    Auth, ChannelOpenHandle, Handle, Msg, Server as _, Session, VerifiedPublicKey,
};
use russh::{Channel, ChannelId, Pty};
use tokio::net::TcpListener;

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
}

impl Drop for Client {
    fn drop(&mut self) {
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
        reply.accept().await;
        Ok(())
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
        self.channels.entry(channel).or_default().size = clamp_size(cols, rows);
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
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
        let command = String::from_utf8_lossy(data).into_owned();
        let handle = session.handle();
        tokio::spawn(async move {
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .output()
                .await;
            let code = match output {
                Ok(output) => {
                    let mut combined = output.stdout;
                    combined.extend_from_slice(&output.stderr);
                    let _ = handle.data(channel, combined).await;
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

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
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
