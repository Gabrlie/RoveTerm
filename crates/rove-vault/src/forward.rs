//! Saved port forwarding rules. Rules carry no secrets; the forwards run in `rove-ssh`.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{HostId, VaultError};

/// Identifier of a saved forwarding rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ForwardId(Uuid);

impl ForwardId {
    /// Generates a new time-ordered identifier.
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for ForwardId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ForwardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ForwardId {
    type Err = VaultError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self).map_err(|_| {
            VaultError::Invalid(format!(
                "invalid forward id `{}`",
                s.chars().take(64).collect::<String>()
            ))
        })
    }
}

/// What a rule forwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ForwardSpec {
    /// `-L`: listen on this device, connect to `dest` from the server.
    Local {
        /// Local address to listen on: an IP literal or `localhost`.
        bind_host: String,
        /// Local port (1–65535).
        bind_port: u16,
        /// Destination as seen from the server.
        dest_host: String,
        /// Destination port (1–65535).
        dest_port: u16,
    },
    /// `-R`: the server listens, connections are made to `dest` from this device.
    Remote {
        /// Server-side address to listen on, interpreted by the server.
        bind_host: String,
        /// Server-side port; 0 lets the server choose.
        bind_port: u16,
        /// Destination as seen from this device.
        dest_host: String,
        /// Destination port (1–65535).
        dest_port: u16,
    },
    /// `-D`: a SOCKS5 proxy on this device; each connection is made from the server.
    Dynamic {
        /// Local address to listen on: an IP literal or `localhost`.
        bind_host: String,
        /// Local port (1–65535).
        bind_port: u16,
    },
}

fn check_host(field: &str, value: &str) -> Result<String, VaultError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 255 || value.chars().any(char::is_whitespace) {
        return Err(VaultError::Invalid(format!("{field} is not a valid host")));
    }
    Ok(value.to_owned())
}

fn check_local_bind(value: &str) -> Result<String, VaultError> {
    let value = check_host("bind_host", value)?;
    if value.eq_ignore_ascii_case("localhost") || value.parse::<IpAddr>().is_ok() {
        Ok(value)
    } else {
        Err(VaultError::Invalid(
            "bind_host must be an IP address or localhost".into(),
        ))
    }
}

fn check_port(field: &str, value: u16) -> Result<u16, VaultError> {
    if value == 0 {
        Err(VaultError::Invalid(format!("{field} must be 1-65535")))
    } else {
        Ok(value)
    }
}

impl ForwardSpec {
    /// Trims hosts and checks hosts and ports. Returns the normalized spec.
    pub fn validate(self) -> Result<Self, VaultError> {
        Ok(match self {
            Self::Local {
                bind_host,
                bind_port,
                dest_host,
                dest_port,
            } => Self::Local {
                bind_host: check_local_bind(&bind_host)?,
                bind_port: check_port("bind_port", bind_port)?,
                dest_host: check_host("dest_host", &dest_host)?,
                dest_port: check_port("dest_port", dest_port)?,
            },
            Self::Remote {
                bind_host,
                bind_port,
                dest_host,
                dest_port,
            } => Self::Remote {
                bind_host: check_host("bind_host", &bind_host)?,
                bind_port,
                dest_host: check_host("dest_host", &dest_host)?,
                dest_port: check_port("dest_port", dest_port)?,
            },
            Self::Dynamic {
                bind_host,
                bind_port,
            } => Self::Dynamic {
                bind_host: check_local_bind(&bind_host)?,
                bind_port: check_port("bind_port", bind_port)?,
            },
        })
    }

    /// Returns `true` when a local listener accepts connections from other machines.
    pub fn listens_publicly(&self) -> bool {
        match self {
            Self::Local { bind_host, .. } | Self::Dynamic { bind_host, .. } => {
                !bind_host.eq_ignore_ascii_case("localhost")
                    && bind_host
                        .parse::<IpAddr>()
                        .is_ok_and(|ip| !ip.is_loopback())
            }
            Self::Remote { .. } => false,
        }
    }
}

/// A saved forwarding rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardRule {
    /// Identifier.
    pub id: ForwardId,
    /// Host whose connection carries the forward.
    pub host_id: HostId,
    /// Display label; may be empty.
    pub label: String,
    /// What to forward.
    pub spec: ForwardSpec,
    /// Start whenever a terminal to the host opens.
    pub auto_start: bool,
    /// Creation time, Unix milliseconds.
    pub created_at: i64,
    /// Last modification time, Unix milliseconds.
    pub updated_at: i64,
}

/// Input for creating or updating a rule. Validated by [`ForwardDraft::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardDraft {
    /// Display label.
    #[serde(default)]
    pub label: String,
    /// What to forward.
    pub spec: ForwardSpec,
    /// Start whenever a terminal to the host opens.
    #[serde(default)]
    pub auto_start: bool,
}

impl ForwardDraft {
    /// Trims the label and validates the spec. Returns the normalized draft.
    pub fn validate(self) -> Result<Self, VaultError> {
        let label = self.label.trim().to_owned();
        if label.chars().count() > 128 {
            return Err(VaultError::Invalid(
                "label is longer than 128 characters".into(),
            ));
        }
        Ok(Self {
            label,
            spec: self.spec.validate()?,
            auto_start: self.auto_start,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(bind_host: &str, bind_port: u16) -> ForwardSpec {
        ForwardSpec::Local {
            bind_host: bind_host.into(),
            bind_port,
            dest_host: "db.internal".into(),
            dest_port: 5432,
        }
    }

    #[test]
    fn local_binds_must_be_addresses() {
        assert!(local(" 127.0.0.1 ", 8080).validate().is_ok());
        assert!(local("localhost", 8080).validate().is_ok());
        assert!(local("::1", 8080).validate().is_ok());
        assert!(local("example.com", 8080).validate().is_err());
        assert!(local("127.0.0.1", 0).validate().is_err());
    }

    #[test]
    fn remote_port_zero_lets_the_server_choose() {
        let spec = ForwardSpec::Remote {
            bind_host: "localhost".into(),
            bind_port: 0,
            dest_host: "127.0.0.1".into(),
            dest_port: 3000,
        };
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn public_listeners_are_detected() {
        assert!(!local("127.0.0.1", 1).listens_publicly());
        assert!(!local("localhost", 1).listens_publicly());
        assert!(local("0.0.0.0", 1).listens_publicly());
        assert!(local("192.168.1.2", 1).listens_publicly());
    }

    #[test]
    fn spec_serializes_with_kind_tag() {
        let json = serde_json::to_value(local("127.0.0.1", 15432)).unwrap();
        assert_eq!(json["kind"], "local");
        assert_eq!(json["dest_port"], 5432);
    }
}
