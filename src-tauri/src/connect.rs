//! Obtains an authenticated SSH connection for a command: reuses the live shared connection when
//! there is one, otherwise resolves credentials (vault secret, typed password, SSH agent), follows
//! the host's route (proxy or jump hosts) and connects.

use std::pin::Pin;
use std::sync::Arc;

use rove_ssh::{AuthSpec, Progress, SshConnection, SshError, SshTarget, Via, parse_quick_connect};
use rove_vault::{AuthMethod, Host, HostId, HostRoute, MAX_JUMP_DEPTH, Secret};
use zeroize::Zeroizing;

use crate::error::CommandError;
use crate::state::AppState;

/// What a value typed for a saved host is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedKind {
    /// The credential the host's method asks for: a password for password hosts, a passphrase
    /// for key hosts.
    Natural,
    /// A login password, whatever the host's method; used to retry after key authentication
    /// failed. Never remembered for hosts that do not use passwords.
    Password,
}

/// A value typed by the user for one attempt.
pub struct Typed {
    pub value: Zeroizing<String>,
    pub kind: TypedKind,
}

/// Where to connect, as given by the WebView.
pub enum Source {
    /// A saved host. `typed` was entered for this attempt; with `remember` it is stored after a
    /// successful login.
    Host {
        host_id: HostId,
        typed: Option<Typed>,
        remember: bool,
    },
    /// A quick-connect string; without `password` the SSH agent is used.
    Quick {
        address: String,
        password: Option<Zeroizing<String>>,
    },
}

impl Source {
    /// Builds a source from command arguments; exactly one of `host_id` and `address` is required.
    /// `password_kind` is `"password"` to log in to a key host with a password; otherwise the
    /// typed value is what the host's method asks for.
    pub fn from_args(
        host_id: Option<String>,
        address: Option<String>,
        password: Option<String>,
        password_kind: Option<String>,
        remember: bool,
    ) -> Result<Self, CommandError> {
        let password = password.map(Zeroizing::new);
        let kind = match password_kind.as_deref() {
            None | Some("natural") => TypedKind::Natural,
            Some("password") => TypedKind::Password,
            Some(other) => {
                return Err(CommandError::new(
                    "input.invalid",
                    format!(
                        "unknown password kind `{}`",
                        other.chars().take(32).collect::<String>()
                    ),
                ));
            }
        };
        match (host_id, address) {
            (Some(host_id), None) => Ok(Self::Host {
                host_id: host_id.parse()?,
                typed: password.map(|value| Typed { value, kind }),
                remember,
            }),
            (None, Some(address)) => Ok(Self::Quick { address, password }),
            _ => Err(CommandError::new(
                "input.invalid",
                "pass exactly one of host_id and address",
            )),
        }
    }
}

fn default_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".into())
}

/// Builds the target of a saved host, with its route identity taken from the jump hosts as they
/// are configured now (see [`rove_ssh::route_key`]). Every saved-host target goes through here so
/// the connection pool and known hosts see the same identity.
pub fn host_target(state: &AppState, host: &Host) -> Result<SshTarget, CommandError> {
    let mut jumps: Vec<Host> = Vec::new();
    let mut next = host.route.clone();
    while let HostRoute::Jump { host: id } = next {
        if jumps.len() >= MAX_JUMP_DEPTH {
            break;
        }
        let jump = state.vault.host(id)?;
        next = jump.route.clone();
        jumps.push(jump);
    }
    Ok(SshTarget::from_host(host, &jumps))
}

/// Builds the target of a quick-connect string.
pub fn quick_target(address: &str) -> Result<SshTarget, CommandError> {
    let quick = parse_quick_connect(address)
        .map_err(|message| CommandError::new("input.invalid", message))?;
    Ok(SshTarget {
        username: quick.username.unwrap_or_else(default_user),
        hostname: quick.hostname,
        port: quick.port.unwrap_or(22),
        host_id: None,
        title: None,
        route: String::new(),
    })
}

fn route_kind(route: &HostRoute) -> &'static str {
    match route {
        HostRoute::Direct => "direct",
        HostRoute::Jump { .. } => "jump",
        HostRoute::Proxy { .. } => "proxy",
        HostRoute::Tailscale { .. } => "tailscale",
    }
}

/// Runs `op` on a connection for `source`, reporting connection steps to `progress`.
///
/// A live shared connection is tried first without credentials; when it can no longer open
/// channels it is evicted and a new connection is made. Fails with `ssh.credentials_required`
/// when a password must be asked for first. A successful saved-host connection updates its
/// last connection time.
pub async fn with_connection<T, F, Fut>(
    state: &AppState,
    source: &Source,
    progress: Option<Progress>,
    op: F,
) -> Result<T, CommandError>
where
    F: Fn(Arc<SshConnection>) -> Fut,
    Fut: Future<Output = Result<T, SshError>> + Send,
{
    match source {
        Source::Host {
            host_id,
            typed,
            remember,
        } => {
            let host = state.vault.host(*host_id)?;
            let target = host_target(state, &host)?;
            let route_name = route_kind(&host.route);
            let started = std::time::Instant::now();
            let value = match try_live(state, &target, &op).await? {
                Some(value) => {
                    tracing::debug!(host_id = %host.id, route = route_name, "reused live ssh connection");
                    value
                }
                None => {
                    tracing::info!(
                        host_id = %host.id,
                        host = %host.hostname,
                        port = host.port,
                        route = route_name,
                        "opening ssh connection"
                    );
                    let auth = match typed {
                        Some(Typed {
                            value,
                            kind: TypedKind::Password,
                        }) => AuthSpec::Password(value.clone()),
                        Some(Typed { value, .. }) => {
                            host_auth(state, &host, Some(value.clone())).await?
                        }
                        None => host_auth(state, &host, None).await?,
                    };
                    let via = match route(state, &host, 0, progress.clone()).await {
                        Ok(via) => via,
                        Err(error) => {
                            tracing::warn!(
                                host_id = %host.id,
                                route = route_name,
                                code = error.code,
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                error = %error.message,
                                "ssh route setup failed"
                            );
                            return Err(error);
                        }
                    };
                    let connection = match state.ssh.shared_via(&target, &auth, &via, progress).await
                    {
                        Ok(connection) => connection,
                        Err(error) => {
                            let cmd_error = CommandError::from(error);
                            tracing::warn!(
                                host_id = %host.id,
                                host = %host.hostname,
                                port = host.port,
                                route = route_name,
                                code = cmd_error.code,
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                error = %cmd_error.message,
                                "ssh connection failed"
                            );
                            return Err(cmd_error);
                        }
                    };
                    tracing::info!(
                        host_id = %host.id,
                        host = %host.hostname,
                        port = host.port,
                        route = route_name,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "ssh connection established"
                    );
                    let value = op(connection).await?;
                    if *remember && let Some(typed) = typed {
                        remember_typed(state, &host, typed).await;
                    }
                    value
                }
            };
            if let Err(error) = state.vault.touch_host(host.id) {
                tracing::warn!(%error, "could not record the connection time");
            }
            Ok(value)
        }
        Source::Quick { address, password } => {
            let target = quick_target(address)?;
            // A typed password is an explicit request to log in with it.
            if password.is_none()
                && let Some(value) = try_live(state, &target, &op).await?
            {
                return Ok(value);
            }
            let started = std::time::Instant::now();
            tracing::info!(host = %target.hostname, port = target.port, "opening quick ssh connection");
            let auth = match password {
                Some(password) => AuthSpec::Password(password.clone()),
                None => AuthSpec::Agent { passphrase: None },
            };
            // A typed value is always sent as a password, so a locked default key falls back to
            // asking for one.
            let connection = match state
                .ssh
                .shared_via(&target, &auth, &Via::Direct, progress)
                .await
            {
                Err(SshError::Agent(_) | SshError::AuthFailed | SshError::PassphraseRequired)
                    if password.is_none() =>
                {
                    return Err(SshError::CredentialsRequired.into());
                }
                Err(error) => {
                    let cmd_error = CommandError::from(error);
                    tracing::warn!(
                        host = %target.hostname,
                        port = target.port,
                        code = cmd_error.code,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        error = %cmd_error.message,
                        "quick ssh connection failed"
                    );
                    return Err(cmd_error);
                }
                Ok(connection) => connection,
            };
            tracing::info!(
                host = %target.hostname,
                port = target.port,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "quick ssh connection established"
            );
            Ok(op(connection).await?)
        }
    }
}

/// Runs `op` on the live shared connection to `target`. Returns `None` when there is none, or when
/// it can no longer open channels (it is then evicted).
async fn try_live<T, F, Fut>(
    state: &AppState,
    target: &SshTarget,
    op: &F,
) -> Result<Option<T>, CommandError>
where
    F: Fn(Arc<SshConnection>) -> Fut,
    Fut: Future<Output = Result<T, SshError>> + Send,
{
    let Some(connection) = state.ssh.live(target).await else {
        return Ok(None);
    };
    match op(connection.clone()).await {
        Ok(value) => Ok(Some(value)),
        Err(SshError::Channel(reason)) => {
            tracing::debug!(%reason, "live ssh connection cannot open channels; reconnecting");
            state.ssh.evict(target, &connection).await;
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

type RouteFuture<'a> = Pin<Box<dyn Future<Output = Result<Via, CommandError>> + Send + 'a>>;

/// Returns how to reach `host`: directly, through its proxy, or through the shared connection to
/// its jump host, which is connected first when needed. `depth` counts the jump hosts already
/// followed.
fn route<'a>(
    state: &'a AppState,
    host: &'a Host,
    depth: usize,
    progress: Option<Progress>,
) -> RouteFuture<'a> {
    Box::pin(async move {
        match &host.route {
            HostRoute::Direct => Ok(Via::Direct),
            #[cfg(desktop)]
            HostRoute::Tailscale { peer_id } => tailnet_route(state, peer_id.clone()).await,
            #[cfg(mobile)]
            HostRoute::Tailscale { .. } => Err(crate::commands::tailscale::unsupported()),
            HostRoute::Proxy {
                protocol,
                host: proxy,
                port,
            } => Ok(Via::Proxy {
                protocol: *protocol,
                host: proxy.clone(),
                port: *port,
            }),
            HostRoute::Jump { host: jump_id } => {
                if depth >= MAX_JUMP_DEPTH {
                    return Err(CommandError::new(
                        "input.invalid",
                        format!("more than {MAX_JUMP_DEPTH} jump hosts"),
                    ));
                }
                let jump = state.vault.host(*jump_id)?;
                let connection = jump_connection(state, &jump, depth + 1, progress)
                    .await
                    .map_err(|error| jump_error(&jump, error))?;
                Ok(Via::Jump(connection))
            }
        }
    })
}

#[cfg(desktop)]
async fn tailnet_route(state: &AppState, peer_id: Option<String>) -> Result<Via, CommandError> {
    use rove_tailscale::{TailnetState, TailscaleError};

    let mut changes = state.tailscale.subscribe();
    let initial = changes.borrow().clone();
    let deadline = state.ssh.timing().connect_timeout;
    let status = tokio::time::timeout(deadline, async {
        if initial.configured && initial.state == TailnetState::Disconnected {
            state.ensure_unlocked().await?;
            if changes.borrow_and_update().state == TailnetState::Disconnected {
                crate::commands::tailscale::auto_connect_state(state).await?;
            }
        }
        loop {
            let state = changes.borrow_and_update().state;
            if state != TailnetState::Starting {
                return Ok::<_, CommandError>(state);
            }
            changes
                .changed()
                .await
                .map_err(|_| TailscaleError::NotConnected)?;
        }
    })
    .await
    .map_err(|_| TailscaleError::NotConnected)??;
    match status {
        TailnetState::Running => Ok(Via::Custom {
            label: "Tailscale".into(),
            dialer: Arc::new(TailnetDialer {
                service: state.tailscale.clone(),
                peer_id,
            }),
        }),
        TailnetState::NeedsLogin | TailnetState::NeedsMachineAuth => {
            Err(TailscaleError::NeedsLogin.into())
        }
        _ => Err(TailscaleError::NotConnected.into()),
    }
}

#[cfg(desktop)]
struct TailnetDialer {
    service: Arc<dyn rove_tailscale::TailnetService>,
    peer_id: Option<String>,
}

#[cfg(desktop)]
#[async_trait::async_trait]
impl rove_ssh::TransportDialer for TailnetDialer {
    async fn dial(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn rove_ssh::SshTransport>, SshError> {
        let started = std::time::Instant::now();
        tracing::info!(
            host,
            port,
            peer_id = ?self.peer_id,
            "dialing ssh tcp over embedded tailscale"
        );
        let stream = self
            .service
            .dial_tcp(host, port, self.peer_id.as_deref())
            .await
            .map_err(|error| {
                tracing::warn!(
                    host,
                    port,
                    peer_id = ?self.peer_id,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    %error,
                    "embedded tailscale tcp dial failed"
                );
                match error {
                    rove_tailscale::TailscaleError::Timeout => SshError::Timeout,
                    other => SshError::Connect(other.to_string()),
                }
            })?;
        tracing::info!(
            host,
            port,
            peer_id = ?self.peer_id,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "embedded tailscale tcp dial connected"
        );
        Ok(Box::new(stream))
    }
}

/// The shared connection to a jump host. Jump hosts authenticate without asking: a stored
/// secret, a key file or the SSH agent.
async fn jump_connection(
    state: &AppState,
    jump: &Host,
    depth: usize,
    progress: Option<Progress>,
) -> Result<Arc<SshConnection>, CommandError> {
    let target = host_target(state, jump)?;
    if let Some(connection) = state.ssh.live(&target).await {
        return Ok(connection);
    }
    let auth = host_auth(state, jump, None).await?;
    let via = route(state, jump, depth, progress.clone()).await?;
    Ok(state.ssh.shared_via(&target, &auth, &via, progress).await?)
}

/// Names the jump host in a failure to reach it; the code becomes `ssh.jump_host`. A failure of
/// a jump host further along keeps its own name.
fn jump_error(jump: &Host, error: CommandError) -> CommandError {
    if error.code == "ssh.jump_host" {
        return error;
    }
    let reason = match error.code {
        "ssh.credentials_required" | "ssh.passphrase_required" => {
            "needs a saved password or an unencrypted key".to_owned()
        }
        _ => error.message,
    };
    CommandError::new(
        "ssh.jump_host",
        format!(
            "{} ({}@{}): {reason}",
            jump.name, jump.username, jump.hostname
        ),
    )
}

/// Resolves credentials for a saved host, unlocking the vault only when a stored secret is needed.
async fn host_auth(
    state: &AppState,
    host: &Host,
    typed: Option<Zeroizing<String>>,
) -> Result<AuthSpec, CommandError> {
    if let AuthMethod::Key { key } = &host.auth {
        state.ensure_unlocked().await?;
        let secret = state.vault.ssh_key_secret(*key)?;
        let stored = Secret::PrivateKey {
            key: secret.private_key,
            passphrase: secret.passphrase,
        };
        return Ok(AuthSpec::resolve(host, Some(stored), typed)?);
    }
    let needs_stored_secret = match &host.auth {
        AuthMethod::Password => typed.is_none(),
        AuthMethod::PrivateKey | AuthMethod::KeyFile { .. } => true,
        AuthMethod::Key { .. } | AuthMethod::Agent | AuthMethod::Interactive => false,
    };
    let stored = if needs_stored_secret && state.vault.has_secret(host.id)? {
        state.ensure_unlocked().await?;
        state.vault.secret(host.id)?
    } else {
        None
    };
    Ok(AuthSpec::resolve(host, stored, typed)?)
}

/// Stores what the user typed for `host` after a successful login: a password for password
/// hosts, a passphrase for key hosts. Agent and interactive hosts store nothing.
async fn remember_typed(state: &AppState, host: &Host, typed: &Typed) {
    let natural_password = matches!(host.auth, AuthMethod::Password | AuthMethod::Interactive);
    if typed.kind == TypedKind::Password && !natural_password {
        return;
    }
    let typed = typed.value.clone();
    let saved = match state.ensure_unlocked().await {
        Ok(()) => remembered_secret(state, host, typed).and_then(|secret| match secret {
            Some(secret) => state
                .vault
                .set_secret(host.id, &secret)
                .map_err(CommandError::from),
            None => Ok(()),
        }),
        Err(error) => Err(error),
    };
    if let Err(error) = saved {
        tracing::warn!(code = error.code, "could not remember the typed credential");
    }
}

fn remembered_secret(
    state: &AppState,
    host: &Host,
    typed: Zeroizing<String>,
) -> Result<Option<Secret>, CommandError> {
    Ok(match &host.auth {
        AuthMethod::Password => Some(Secret::Password { password: typed }),
        AuthMethod::KeyFile { .. } => Some(Secret::Passphrase { passphrase: typed }),
        AuthMethod::PrivateKey => match state.vault.secret(host.id)? {
            Some(Secret::PrivateKey { key, .. }) => Some(Secret::PrivateKey {
                key,
                passphrase: Some(typed),
            }),
            _ => None,
        },
        // A typed passphrase for a library key is not stored; the library holds the key's own.
        AuthMethod::Key { .. } | AuthMethod::Agent | AuthMethod::Interactive => None,
    })
}
