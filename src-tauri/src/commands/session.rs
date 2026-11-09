//! Terminal session commands.

use std::sync::Arc;

use rove_session::{SessionId, SessionSummary, TermSize};
use tauri::State;
use tauri::ipc::{Channel, InvokeResponseBody, Response};

use crate::connect::{Source, with_connection};
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
    let source = Source::from_args(Some(host_id), None, password, remember)?;
    let opened = with_connection(&state, &source, |connection| async move {
        connection.open_shell(size).await
    })
    .await?;
    let id = state
        .sessions
        .attach(opened, Arc::new(ChannelSink(on_output)));
    if let Source::Host { host_id, .. } = source {
        crate::commands::forward::autostart(&state, host_id).await;
    }
    Ok(id)
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
    let source = Source::from_args(None, Some(address), password, false)?;
    let opened = with_connection(&state, &source, |connection| async move {
        connection.open_shell(size).await
    })
    .await?;
    Ok(state
        .sessions
        .attach(opened, Arc::new(ChannelSink(on_output))))
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
