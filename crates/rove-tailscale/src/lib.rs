//! Read-only discovery of devices reachable through the installed Tailscale client.
//!
//! [`TailnetDiscovery`] is the Definition, [`CliDiscovery`] the desktop Provider, and the
//! application's device browser a Consumer. Discovery neither changes the network nor grants
//! SSH access; selected addresses use the application's ordinary SSH and SFTP providers.

mod cli;
mod status;

use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use serde::{Deserialize, Serialize};

pub use cli::CliDiscovery;

/// Stable Tailscale device identity, from `PeerStatus.ID`, independent of a node's public key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TailscalePeerId(String);

impl TailscalePeerId {
    /// Accepts a non-empty printable, whitespace-free identifier of at most 256 bytes.
    pub fn new(value: String) -> Result<Self, TailscaleError> {
        if value.is_empty()
            || value.len() > 256
            || value.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(TailscaleError::InvalidStatus("invalid peer identity"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TailscalePeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<String> for TailscalePeerId {
    type Error = TailscaleError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<TailscalePeerId> for String {
    fn from(value: TailscalePeerId) -> Self {
        value.0
    }
}

/// Client or daemon state, not a statement that a peer's SSH service is reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TailnetState {
    NotInstalled,
    ServiceUnavailable,
    PermissionDenied,
    NoState,
    NeedsLogin,
    NeedsMachineAuth,
    Stopped,
    Starting,
    Running,
    Unknown,
}

/// Fields needed to display and select a device. Account lists and authentication URLs are omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetPeer {
    pub id: TailscalePeerId,
    pub name: String,
    pub dns_name: String,
    /// Deduplicated addresses, with IPv4 before IPv6.
    pub addresses: Vec<IpAddr>,
    pub os: Option<String>,
    /// Whether the device is connected to the Tailscale control plane.
    pub online: bool,
}

/// One read-only snapshot. `captured_at` is the local capture time in Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetSnapshot {
    pub state: TailnetState,
    pub peers: Vec<TailnetPeer>,
    pub captured_at: i64,
    /// A static diagnostic without CLI output, account details or login URLs.
    pub message: Option<String>,
}

/// Discovery failure. Errors never contain raw client output.
#[derive(Debug, thiserror::Error)]
pub enum TailscaleError {
    #[error("invalid Tailscale discovery configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("the Tailscale status command timed out")]
    Timeout,
    #[error("the Tailscale status command exceeded its output limit")]
    OutputLimit,
    #[error("the Tailscale client returned invalid JSON")]
    InvalidJson,
    #[error("the Tailscale client returned an invalid status: {0}")]
    InvalidStatus(&'static str),
    #[error("could not communicate with the Tailscale client: {0}")]
    Io(#[source] std::io::Error),
    #[error("the local clock cannot be represented as Unix milliseconds")]
    Clock,
}

/// Definition of local tailnet discovery. Each call starts a fresh bounded read.
#[async_trait]
pub trait TailnetDiscovery: Send + Sync {
    async fn snapshot(&self) -> Result<TailnetSnapshot, TailscaleError>;
}

/// `[plugin.tailscale]` configuration. Missing client software is reported by discovery, not boot.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TailscaleConfig {
    /// Absolute executable path, or automatic platform discovery.
    pub program: Option<PathBuf>,
    pub timeout_ms: u64,
    /// Shared bound for stdout and stderr together.
    pub max_output_bytes: usize,
}

impl Default for TailscaleConfig {
    fn default() -> Self {
        Self {
            program: None,
            timeout_ms: 5_000,
            max_output_bytes: 8 * 1024 * 1024,
        }
    }
}

impl TailscaleConfig {
    pub(crate) fn validate(self) -> Result<Self, TailscaleError> {
        if let Some(program) = &self.program
            && (!program.is_absolute()
                || program.as_os_str().is_empty()
                || program.to_string_lossy().chars().any(char::is_control))
        {
            return Err(TailscaleError::InvalidConfig(
                "program must be an absolute path without control characters",
            ));
        }
        if !(100..=60_000).contains(&self.timeout_ms) {
            return Err(TailscaleError::InvalidConfig(
                "timeout_ms must be between 100 and 60000",
            ));
        }
        if !(1024..=64 * 1024 * 1024).contains(&self.max_output_bytes) {
            return Err(TailscaleError::InvalidConfig(
                "max_output_bytes must be between 1024 and 67108864",
            ));
        }
        Ok(self)
    }
}

/// Registers the CLI Provider as `dyn TailnetDiscovery`.
pub struct TailscalePlugin;

impl Plugin for TailscalePlugin {
    fn name(&self) -> &'static str {
        "tailscale"
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: TailscaleConfig = config.parse()?;
        let provider =
            CliDiscovery::new(config).map_err(|error| KernelError::plugin("tailscale", error))?;
        Ok(vec![
            ctx.provide::<dyn TailnetDiscovery>(Arc::new(provider))?,
        ])
    }
}
