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

/// What a key file holds, judged from its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyFileKind {
    /// OpenSSH, PEM or PuTTY private key.
    Private,
    /// One OpenSSH public key line.
    Public,
}

fn classify(text: &str) -> Option<KeyFileKind> {
    let text = text.trim();
    if text.starts_with("-----BEGIN ") || text.starts_with("PuTTY-User-Key-File-") {
        return Some(KeyFileKind::Private);
    }
    let first = text.split_whitespace().next().unwrap_or_default();
    let public_type = first.starts_with("ssh-")
        || first.starts_with("ecdsa-sha2-")
        || first.starts_with("sk-ssh-")
        || first.starts_with("sk-ecdsa-");
    (public_type && text.lines().count() == 1).then_some(KeyFileKind::Public)
}

/// Reads a key file of at most [`MAX_KEY_TEXT`] bytes.
async fn read_key_text(path: &Path) -> Result<String, CommandError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| CommandError::new("fs.io", error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_KEY_TEXT as u64 {
        return Err(CommandError::new(
            "input.invalid",
            "not a key file (too large or not a file)",
        ));
    }
    tokio::fs::read_to_string(path)
        .await
        .map_err(|error| CommandError::new("fs.io", error.to_string()))
}

/// The other half of a key pair, when its file exists and holds that kind of key.
async fn partner(path: &Path, kind: KeyFileKind) -> Option<String> {
    let text = read_key_text(path).await.ok()?;
    (classify(&text) == Some(kind)).then_some(text)
}

/// Both halves found from one chosen file, for review before importing.
#[derive(Debug, Serialize)]
pub struct LoadedKey {
    pub private_key: Option<String>,
    pub public_key: Option<String>,
    /// File name without `.pub`, as a suggested key name.
    pub name: String,
    /// The local key path or the native picker's file name.
    pub path: String,
}

/// Reads a selected key. For local paths, also looks for its companion file.
#[tauri::command]
pub async fn key_load_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<LoadedKey, CommandError> {
    if !crate::mobile_files::is_stage_uri(&path) {
        return load_local_key_file(path).await;
    }
    let staged = state
        .mobile_files
        .stage_temp_async(&app, &path, Some(MAX_KEY_TEXT as u64))
        .await?;
    let mut text = Zeroizing::new(read_key_text(staged.path()).await?);
    let staged_name = staged.name().to_owned();
    drop(staged);
    match classify(&text) {
        Some(KeyFileKind::Private) => Ok(LoadedKey {
            private_key: Some(std::mem::take(&mut *text)),
            public_key: None,
            name: staged_name.clone(),
            path: staged_name,
        }),
        Some(KeyFileKind::Public) => Ok(LoadedKey {
            private_key: None,
            public_key: Some(text.trim().to_owned()),
            name: staged_name
                .strip_suffix(".pub")
                .unwrap_or(&staged_name)
                .to_owned(),
            path: staged_name,
        }),
        None => Err(unrecognized_key()),
    }
}

async fn load_local_key_file(path: String) -> Result<LoadedKey, CommandError> {
    let chosen = Path::new(&path);
    if !chosen.is_absolute() {
        return Err(CommandError::new("input.invalid", "not an absolute path"));
    }
    let text = read_key_text(chosen).await?;
    let file_name = chosen
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    match classify(&text) {
        Some(KeyFileKind::Private) => {
            let public = chosen.with_file_name(format!("{file_name}.pub"));
            Ok(LoadedKey {
                public_key: partner(&public, KeyFileKind::Public)
                    .await
                    .map(|key| key.trim().to_owned()),
                private_key: Some(text),
                name: file_name,
                path,
            })
        }
        Some(KeyFileKind::Public) => {
            let stem = file_name
                .strip_suffix(".pub")
                .unwrap_or(&file_name)
                .to_owned();
            let private_path = chosen.with_file_name(&stem);
            let private_key = if stem == file_name {
                None
            } else {
                partner(&private_path, KeyFileKind::Private).await
            };
            Ok(LoadedKey {
                path: if private_key.is_some() {
                    private_path.to_string_lossy().into_owned()
                } else {
                    path
                },
                private_key,
                public_key: Some(text.trim().to_owned()),
                name: stem,
            })
        }
        None => Err(unrecognized_key()),
    }
}

fn unrecognized_key() -> CommandError {
    CommandError::new(
        "key.unrecognized",
        "the file is neither a private nor a public SSH key",
    )
}

/// Default key files in `~/.ssh` that exist on this device, most preferred first. The app is not
/// sandboxed on desktop, so these are read directly; mobile has no such directory.
#[tauri::command]
pub fn key_default_files(app: tauri::AppHandle) -> Vec<String> {
    #[cfg(desktop)]
    {
        use tauri::Manager;
        const NAMES: [&str; 5] = [
            "id_ed25519",
            "id_ecdsa",
            "id_rsa",
            "id_ed25519_sk",
            "id_ecdsa_sk",
        ];
        let Ok(home) = app.path().home_dir() else {
            return Vec::new();
        };
        NAMES
            .iter()
            .map(|name| home.join(".ssh").join(name))
            .filter(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Vec::new()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_key_files() {
        assert_eq!(
            classify(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbg==\n-----END OPENSSH PRIVATE KEY-----\n"
            ),
            Some(KeyFileKind::Private)
        );
        assert_eq!(
            classify("PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n"),
            Some(KeyFileKind::Private)
        );
        assert_eq!(
            classify("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA me@laptop\n"),
            Some(KeyFileKind::Public)
        );
        assert_eq!(
            classify("ecdsa-sha2-nistp256 AAAAE2VjZHNh"),
            Some(KeyFileKind::Public)
        );
        assert_eq!(classify("ssh-rsa AAAA\nssh-rsa BBBB\n"), None);
        assert_eq!(classify("Host example\n  User root\n"), None);
        assert_eq!(classify(""), None);
    }

    #[tokio::test]
    async fn loads_both_halves_from_either_file() {
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("id_test");
        std::fs::write(
            &private,
            "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n-----END OPENSSH PRIVATE KEY-----\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("id_test.pub"), "ssh-ed25519 AAAA me@host\n").unwrap();

        let from_private = load_local_key_file(private.to_string_lossy().into_owned())
            .await
            .unwrap();
        assert!(from_private.private_key.is_some());
        assert_eq!(
            from_private.public_key.as_deref(),
            Some("ssh-ed25519 AAAA me@host")
        );
        assert_eq!(from_private.name, "id_test");

        let from_public = load_local_key_file(
            dir.path()
                .join("id_test.pub")
                .to_string_lossy()
                .into_owned(),
        )
        .await
        .unwrap();
        assert!(from_public.private_key.is_some());
        assert_eq!(from_public.name, "id_test");
        assert_eq!(from_public.path, private.to_string_lossy());

        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "hello").unwrap();
        let error = load_local_key_file(other.to_string_lossy().into_owned())
            .await
            .unwrap_err();
        assert_eq!(error.code, "key.unrecognized");
        assert!(load_local_key_file("relative/id_rsa".into()).await.is_err());
    }
}
