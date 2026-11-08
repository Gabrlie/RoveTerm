//! Terminal session commands.

use std::sync::Arc;

use rove_session::{SessionId, SessionSummary, TermSize};
use rove_ssh::{AuthSpec, SshError, SshTarget, parse_quick_connect};
use rove_vault::{AuthMethod, HostId, Secret};
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody, Response};
use zeroize::Zeroizing;

use crate::error::CommandError;
use crate::sink::ChannelSink;
use crate::state::AppState;

/// Largest accepted single input write.
const MAX_WRITE: usize = 1024 * 1024;

fn session_id(id: &str) -> Result<SessionId, CommandError> {
    Ok(id.parse()?)
}

/// Opens a local shell and streams its output to `on_output`. Returns the session id.
#[tauri::command]
pub async fn session_open_local(
    state: State<'_, AppState>,
    on_output: Channel<InvokeResponseBody>,
    cols: u16,
    rows: u16,
) -> Result<SessionId, CommandError> {
    let size = TermSize::new(cols, rows)?;
    #[cfg(desktop)]
    {
        let opened = state.pty.open(rove_pty::LocalRequest {
            size: Some(size),
            ..Default::default()
        })?;
        Ok(state
            .sessions
            .attach(opened, Arc::new(ChannelSink(on_output))))
    }
    #[cfg(mobile)]
    {
        let _ = (state, on_output, size);
        Err(CommandError::new(
            "session.unsupported",
            "local terminals are not available on this platform",
        ))
    }
}

/// Opens a shell on a saved host.
///
/// `password` is a password typed by the user for this attempt; with `remember`, it is stored in
/// the vault after a successful login. Fails with `ssh.credentials_required` when the host needs a
/// password and none is available.
#[tauri::command]
pub async fn session_open_ssh(
    state: State<'_, AppState>,
    on_output: Channel<InvokeResponseBody>,
    host_id: String,
    password: Option<String>,
    remember: bool,
    cols: u16,
    rows: u16,
) -> Result<SessionId, CommandError> {
    let size = TermSize::new(cols, rows)?;
    let host_id: HostId = host_id.parse()?;
    let host = state.vault.host(host_id)?;
    let target = SshTarget::from_host(&host);
    if let Some(opened) = state.ssh.open_live(&target, size).await? {
        return Ok(state
            .sessions
            .attach(opened, Arc::new(ChannelSink(on_output))));
    }
    let typed = password.map(Zeroizing::new);
    let needs_stored_secret = match &host.auth {
        AuthMethod::Password => typed.is_none(),
        AuthMethod::PrivateKey | AuthMethod::KeyFile { .. } => true,
        AuthMethod::Agent | AuthMethod::Interactive => false,
    };
    let stored = if needs_stored_secret && state.vault.has_secret(host_id)? {
        state.ensure_unlocked().await?;
        state.vault.secret(host_id)?
    } else {
        None
    };
    let auth = AuthSpec::resolve(&host, stored, typed.clone())?;
    let opened = state.ssh.open_terminal(&target, &auth, size).await?;
    if remember
        && let Some(password) = typed
        && matches!(host.auth, AuthMethod::Password)
    {
        let saved = match state.ensure_unlocked().await {
            Ok(()) => state
                .vault
                .set_secret(host_id, &Secret::Password { password })
                .map_err(CommandError::from),
            Err(error) => Err(error),
        };
        if let Err(error) = saved {
            tracing::warn!(code = error.code, "could not remember password");
        }
    }
    Ok(state
        .sessions
        .attach(opened, Arc::new(ChannelSink(on_output))))
}

/// Opens a shell from a quick-connect string. Uses `password` when given, otherwise the SSH agent.
#[tauri::command]
pub async fn session_open_quick(
    state: State<'_, AppState>,
    on_output: Channel<InvokeResponseBody>,
    address: String,
    password: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<SessionId, CommandError> {
    let size = TermSize::new(cols, rows)?;
    let quick = parse_quick_connect(&address)
        .map_err(|message| CommandError::new("input.invalid", message))?;
    let target = SshTarget {
        username: quick.username.unwrap_or_else(default_user),
        hostname: quick.hostname,
        port: quick.port.unwrap_or(22),
        host_id: None,
        title: None,
    };
    if password.is_none()
        && let Some(opened) = state.ssh.open_live(&target, size).await?
    {
        return Ok(state
            .sessions
            .attach(opened, Arc::new(ChannelSink(on_output))));
    }
    let auth = match password {
        Some(password) => AuthSpec::Password(Zeroizing::new(password)),
        None => AuthSpec::Agent,
    };
    let opened = match state.ssh.open_terminal(&target, &auth, size).await {
        Err(SshError::Agent(_) | SshError::AuthFailed) if matches!(auth, AuthSpec::Agent) => {
            return Err(SshError::CredentialsRequired.into());
        }
        result => result?,
    };
    Ok(state
        .sessions
        .attach(opened, Arc::new(ChannelSink(on_output))))
}

fn default_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".into())
}

/// Sends input to a session.
#[tauri::command]
pub async fn session_write(
    state: State<'_, AppState>,
    id: String,
    data: String,
) -> Result<(), CommandError> {
    if data.len() > MAX_WRITE {
        return Err(CommandError::new("input.invalid", "input too large"));
    }
    Ok(state
        .sessions
        .write(session_id(&id)?, data.as_bytes())
        .await?)
}

/// Resizes a session.
#[tauri::command]
pub async fn session_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), CommandError> {
    Ok(state
        .sessions
        .resize(session_id(&id)?, TermSize::new(cols, rows)?)
        .await?)
}

/// Closes a session.
#[tauri::command]
pub async fn session_close(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.sessions.close(session_id(&id)?).await?)
}

/// Re-attaches a view: returns the replay buffer as an `ArrayBuffer` and streams subsequent output
/// to `on_output`. An ended session's exit message follows immediately.
#[tauri::command]
pub async fn session_reattach(
    state: State<'_, AppState>,
    id: String,
    on_output: Channel<InvokeResponseBody>,
) -> Result<Response, CommandError> {
    let sink = Arc::new(ChannelSink(on_output));
    let (replay, exited) = state.sessions.reattach(session_id(&id)?, sink.clone())?;
    if let Some(status) = exited {
        rove_session::OutputSink::send(sink.as_ref(), rove_session::OutputChunk::Exit(status));
    }
    Ok(Response::new(replay.to_vec()))
}

/// Lists live sessions.
#[tauri::command]
pub fn session_list(state: State<'_, AppState>) -> Vec<SessionSummary> {
    state.sessions.list()
}
