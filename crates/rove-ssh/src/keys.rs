//! Private key loading: `~` expansion, passphrase handling, and the default identity files that
//! OpenSSH tries when no key is configured.

use std::path::{Path, PathBuf};

use russh::keys::PrivateKey;

use crate::SshError;

/// Identity files tried, in order, after the SSH agent; the same defaults as OpenSSH.
const DEFAULT_IDENTITIES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// Expands a leading `~` (`~/` or `~\`) against the user's home directory.
pub(crate) fn expand_home(path: &str) -> PathBuf {
    let rest = path
        .strip_prefix("~/")
        .or_else(|| path.strip_prefix("~\\"))
        .or_else(|| (path == "~").then_some(""));
    match (rest, std::env::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// Decodes key text. An encrypted key without a passphrase, or whose passphrase does not decrypt
/// it, yields [`SshError::PassphraseRequired`]; other failures yield [`SshError::Key`].
pub(crate) fn decode(text: &str, passphrase: Option<&str>) -> Result<PrivateKey, SshError> {
    use russh::keys::{Error, ssh_key};
    match russh::keys::decode_secret_key(text, passphrase) {
        Ok(key) => Ok(key),
        Err(Error::KeyIsEncrypted) => Err(SshError::PassphraseRequired),
        Err(Error::SshKey(ssh_key::Error::Crypto)) if passphrase.is_some() => {
            Err(SshError::PassphraseRequired)
        }
        Err(error) => Err(SshError::Key(error.to_string())),
    }
}

/// Reads and decodes a key file; see [`decode`]. A leading `~` is expanded.
pub(crate) fn load(path: &str, passphrase: Option<&str>) -> Result<PrivateKey, SshError> {
    let path = expand_home(path);
    let text = std::fs::read_to_string(&path)
        .map(zeroize::Zeroizing::new)
        .map_err(|error| SshError::Key(format!("{}: {error}", path.display())))?;
    decode(&text, passphrase)
}

/// Outcome of loading the default identities.
pub(crate) struct DefaultKeys {
    /// Keys that decoded.
    pub keys: Vec<PrivateKey>,
    /// At least one default identity exists but is encrypted and `passphrase` did not open it.
    pub locked: bool,
}

/// Loads the default identity files under `<home>/.ssh` that exist. Unreadable or malformed
/// files are skipped.
pub(crate) fn default_keys(home: Option<&Path>, passphrase: Option<&str>) -> DefaultKeys {
    let mut found = DefaultKeys {
        keys: Vec::new(),
        locked: false,
    };
    let Some(dir) = home.map(|home| home.join(".ssh")) else {
        return found;
    };
    for name in DEFAULT_IDENTITIES {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        match load(&path.to_string_lossy(), passphrase) {
            Ok(key) => found.keys.push(key),
            Err(SshError::PassphraseRequired) => found.locked = true,
            Err(error) => tracing::debug!(%error, path = %path.display(), "skipping identity"),
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::ssh_key::{Cipher, Kdf, LineEnding, private::Ed25519Keypair};

    fn plain() -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]))
    }

    fn encrypted(passphrase: &str) -> String {
        let kdf = Kdf::Bcrypt {
            salt: vec![1; 16],
            rounds: 2,
        };
        plain()
            .encrypt_with(Cipher::Aes256Ctr, kdf, 42, passphrase)
            .unwrap()
            .to_openssh(LineEnding::LF)
            .unwrap()
            .to_string()
    }

    #[test]
    fn encrypted_keys_ask_for_a_passphrase() {
        let text = encrypted("right");
        assert!(matches!(
            decode(&text, None),
            Err(SshError::PassphraseRequired)
        ));
        assert!(matches!(
            decode(&text, Some("wrong")),
            Err(SshError::PassphraseRequired)
        ));
        let key = decode(&text, Some("right")).unwrap();
        assert_eq!(key.public_key(), plain().public_key());
        assert!(matches!(decode("not a key", None), Err(SshError::Key(_))));
    }

    #[test]
    fn default_identities_report_locked_keys() {
        let home = tempfile::tempdir().unwrap();
        let ssh = home.path().join(".ssh");
        std::fs::create_dir(&ssh).unwrap();
        std::fs::write(ssh.join("id_ed25519"), encrypted("pw")).unwrap();
        std::fs::write(ssh.join("id_rsa"), "garbage").unwrap();

        let found = default_keys(Some(home.path()), None);
        assert!(found.keys.is_empty());
        assert!(found.locked);

        let found = default_keys(Some(home.path()), Some("pw"));
        assert_eq!(found.keys.len(), 1);
        assert!(!found.locked);

        let found = default_keys(None, None);
        assert!(found.keys.is_empty() && !found.locked);
    }

    #[test]
    fn tilde_expands_to_home() {
        let Some(home) = std::env::home_dir() else {
            return;
        };
        assert_eq!(expand_home("~/.ssh/id"), home.join(".ssh/id"));
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("/abs/key"), PathBuf::from("/abs/key"));
        assert_eq!(expand_home("~other/key"), PathBuf::from("~other/key"));
    }
}
