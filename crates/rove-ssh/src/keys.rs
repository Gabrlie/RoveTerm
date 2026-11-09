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
    /// Keys that decoded, with their files.
    pub keys: Vec<(PathBuf, PrivateKey)>,
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
            Ok(key) => found.keys.push((path, key)),
            Err(SshError::PassphraseRequired) => found.locked = true,
            Err(error) => tracing::debug!(%error, path = %path.display(), "skipping identity"),
        }
    }
    found
}

/// Algorithm of a generated key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyType {
    /// Ed25519, the default.
    Ed25519,
    /// ECDSA on NIST P-256.
    EcdsaP256,
    /// RSA, 3072 bits, for servers without Ed25519 support.
    Rsa3072,
}

/// Public facts derived from a private key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// Algorithm name, for example `ssh-ed25519`.
    pub algorithm: String,
    /// Public key in OpenSSH single-line form, with the private key's comment.
    pub public_key: String,
    /// `SHA256:` fingerprint.
    pub fingerprint: String,
}

/// Decodes `private_key` and derives its public facts; see [`decode`] for the errors.
pub fn describe(private_key: &str, passphrase: Option<&str>) -> Result<KeyInfo, SshError> {
    let key = decode(private_key, passphrase)?;
    let public = key.public_key();
    Ok(KeyInfo {
        algorithm: public.algorithm().as_str().to_owned(),
        public_key: public
            .to_openssh()
            .map_err(|error| SshError::Key(error.to_string()))?,
        fingerprint: public.fingerprint(russh::keys::HashAlg::Sha256).to_string(),
    })
}

/// Generates a key and returns it in OpenSSH form, encrypted when `passphrase` is given.
pub fn generate(
    kind: KeyType,
    comment: &str,
    passphrase: Option<&str>,
) -> Result<zeroize::Zeroizing<String>, SshError> {
    use russh::keys::ssh_key::{Algorithm, EcdsaCurve, LineEnding};
    let key_error = |error: russh::keys::ssh_key::Error| SshError::Key(error.to_string());
    let mut rng = rand::rng();
    let mut key = match kind {
        KeyType::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519),
        KeyType::EcdsaP256 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        ),
        KeyType::Rsa3072 => {
            let keypair = russh::keys::ssh_key::private::RsaKeypair::random(&mut rng, 3072)
                .map_err(key_error)?;
            Ok(PrivateKey::from(keypair))
        }
    }
    .map_err(key_error)?;
    key.set_comment(comment);
    if let Some(passphrase) = passphrase.filter(|passphrase| !passphrase.is_empty()) {
        key = key.encrypt(&mut rng, passphrase).map_err(key_error)?;
    }
    key.to_openssh(LineEnding::LF)
        .map(|text| zeroize::Zeroizing::new(text.to_string()))
        .map_err(key_error)
}

/// Builds a shell command that adds `public_key` to `~/.ssh/authorized_keys` once, creating the
/// directory and file with private permissions. Only the algorithm and key data are used; the
/// comment is dropped, so nothing from it reaches the shell.
pub fn authorize_command(public_key: &str) -> Result<String, SshError> {
    let parsed = russh::keys::PublicKey::from_openssh(public_key)
        .map_err(|error| SshError::Key(format!("invalid public key: {error}")))?;
    let mut bare = parsed.clone();
    bare.set_comment("");
    let line = bare
        .to_openssh()
        .map_err(|error| SshError::Key(error.to_string()))?;
    let line = line.trim();
    // Algorithm names and base64 only; anything else would need quoting.
    if !line
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '+' | '/' | '=' | '-' | '.' | '@'))
    {
        return Err(SshError::Key("unexpected characters in public key".into()));
    }
    Ok(format!(
        "umask 077 && mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && \
         (grep -qF '{line}' ~/.ssh/authorized_keys || printf '%s\\n' '{line}' >> ~/.ssh/authorized_keys)"
    ))
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
    fn generated_keys_describe_and_open_with_their_passphrase() {
        for kind in [KeyType::Ed25519, KeyType::EcdsaP256] {
            let text = generate(kind, "me@laptop", Some("pw")).unwrap();
            assert!(matches!(
                describe(&text, None),
                Err(SshError::PassphraseRequired)
            ));
            let info = describe(&text, Some("pw")).unwrap();
            assert!(
                info.public_key.ends_with("me@laptop"),
                "{}",
                info.public_key
            );
            assert!(info.fingerprint.starts_with("SHA256:"));
        }
        let plain = generate(KeyType::Ed25519, "", None).unwrap();
        assert_eq!(describe(&plain, None).unwrap().algorithm, "ssh-ed25519");
    }

    #[test]
    fn authorize_command_drops_the_comment() {
        let text = generate(KeyType::Ed25519, "x'; rm -rf ~ #", None).unwrap();
        let public = describe(&text, None).unwrap().public_key;
        let command = authorize_command(&public).unwrap();
        assert!(!command.contains("rm -rf"), "{command}");
        assert!(command.contains("ssh-ed25519 AAAA"));
        assert!(authorize_command("ssh-ed25519 not-base64'").is_err());
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
