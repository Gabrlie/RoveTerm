//! Embedded Tailscale networking and peer discovery.

mod embedded;

use std::{fmt, net::IpAddr, sync::Arc, time::Duration};

use async_trait::async_trait;
use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use zeroize::Zeroizing;

pub use embedded::{EmbeddedTailnet, generate_key_state};

/// Stable Tailscale device identity, from the control plane, independent of a node's public key.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TailnetState {
    Unconfigured,
    Disconnected,
    Starting,
    NeedsLogin,
    NeedsMachineAuth,
    Running,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetDerpRegion {
    pub id: u16,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TailnetPath {
    Direct {
        endpoint: String,
    },
    Relay {
        region_id: u16,
        region_code: String,
        region_name: String,
    },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetPingResult {
    pub peer_id: TailscalePeerId,
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub path: TailnetPath,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetSelfNode {
    pub id: TailscalePeerId,
    pub name: String,
    pub dns_name: String,
    pub addresses: Vec<IpAddr>,
    pub tailnet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_derp: Option<TailnetDerpRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetPeer {
    pub id: TailscalePeerId,
    pub name: String,
    pub dns_name: String,
    pub addresses: Vec<IpAddr>,
    pub os: Option<String>,
    pub online: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetStatus {
    pub configured: bool,
    pub state: TailnetState,
    pub self_node: Option<TailnetSelfNode>,
    pub auth_url: Option<String>,
    pub peer_count: usize,
    /// Unix milliseconds when the displayed status changed.
    pub updated_at: i64,
    pub message: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetSnapshot {
    pub status: TailnetStatus,
    pub peers: Vec<TailnetPeer>,
    /// Unix milliseconds when the snapshot was read.
    pub captured_at: i64,
}

pub struct ConnectRequest {
    pub key_state_json: Zeroizing<Vec<u8>>,
    pub auth_key: Option<Zeroizing<String>>,
    pub control_url: Option<String>,
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNetworkAddr {
    pub ip: IpAddr,
    pub prefix_len: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNetworkInterface {
    pub id: u64,
    pub name: String,
    pub addrs: Vec<HostNetworkAddr>,
    pub default_v4: bool,
    pub default_v6: bool,
}

/// The host's active network interfaces. An empty list signals disconnection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNetworkSnapshot {
    pub interfaces: Vec<HostNetworkInterface>,
}

pub trait TailnetStream:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static
{
}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static> TailnetStream for T {}

/// A local tailnet connection and its peer directory.
#[async_trait]
pub trait TailnetService: Send + Sync {
    fn status(&self) -> TailnetStatus;
    fn subscribe(&self) -> watch::Receiver<TailnetStatus>;
    async fn snapshot(&self) -> Result<TailnetSnapshot, TailscaleError>;
    async fn connect(&self, request: ConnectRequest) -> Result<TailnetStatus, TailscaleError>;
    async fn disconnect(&self, configured: bool) -> Result<TailnetStatus, TailscaleError>;
    async fn ping_peer(
        &self,
        peer_id: &TailscalePeerId,
    ) -> Result<TailnetPingResult, TailscaleError>;
    /// Read the current path to a peer from local routing state, when available.
    async fn peer_path(&self, _host: &str, _peer_id: Option<&str>) -> Option<TailnetPath> {
        None
    }
    async fn dial_tcp(
        &self,
        host: &str,
        port: u16,
        peer_id: Option<&str>,
    ) -> Result<Box<dyn TailnetStream>, TailscaleError>;
    fn set_configured(&self, configured: bool);
    /// Publish the latest host network state for platforms that use an external monitor.
    fn update_network(&self, _snapshot: HostNetworkSnapshot) {}
    /// Check the control connection after the host application returns to the foreground.
    fn notify_foreground(&self) {}
}

/// Errors are safe to relay to the WebView and logs.
#[derive(Debug, thiserror::Error)]
pub enum TailscaleError {
    #[error("invalid Tailscale configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("invalid Tailscale node data: {0}")]
    InvalidStatus(&'static str),
    #[error("invalid Tailscale identity")]
    InvalidKeyState,
    #[error("Tailscale registration rejected the auth key")]
    KeyRejected,
    #[error("Tailscale needs browser login or machine approval")]
    NeedsLogin,
    #[error("Tailscale is disconnected")]
    NotConnected,
    #[error("Tailscale operation timed out")]
    Timeout,
    #[error("the embedded Tailscale device failed")]
    Device,
    #[error("Tailscale peer has no matching tailnet address")]
    PeerNotFound,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TailscaleConfig {
    pub connect_timeout_ms: u64,
    pub peer_refresh_ms: u64,
}

impl Default for TailscaleConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 15_000,
            peer_refresh_ms: 2_000,
        }
    }
}

impl TailscaleConfig {
    fn resolve(self) -> Result<(Duration, Duration), TailscaleError> {
        if !(100..=120_000).contains(&self.connect_timeout_ms) {
            return Err(TailscaleError::InvalidConfig(
                "connect_timeout_ms must be 100–120000",
            ));
        }
        if !(250..=60_000).contains(&self.peer_refresh_ms) {
            return Err(TailscaleError::InvalidConfig(
                "peer_refresh_ms must be 250–60000",
            ));
        }
        Ok((
            Duration::from_millis(self.connect_timeout_ms),
            Duration::from_millis(self.peer_refresh_ms),
        ))
    }
}

pub struct TailscalePlugin;

impl Plugin for TailscalePlugin {
    fn name(&self) -> &'static str {
        "tailscale"
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let provider = EmbeddedTailnet::new(config.parse()?)
            .map_err(|error| KernelError::plugin("tailscale", error))?;
        Ok(vec![ctx.provide::<dyn TailnetService>(Arc::new(provider))?])
    }
}
