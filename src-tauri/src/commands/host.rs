//! Saved host commands.

use rove_ssh::{HostKeyAnswer, ImportReport, import_ssh_config};
use rove_vault::{Host, HostDraft, HostId, Secret};
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;

/// Lists saved hosts with whether each has a stored secret.
#[tauri::command]
pub fn host_list(state: State<'_, AppState>) -> Result<Vec<HostEntry>, CommandError> {
    let hosts = state.vault.list_hosts()?;
    hosts
        .into_iter()
        .map(|host| {
            let has_secret = state.vault.has_secret(host.id)?;
            Ok(HostEntry { host, has_secret })
        })
        .collect()
}

/// A host listing row.
#[derive(serde::Serialize)]
pub struct HostEntry {
    /// The host.
    #[serde(flatten)]
    pub host: Host,
    /// Whether a secret is stored for it.
    pub has_secret: bool,
}

/// Creates a host, optionally storing `secret`. When storing the secret fails, the new host is
/// removed again so no half-saved host remains.
#[tauri::command]
pub async fn host_create(
    state: State<'_, AppState>,
    draft: HostDraft,
    secret: Option<Secret>,
) -> Result<Host, CommandError> {
    let draft = draft.validate()?;
    if secret.is_some() {
        state.ensure_unlocked().await?;
    }
    let host = state.vault.create_host(draft)?;
    if let Some(secret) = secret
        && let Err(error) = state.vault.set_secret(host.id, &secret)
    {
        if let Err(cleanup) = state.vault.delete_host(host.id) {
            tracing::error!(%cleanup, "removing half-saved host failed");
        }
        return Err(error.into());
    }
    Ok(host)
}

/// Updates a host. `secret` replaces the stored secret; `clear_secret` deletes it.
#[tauri::command]
pub async fn host_update(
    state: State<'_, AppState>,
    id: String,
    draft: HostDraft,
    secret: Option<Secret>,
    clear_secret: bool,
) -> Result<Host, CommandError> {
    let id: HostId = id.parse()?;
    let draft = draft.validate()?;
    if secret.is_some() {
        state.ensure_unlocked().await?;
    }
    let host = state.vault.update_host(id, draft)?;
    if clear_secret {
        state.vault.delete_secret(id)?;
    }
    if let Some(secret) = secret {
        state.vault.set_secret(id, &secret)?;
    }
    Ok(host)
}

/// Deletes a host and its stored secret.
#[tauri::command]
pub fn host_delete(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.vault.delete_host(id.parse()?)?)
}

/// Imports hosts from `~/.ssh/config`.
#[tauri::command]
pub fn host_import_ssh_config(state: State<'_, AppState>) -> Result<ImportReport, CommandError> {
    let home = std::env::home_dir()
        .ok_or_else(|| CommandError::new("app.no_home", "home directory not found"))?;
    let path = home.join(".ssh").join("config");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ImportReport::default());
        }
        Err(error) => {
            return Err(CommandError::new(
                "app.io",
                format!("read {}: {error}", path.display()),
            ));
        }
    };
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".into());
    Ok(import_ssh_config(&state.vault, &text, &home, &user)?)
}

/// Answers a pending host key prompt. Returns `false` when the prompt has expired.
#[tauri::command]
pub fn hostkey_respond(
    state: State<'_, AppState>,
    request_id: String,
    answer: HostKeyAnswer,
) -> bool {
    state.prompts.respond(&request_id, answer)
}
