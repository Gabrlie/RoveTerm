//! Host, secret and known-host records.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::VaultError;

/// Opaque identifier of a saved host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HostId(Uuid);

impl HostId {
    /// Generates a new time-ordered identifier.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for HostId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for HostId {
    type Err = VaultError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self).map_err(|_| {
            VaultError::Invalid(format!(
                "invalid host id `{}`",
                s.chars().take(64).collect::<String>()
            ))
        })
    }
}

/// How a host authenticates. Secrets for `Password` and `PrivateKey` live in the encrypted secret
/// store; the others need none, except an optional passphrase for `KeyFile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Password stored in the vault, or asked for when none is stored.
    Password,
    /// Private key stored in the vault.
    PrivateKey,
    /// Private key read from a file on this device.
    KeyFile {
        /// Path to the private key file.
        path: String,
    },
    /// Keys offered by the running SSH agent.
    Agent,
    /// Ask for a password or keyboard-interactive responses at every connection.
    Interactive,
}

/// A saved host as stored and listed. Contains no secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// Identifier.
    pub id: HostId,
    /// Display name.
    pub name: String,
    /// Optional group for the sidebar.
    pub group: Option<String>,
    /// DNS name or IP address.
    pub hostname: String,
    /// TCP port.
    pub port: u16,
    /// Login user.
    pub username: String,
    /// Authentication method.
    pub auth: AuthMethod,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
    /// Last modification time, Unix milliseconds.
    pub updated_at: i64,
}

/// Input for creating or updating a host. Validated by [`HostDraft::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostDraft {
    /// Display name; empty falls back to `hostname`.
    #[serde(default)]
    pub name: String,
    /// Optional group.
    #[serde(default)]
    pub group: Option<String>,
    /// DNS name or IP address.
    pub hostname: String,
    /// TCP port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Login user.
    pub username: String,
    /// Authentication method.
    pub auth: AuthMethod,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_port() -> u16 {
    22
}

impl HostDraft {
    /// Trims text fields and checks lengths and characters. Returns the normalized draft.
    pub fn validate(mut self) -> Result<Self, VaultError> {
        self.hostname = self.hostname.trim().to_owned();
        self.username = self.username.trim().to_owned();
        self.name = self.name.trim().to_owned();
        self.group = self
            .group
            .map(|group| group.trim().to_owned())
            .filter(|group| !group.is_empty());
        self.tags = self
            .tags
            .into_iter()
            .map(|tag| tag.trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .collect();
        if self.hostname.is_empty()
            || self.hostname.len() > 255
            || self.hostname.chars().any(char::is_whitespace)
        {
            return Err(VaultError::Invalid(
                "hostname must be 1-255 characters without spaces".into(),
            ));
        }
        if self.username.is_empty()
            || self.username.len() > 64
            || self.username.chars().any(char::is_whitespace)
        {
            return Err(VaultError::Invalid(
                "username must be 1-64 characters without spaces".into(),
            ));
        }
        if self.port == 0 {
            return Err(VaultError::Invalid("port must be 1-65535".into()));
        }
        if self.name.is_empty() {
            self.name = self.hostname.clone();
        }
        if self.name.chars().count() > 128
            || self
                .group
                .as_ref()
                .is_some_and(|group| group.chars().count() > 64)
        {
            return Err(VaultError::Invalid("name or group too long".into()));
        }
        if self.tags.len() > 32 || self.tags.iter().any(|tag| tag.chars().count() > 32) {
            return Err(VaultError::Invalid(
                "at most 32 tags of up to 32 characters".into(),
            ));
        }
        if let AuthMethod::KeyFile { path } = &self.auth
            && path.trim().is_empty()
        {
            return Err(VaultError::Invalid(
                "key file path must not be empty".into(),
            ));
        }
        Ok(self)
    }
}

/// A host's stored secret. Zeroized on drop; `Debug` never prints contents.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Secret {
    /// Login password.
    Password {
        /// The password.
        password: Zeroizing<String>,
    },
    /// Private key in OpenSSH or PEM text form.
    PrivateKey {
        /// Key text.
        key: Zeroizing<String>,
        /// Passphrase protecting the key, if any.
        passphrase: Option<Zeroizing<String>>,
    },
    /// Passphrase for an [`AuthMethod::KeyFile`] key.
    Passphrase {
        /// The passphrase.
        passphrase: Zeroizing<String>,
    },
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Password { .. } => "Password",
            Self::PrivateKey { .. } => "PrivateKey",
            Self::Passphrase { .. } => "Passphrase",
        };
        write!(f, "Secret::{kind}(..)")
    }
}

/// Result of comparing a server's host key with the known-hosts table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KnownHostStatus {
    /// The presented key matches the trusted key.
    Trusted,
    /// No key is recorded for this host and port.
    Unknown,
    /// A different key is recorded: possible man-in-the-middle attack.
    Mismatch {
        /// Algorithm of the recorded key.
        expected_algorithm: String,
        /// SHA-256 fingerprint of the recorded key.
        expected_fingerprint: String,
    },
}

/// A server public key presented during the SSH handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostKey {
    /// Key algorithm, for example `ssh-ed25519`.
    pub algorithm: String,
    /// Public key in OpenSSH single-line form.
    pub public_key: String,
    /// `SHA256:` fingerprint for display.
    pub fingerprint: String,
}

/// Current Unix time in milliseconds.
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> HostDraft {
        HostDraft {
            name: " ".into(),
            group: Some("  ".into()),
            hostname: " example.com ".into(),
            port: 22,
            username: "root".into(),
            auth: AuthMethod::Agent,
            tags: vec![" prod ".into(), "".into()],
        }
    }

    #[test]
    fn validate_normalizes_fields() {
        let draft = draft().validate().unwrap();
        assert_eq!(draft.hostname, "example.com");
        assert_eq!(draft.name, "example.com");
        assert_eq!(draft.group, None);
        assert_eq!(draft.tags, vec!["prod".to_owned()]);
    }

    #[test]
    fn validate_rejects_bad_input() {
        assert!(
            HostDraft {
                hostname: "a b".into(),
                ..draft()
            }
            .validate()
            .is_err()
        );
        assert!(
            HostDraft {
                username: "".into(),
                ..draft()
            }
            .validate()
            .is_err()
        );
        assert!(HostDraft { port: 0, ..draft() }.validate().is_err());
        assert!(
            HostDraft {
                auth: AuthMethod::KeyFile { path: " ".into() },
                ..draft()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::Password {
            password: Zeroizing::new("hunter2".into()),
        };
        assert_eq!(format!("{secret:?}"), "Secret::Password(..)");
    }
}
