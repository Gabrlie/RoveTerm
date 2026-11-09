//! Obtains an authenticated SSH connection for a command: reuses the live shared connection when
//! there is one, otherwise resolves credentials (vault secret, typed password, SSH agent) and
//! connects.

use std::sync::Arc;

use rove_ssh::{AuthSpec, SshConnection, SshError, SshTarget, parse_quick_connect};
use rove_vault::{AuthMethod, Host, HostId, Secret};
use zeroize::Zeroizing;

use crate::error::CommandError;
use crate::state::AppState;

/// Where to connect, as given by the WebView.
pub enum Source {
    /// A saved host. `password` was typed for this attempt; with `remember` it is stored after a
    /// successful login.
    Host {
        host_id: HostId,
        password: Option<Zeroizing<String>>,
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
    pub fn from_args(
        host_id: Option<String>,
        address: Option<String>,
        password: Option<String>,
        remember: bool,
    ) -> Result<Self, CommandError> {
        let password = password.map(Zeroizing::new);
        match (host_id, address) {
            (Some(host_id), None) => Ok(Self::Host {
                host_id: host_id.parse()?,
                password,
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
    })
}

/// Runs `op` on a connection for `source`.
///
/// A live shared connection is tried first without credentials; when it can no longer open
/// channels it is evicted and a new connection is made. Fails with `ssh.credentials_required`
/// when a password must be asked for first.
pub async fn with_connection<T, F, Fut>(
    state: &AppState,
    source: &Source,
    op: F,
) -> Result<T, CommandError>
where
    F: Fn(Arc<SshConnection>) -> Fut,
    Fut: Future<Output = Result<T, SshError>> + Send,
{
    match source {
        Source::Host {
            host_id,
            password,
            remember,
        } => {
            let host = state.vault.host(*host_id)?;
            let target = SshTarget::from_host(&host);
            if let Some(value) = try_live(state, &target, &op).await? {
                return Ok(value);
            }
            let auth = host_auth(state, &host, password.clone()).await?;
            let connection = state.ssh.shared(&target, &auth).await?;
            let value = op(connection).await?;
            if *remember && let Some(typed) = password {
                remember_typed(state, &host, typed.clone()).await;
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
            let connection = match state.ssh.shared(&target, &auth).await {
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

/// Resolves credentials for a saved host, unlocking the vault only when a stored secret is needed.
async fn host_auth(
    state: &AppState,
    host: &Host,
    typed: Option<Zeroizing<String>>,
) -> Result<AuthSpec, CommandError> {
    let needs_stored_secret = match &host.auth {
        AuthMethod::Password => typed.is_none(),
        AuthMethod::PrivateKey | AuthMethod::KeyFile { .. } => true,
        AuthMethod::Agent | AuthMethod::Interactive => false,
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
async fn remember_typed(state: &AppState, host: &Host, typed: Zeroizing<String>) {
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
        AuthMethod::Agent | AuthMethod::Interactive => None,
    })
}
