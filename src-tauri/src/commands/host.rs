//! Saved host commands.

use std::path::{Path, PathBuf};

use rove_ssh::{HostKeyAnswer, ImportReport, import_ssh_config};
use rove_vault::{Host, HostDraft, HostId, Secret};
use tauri::{AppHandle, Manager, State};
use tokio::io::AsyncReadExt;

use crate::error::CommandError;
use crate::mobile_files::{absolute_local_path, is_stage_uri};
use crate::state::AppState;

const MAX_SSH_CONFIG_BYTES: u64 = 2 * 1024 * 1024;

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

/// The notes kept about host `id` (empty when there are none).
#[tauri::command]
pub fn host_notes_get(state: State<'_, AppState>, id: String) -> Result<String, CommandError> {
    Ok(state.vault.host_notes(id.parse()?)?)
}

/// Replaces the notes kept about host `id`; empty text removes them.
#[tauri::command]
pub fn host_notes_set(
    state: State<'_, AppState>,
    id: String,
    text: String,
) -> Result<(), CommandError> {
    Ok(state.vault.set_host_notes(id.parse()?, &text)?)
}

/// Deletes a host and its stored secret.
#[tauri::command]
pub fn host_delete(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.vault.delete_host(id.parse()?)?)
}

/// Imports hosts from a selected SSH config or the default `~/.ssh/config`.
#[tauri::command]
pub async fn host_import_ssh_config(
    app: AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<ImportReport, CommandError> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".into());
    if let Some(raw) = path {
        let paths = app.path();
        let base_home = paths
            .home_dir()
            .ok()
            .or_else(|| paths.app_data_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"));
        let text = if is_stage_uri(&raw) {
            let staged = state
                .mobile_files
                .stage_temp_async(&app, &raw, Some(MAX_SSH_CONFIG_BYTES))
                .await?;
            let text = read_ssh_config_file(staged.path()).await?;
            drop(staged);
            text
        } else {
            read_explicit_ssh_config(&raw).await?
        };
        return Ok(import_ssh_config(&state.vault, &text, &base_home, &user)?);
    }

    let home = std::env::home_dir()
        .ok_or_else(|| CommandError::new("app.no_home", "home directory not found"))?;
    let path = home.join(".ssh").join("config");
    let text = match tokio::fs::read_to_string(&path).await {
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
    Ok(import_ssh_config(&state.vault, &text, &home, &user)?)
}

async fn read_explicit_ssh_config(raw: &str) -> Result<String, CommandError> {
    read_ssh_config_file(absolute_local_path(raw)?).await
}

async fn read_ssh_config_file(path: &Path) -> Result<String, CommandError> {
    let io_error = |error| CommandError::new("fs.io", format!("read {}: {error}", path.display()));
    let metadata = tokio::fs::metadata(path).await.map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_SSH_CONFIG_BYTES {
        return Err(CommandError::new(
            "input.invalid",
            "SSH config must be a regular file of at most 2 MiB",
        ));
    }
    let file = tokio::fs::File::open(path).await.map_err(io_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_SSH_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_SSH_CONFIG_BYTES {
        return Err(CommandError::new(
            "input.invalid",
            "SSH config exceeds 2 MiB",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| CommandError::new("input.invalid", "SSH config must be UTF-8"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_an_explicit_ssh_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let text = "Host example\n  HostName example.com\n  User alice\n";
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            read_explicit_ssh_config(path.to_str().unwrap())
                .await
                .unwrap(),
            text
        );
    }

    #[tokio::test]
    async fn rejects_explicit_configs_that_are_too_large_or_have_invalid_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oversized");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_SSH_CONFIG_BYTES + 1)
            .unwrap();
        for invalid in [
            path.to_str().unwrap(),
            "relative/config",
            dir.path().to_str().unwrap(),
        ] {
            let error = read_explicit_ssh_config(invalid).await.unwrap_err();
            assert_eq!(error.code, "input.invalid");
        }
    }

    #[tokio::test]
    async fn rejects_explicit_configs_that_are_not_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, [0xff]).unwrap();
        let error = read_explicit_ssh_config(path.to_str().unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.code, "input.invalid");
    }
}
