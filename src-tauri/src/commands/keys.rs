//! Key library commands: list, generate, import, edit, reveal, delete, and install a public key
//! on a host.

use std::path::Path;
use std::time::Duration;

use rove_ssh::{KeyType, authorize_command, check_public_key, describe_key, generate_key};
use rove_vault::{KeyDraft, KeyId, KeySecret, SshKey};
use serde::Serialize;
use tauri::State;
use zeroize::Zeroizing;

use crate::connect::{Source, with_connection};
use crate::error::CommandError;
use crate::state::AppState;

/// Largest accepted private key text or key file.
const MAX_KEY_TEXT: usize = 64 * 1024;
/// Longest accepted passphrase.
const MAX_PASSPHRASE: usize = 1024;
/// Time allowed for installing a public key.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(20);

fn key_id(id: &str) -> Result<KeyId, CommandError> {
    Ok(id.parse()?)
}

fn passphrase(value: Option<String>) -> Result<Option<Zeroizing<String>>, CommandError> {
    match value.filter(|value| !value.is_empty()) {
        Some(value) if value.len() > MAX_PASSPHRASE => {
            Err(CommandError::new("input.invalid", "passphrase too long"))
        }
        value => Ok(value.map(Zeroizing::new)),
    }
}

/// Checks a user-supplied public key; failures get their own code so they are not mistaken for
/// an unreadable private key.
fn checked_public(public_key: &str, info: &rove_ssh::KeyInfo) -> Result<String, CommandError> {
    check_public_key(public_key, info)
        .map_err(|error| CommandError::new("key.public_invalid", error.to_string()))
}

/// Derives the public facts of `private_key` and builds a draft. An encrypted key needs its
/// passphrase (`ssh.passphrase_required` otherwise). A supplied `public_key` is kept as the
/// user wrote it (for example with their own comment) once it is checked to belong to the
/// private key; without one, the public key derived from the private key is used.
fn draft(
    name: String,
    private_key: Zeroizing<String>,
    passphrase: Option<Zeroizing<String>>,
    public_key: Option<String>,
) -> Result<KeyDraft, CommandError> {
    if private_key.len() > MAX_KEY_TEXT {
        return Err(CommandError::new("input.invalid", "private key too large"));
    }
    let info = describe_key(&private_key, passphrase.as_deref().map(String::as_str))?;
    let public_key = match public_key.filter(|text| !text.trim().is_empty()) {
        Some(text) => checked_public(&text, &info)?,
        None => info.public_key.clone(),
    };
    Ok(KeyDraft {
        name,
        algorithm: info.algorithm,
        public_key,
        fingerprint: info.fingerprint,
        secret: KeySecret {
            private_key,
            passphrase,
        },
    })
}

/// Lists library keys.
#[tauri::command]
pub fn key_list(state: State<'_, AppState>) -> Result<Vec<SshKey>, CommandError> {
    Ok(state.vault.list_ssh_keys()?)
}

/// Generates a key and adds it to the library.
#[tauri::command]
pub async fn key_generate(
    state: State<'_, AppState>,
    name: String,
    kind: KeyType,
    comment: String,
    passphrase: Option<String>,
) -> Result<SshKey, CommandError> {
    if comment.chars().count() > 128 || comment.chars().any(char::is_control) {
        return Err(CommandError::new("input.invalid", "invalid key comment"));
    }
    let passphrase = self::passphrase(passphrase)?;
    state.ensure_unlocked().await?;
    let secret = passphrase.clone();
    // RSA generation takes seconds.
    let private_key = tauri::async_runtime::spawn_blocking(move || {
        generate_key(kind, comment.trim(), secret.as_deref().map(String::as_str))
    })
    .await
    .map_err(|error| CommandError::new("app.task", error.to_string()))??;
    Ok(state
        .vault
        .create_ssh_key(draft(name, private_key, passphrase, None)?)?)
}

/// Adds a pasted or loaded private key to the library, with an optional public key of the
/// user's own.
#[tauri::command]
pub async fn key_import(
    state: State<'_, AppState>,
    name: String,
    private_key: String,
    passphrase: Option<String>,
    public_key: Option<String>,
) -> Result<SshKey, CommandError> {
    let draft = draft(
        name,
        Zeroizing::new(private_key),
        self::passphrase(passphrase)?,
        public_key,
    )?;
    state.ensure_unlocked().await?;
    Ok(state.vault.create_ssh_key(draft)?)
}

/// Reads a private key file chosen by the user so its text can be reviewed before importing.
#[tauri::command]
pub async fn key_read_file(path: String) -> Result<String, CommandError> {
    let path = Path::new(&path);
    if !path.is_absolute() {
        return Err(CommandError::new("input.invalid", "not an absolute path"));
    }
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| CommandError::new("fs.io", error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_KEY_TEXT as u64 {
        return Err(CommandError::new(
            "input.invalid",
            "not a private key file (too large or not a file)",
        ));
    }
    tokio::fs::read_to_string(path)
        .await
        .map_err(|error| CommandError::new("fs.io", error.to_string()))
}

/// Renames a key; with `public_key`, also replaces its public key (checked against the stored
/// private key); with `private_key`, replaces both halves.
#[tauri::command]
pub async fn key_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    private_key: Option<String>,
    passphrase: Option<String>,
    public_key: Option<String>,
) -> Result<SshKey, CommandError> {
    let id = key_id(&id)?;
    match private_key {
        None => {
            let renamed = state.vault.rename_ssh_key(id, &name)?;
            let Some(public_key) = public_key.filter(|text| !text.trim().is_empty()) else {
                return Ok(renamed);
            };
            state.ensure_unlocked().await?;
            let secret = state.vault.ssh_key_secret(id)?;
            let info = describe_key(
                &secret.private_key,
                secret.passphrase.as_deref().map(String::as_str),
            )?;
            let line = checked_public(&public_key, &info)?;
            Ok(state.vault.set_ssh_key_public(id, &line)?)
        }
        Some(private_key) => {
            let draft = draft(
                name,
                Zeroizing::new(private_key),
                self::passphrase(passphrase)?,
                public_key,
            )?;
            state.ensure_unlocked().await?;
            Ok(state.vault.replace_ssh_key(id, draft)?)
        }
    }
}

/// Deletes a key that no host uses (`key.in_use` otherwise).
#[tauri::command]
pub fn key_delete(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    Ok(state.vault.delete_ssh_key(key_id(&id)?)?)
}

/// The private half of a key, shown on request.
#[derive(Serialize)]
pub struct RevealedKey {
    private_key: String,
    passphrase: Option<String>,
}

/// Returns the private key and passphrase so the user can view or copy them.
#[tauri::command]
pub async fn key_reveal(
    state: State<'_, AppState>,
    id: String,
) -> Result<RevealedKey, CommandError> {
    let id = key_id(&id)?;
    state.ensure_unlocked().await?;
    let secret = state.vault.ssh_key_secret(id)?;
    Ok(RevealedKey {
        private_key: secret.private_key.to_string(),
        passphrase: secret.passphrase.map(|value| value.to_string()),
    })
}

/// Adds the key's public half to `~/.ssh/authorized_keys` on a saved host, connecting the same
/// way as opening a terminal.
#[tauri::command]
pub async fn key_install(
    state: State<'_, AppState>,
    id: String,
    host_id: String,
    password: Option<String>,
    password_kind: Option<String>,
) -> Result<(), CommandError> {
    let key = state.vault.ssh_key(key_id(&id)?)?;
    let command = authorize_command(&key.public_key)?;
    let source = Source::from_args(Some(host_id), None, password, password_kind, false)?;
    let output = tokio::time::timeout(
        INSTALL_TIMEOUT,
        with_connection(&state, &source, None, |connection| {
            let command = command.clone();
            async move { connection.exec(&command, 4096).await }
        }),
    )
    .await
    .map_err(|_| CommandError::new("ssh.timeout", "installing the key timed out"))??;
    if output.code == Some(0) {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(CommandError::new(
        "key.install_failed",
        format!("exit {:?}: {}", output.code, stderr.trim()),
    ))
}
