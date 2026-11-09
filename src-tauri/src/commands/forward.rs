//! Port forwarding rule and runtime commands.

use std::sync::Arc;

use rove_ssh::{ForwardState, ForwardStatus, Forwards};
use rove_vault::{ForwardDraft, ForwardId, ForwardRule, HostId};
use serde::Serialize;
use tauri::State;

use crate::connect::{Source, with_connection};
use crate::error::CommandError;
use crate::state::AppState;

/// A rule with its runtime status.
#[derive(Serialize)]
pub struct ForwardEntry {
    /// The saved rule.
    #[serde(flatten)]
    pub rule: ForwardRule,
    /// Runtime status while running or after a failure.
    pub status: Option<ForwardStatus>,
    /// The local listener is reachable from other machines.
    pub public: bool,
}

fn entry(forwards: &Forwards, rule: ForwardRule) -> ForwardEntry {
    ForwardEntry {
        status: forwards.status(rule.id),
        public: rule.spec.listens_publicly(),
        rule,
    }
}

/// Lists rules of `host_id`, or of every host, with their status.
#[tauri::command]
pub fn forward_list(
    state: State<'_, AppState>,
    host_id: Option<String>,
) -> Result<Vec<ForwardEntry>, CommandError> {
    let host: Option<HostId> = host_id.map(|id| id.parse()).transpose()?;
    Ok(state
        .vault
        .list_forwards(host)?
        .into_iter()
        .map(|rule| entry(&state.forwards, rule))
        .collect())
}

/// Saves a new rule for `host_id`.
#[tauri::command]
pub fn forward_create(
    state: State<'_, AppState>,
    host_id: String,
    draft: ForwardDraft,
) -> Result<ForwardEntry, CommandError> {
    let rule = state.vault.create_forward(host_id.parse()?, draft)?;
    Ok(entry(&state.forwards, rule))
}

/// Replaces a rule. A running forward is stopped, since it no longer matches the rule.
#[tauri::command]
pub fn forward_update(
    state: State<'_, AppState>,
    id: String,
    draft: ForwardDraft,
) -> Result<ForwardEntry, CommandError> {
    let id: ForwardId = id.parse()?;
    let rule = state.vault.update_forward(id, draft)?;
    state.forwards.stop(id);
    Ok(entry(&state.forwards, rule))
}

/// Stops and deletes a rule.
#[tauri::command]
pub fn forward_delete(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let id: ForwardId = id.parse()?;
    state.vault.delete_forward(id)?;
    state.forwards.stop(id);
    Ok(())
}

/// Starts a rule on its host's shared connection, connecting first when needed. Credentials
/// follow the same rules as opening a terminal.
#[tauri::command]
pub async fn forward_start(
    state: State<'_, AppState>,
    id: String,
    password: Option<String>,
    remember: bool,
) -> Result<ForwardStatus, CommandError> {
    let rule = state.vault.forward(id.parse()?)?;
    let source = Source::from_args(
        Some(rule.host_id.to_string()),
        None,
        password,
        None,
        remember,
    )?;
    let forwards = state.forwards.clone();
    with_connection(&state, &source, None, |connection| {
        let forwards = forwards.clone();
        let spec = rule.spec.clone();
        async move {
            forwards
                .start(rule.id, rule.host_id, connection, spec)
                .await
        }
    })
    .await
}

/// Stops a forward. Returns `false` when it was not running.
#[tauri::command]
pub fn forward_stop(state: State<'_, AppState>, id: String) -> Result<bool, CommandError> {
    Ok(state.forwards.stop(id.parse()?))
}

/// Starts the host's auto-start rules on its live connection. Failures are recorded as `failed`
/// statuses for the UI.
pub async fn autostart(state: &AppState, host_id: HostId) {
    let rules = match state.vault.list_forwards(Some(host_id)) {
        Ok(rules) => rules,
        Err(error) => {
            tracing::warn!(%error, "listing forwards for auto-start failed");
            return;
        }
    };
    let pending: Vec<_> = rules
        .into_iter()
        .filter(|rule| rule.auto_start)
        .filter(|rule| {
            state
                .forwards
                .status(rule.id)
                .is_none_or(|status| status.state != ForwardState::Running)
        })
        .collect();
    if pending.is_empty() {
        return;
    }
    let target = match state
        .vault
        .host(host_id)
        .map_err(CommandError::from)
        .and_then(|host| crate::connect::host_target(state, &host))
    {
        Ok(target) => target,
        Err(error) => {
            tracing::warn!(code = error.code, message = %error.message, "loading host for auto-start failed");
            return;
        }
    };
    let Some(connection) = state.ssh.live(&target).await else {
        return;
    };
    for rule in pending {
        let started = state
            .forwards
            .start(rule.id, host_id, Arc::clone(&connection), rule.spec.clone())
            .await;
        if let Err(error) = started {
            tracing::warn!(%error, rule = %rule.id, "auto-start forward failed");
            state
                .forwards
                .record_failure(rule.id, host_id, rule.spec, &error);
        }
    }
}
