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
            let target = SshTarget::from_host(&host);
            let value = match try_live(state, &target, &op).await? {
                Some(value) => value,
                None => {
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
                    let via = route(state, &host, 0, progress.clone()).await?;
                    let connection = state.ssh.shared_via(&target, &auth, &via, progress).await?;
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
                result => result?,
            };
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

/// The shared connection to a jump host. Jump hosts authenticate without asking: a stored
/// secret, a key file or the SSH agent.
async fn jump_connection(
    state: &AppState,
    jump: &Host,
    depth: usize,
    progress: Option<Progress>,
) -> Result<Arc<SshConnection>, CommandError> {
    let target = SshTarget::from_host(jump);
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
