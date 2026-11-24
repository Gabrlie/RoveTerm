use std::{
    net::{IpAddr, SocketAddr},
    panic::AssertUnwindSafe,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use futures::FutureExt;
use tailscale::{
    AuthStatus, ControlLinkState, Device, NodeInfo, NodeStatus, ProbePath, ProbeResult,
    keys::PersistState,
};
use tokio::{
    sync::{Mutex, oneshot, watch},
    task::JoinHandle,
    time::{Instant as TokioInstant, MissedTickBehavior, interval, sleep, sleep_until, timeout},
};
use zeroize::Zeroizing;

use crate::{
    ConnectRequest, TailnetDerpRegion, TailnetPath, TailnetPeer, TailnetPingResult,
    TailnetSelfNode, TailnetService, TailnetSnapshot, TailnetState, TailnetStatus, TailnetStream,
    TailscaleConfig, TailscaleError, TailscalePeerId,
};

pub fn generate_key_state() -> Result<Zeroizing<Vec<u8>>, TailscaleError> {
    let mut bytes = Zeroizing::new(Vec::new());
    serde_json::to_writer(&mut *bytes, &PersistState::default())
        .map_err(|_| TailscaleError::InvalidKeyState)?;
    Ok(bytes)
}

struct Current {
    status: watch::Sender<TailnetStatus>,
    peers: Vec<TailnetPeer>,
    device: Option<Arc<Device>>,
}

impl Current {
    fn publish(
        &mut self,
        configured: bool,
        state: TailnetState,
        auth_url: Option<String>,
        message: Option<String>,
        self_node: Option<TailnetSelfNode>,
        peers: Vec<TailnetPeer>,
    ) {
        let peers_changed = self.peers != peers;
        self.peers = peers;
        self.status.send_if_modified(|current| {
            let mut next = TailnetStatus {
                configured,
                state,
                self_node,
                auth_url,
                peer_count: self.peers.len(),
                updated_at: current.updated_at,
                message,
            };
            if !peers_changed && *current == next {
                return false;
            }
            if current.state != state {
                tracing::info!(from = ?current.state, to = ?state, "Tailscale state changed");
            }
            if current.peer_count != next.peer_count {
                tracing::info!(peer_count = next.peer_count, "updated Tailscale peer count");
            }
            next.updated_at = now_millis();
            *current = next;
            true
        });
    }

    fn idle(&mut self, configured: bool) {
        let state = if configured {
            TailnetState::Disconnected
        } else {
            TailnetState::Unconfigured
        };
        self.publish(configured, state, None, None, None, Vec::new());
    }
}

type Shared = Arc<parking_lot::Mutex<Current>>;

struct Live {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

pub struct EmbeddedTailnet {
    current: Shared,
    live: Mutex<Option<Live>>,
    connect_timeout: Duration,
    peer_refresh: Duration,
}

impl EmbeddedTailnet {
    pub fn new(config: TailscaleConfig) -> Result<Self, TailscaleError> {
        let (connect_timeout, peer_refresh) = config.resolve()?;
        Ok(Self {
            current: Arc::new(parking_lot::Mutex::new(Current {
                status: watch::Sender::new(TailnetStatus {
                    configured: false,
                    state: TailnetState::Unconfigured,
                    self_node: None,
                    auth_url: None,
                    peer_count: 0,
                    updated_at: now_millis(),
                    message: None,
                }),
                peers: Vec::new(),
                device: None,
            })),
            live: Mutex::new(None),
            connect_timeout,
            peer_refresh,
        })
    }
}

async fn stop(live: &mut Option<Live>) {
    if let Some(active) = live.as_mut() {
        drop(active.stop.take());
        // The task also exits when the command initiating shutdown is cancelled.
        let _ = (&mut active.task).await;
        live.take();
    }
}

#[async_trait]
impl TailnetService for EmbeddedTailnet {
    fn status(&self) -> TailnetStatus {
        self.current.lock().status.borrow().clone()
    }

    fn subscribe(&self) -> watch::Receiver<TailnetStatus> {
        self.current.lock().status.subscribe()
    }

    async fn snapshot(&self) -> Result<TailnetSnapshot, TailscaleError> {
        let current = self.current.lock();
        Ok(TailnetSnapshot {
            status: current.status.borrow().clone(),
            peers: current.peers.clone(),
            captured_at: now_millis(),
        })
    }

    async fn connect(&self, request: ConnectRequest) -> Result<TailnetStatus, TailscaleError> {
        rove_http::install_crypto_provider();
        let ConnectRequest {
            key_state_json,
            auth_key,
            control_url,
            hostname,
        } = request;
        let params = ConnectParams {
            key_state_json,
            control_url,
            hostname,
        };
        let config = params.resolve()?;
        let mut live = self.live.lock().await;
        stop(&mut live).await;
        self.current
            .lock()
            .publish(true, TailnetState::Starting, None, None, None, Vec::new());

        let (stop, mut cancelled) = oneshot::channel();
        let current = self.current.clone();
        let connect_timeout = self.connect_timeout;
        let peer_refresh = self.peer_refresh;
        let task = tokio::spawn(async move {
            let result = AssertUnwindSafe(run(
                &current,
                params,
                config,
                auth_key,
                connect_timeout,
                peer_refresh,
                &mut cancelled,
            ))
            .catch_unwind()
            .await
            .unwrap_or(Err(TailscaleError::Device));
            let device = {
                let mut current = current.lock();
                let device = current.device.take();
                if let Err(error) = result {
                    tracing::warn!(%error, "Tailscale connection ended");
                    current.publish(
                        true,
                        TailnetState::Error,
                        None,
                        Some(error.to_string()),
                        None,
                        Vec::new(),
                    );
                } else {
                    current.idle(true);
                }
                device
            };
            if let Some(device) = device {
                shutdown_device(&device, connect_timeout).await;
            }
        });
        *live = Some(Live {
            stop: Some(stop),
            task,
        });
        Ok(self.status())
    }

    async fn disconnect(&self, configured: bool) -> Result<TailnetStatus, TailscaleError> {
        let mut live = self.live.lock().await;
        stop(&mut live).await;
        self.current.lock().idle(configured);
        Ok(self.status())
    }

    async fn ping_peer(
        &self,
        peer_id: &TailscalePeerId,
    ) -> Result<TailnetPingResult, TailscaleError> {
        const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
        let device = self.running_device()?;
        let result = device
            .ping_peer(peer_id.as_str(), PROBE_TIMEOUT)
            .await
            .map_err(|_| TailscaleError::Device)?;
        let (latency_ms, path, message) = match result {
            ProbeResult::Reachable { latency, path } => (
                Some(latency.as_millis().try_into().unwrap_or(u64::MAX)),
                tailnet_path(path),
                None,
            ),
            ProbeResult::Unreachable(reason) => {
                (None, TailnetPath::Unknown, Some(reason.to_owned()))
            }
        };
        Ok(TailnetPingResult {
            peer_id: peer_id.clone(),
            reachable: latency_ms.is_some(),
            latency_ms,
            path,
            message,
        })
    }

    async fn peer_path(&self, host: &str, peer_id: Option<&str>) -> Option<TailnetPath> {
        let device = self.running_device().ok()?;
        let node;
        let stable_id = if let Some(id) = peer_id {
            id
        } else {
            node = if let Some(ip) = parse_host_ip(host) {
                device.peer_by_tailnet_ip(ip).await.ok()??
            } else {
                resolve_peer(&device, host, None).await.ok()?
            };
            &node.stable_id.0
        };
        device.peer_path(stable_id).await.ok().map(tailnet_path)
    }

    async fn dial_tcp(
        &self,
        host: &str,
        port: u16,
        peer_id: Option<&str>,
    ) -> Result<Box<dyn TailnetStream>, TailscaleError> {
        let device = self.running_device()?;
        let started = Instant::now();
        tracing::info!(port, "starting Tailscale TCP dial");
        let stream = timeout(self.connect_timeout, async {
            let ip = match parse_host_ip(host) {
                Some(ip) => ip,
                None => peer_address(&resolve_peer(&device, host, peer_id).await?),
            };
            tracing::info!(%ip, port, "resolved Tailscale TCP destination");
            device
                .tcp_connect(SocketAddr::new(ip, port))
                .await
                .map_err(|_| TailscaleError::Device)
        })
        .await
        .map_err(|_| TailscaleError::Timeout)
        .and_then(|result| result);
        let elapsed_ms = started.elapsed().as_millis();
        match stream {
            Ok(stream) => {
                tracing::info!(elapsed_ms, "Tailscale TCP connected");
                Ok(Box::new(stream))
            }
            Err(error) => {
                tracing::warn!(elapsed_ms, %error, "Tailscale TCP dial failed");
                if matches!(error, TailscaleError::Timeout | TailscaleError::Device) {
                    tokio::spawn(async move {
                        let _ = device.probe_control().await;
                    });
                }
                Err(error)
            }
        }
    }

    fn set_configured(&self, configured: bool) {
        let mut current = self.current.lock();
        let idle = matches!(
            current.status.borrow().state,
            TailnetState::Unconfigured | TailnetState::Disconnected
        );
        if idle {
            current.idle(configured);
        }
    }
}

impl EmbeddedTailnet {
    fn running_device(&self) -> Result<Arc<Device>, TailscaleError> {
        let current = self.current.lock();
        match current.status.borrow().state {
            TailnetState::Running => current.device.clone().ok_or(TailscaleError::NotConnected),
            TailnetState::NeedsLogin | TailnetState::NeedsMachineAuth => {
                Err(TailscaleError::NeedsLogin)
            }
            _ => Err(TailscaleError::NotConnected),
        }
    }
}

fn derp_region(region: tailscale::DerpRegion) -> Option<TailnetDerpRegion> {
    Some(TailnetDerpRegion {
        id: region.id.try_into().ok()?,
        code: region.code,
        name: region.name,
    })
}

struct ConnectParams {
    key_state_json: Zeroizing<Vec<u8>>,
    control_url: Option<String>,
    hostname: Option<String>,
}

impl ConnectParams {
    fn resolve(&self) -> Result<tailscale::Config, TailscaleError> {
        let key_state: PersistState = serde_json::from_slice(&self.key_state_json)
            .map_err(|_| TailscaleError::InvalidKeyState)?;
        let mut config = tailscale::Config::from(key_state);
        config.client_name = Some("RoveTerm".into());
        if let Some(url) = &self.control_url {
            let parsed =
                url::Url::parse(url).map_err(|_| TailscaleError::InvalidConfig("control URL"))?;
            if !is_web_url(&parsed) {
                return Err(TailscaleError::InvalidConfig("control URL"));
            }
            config.control_server_url = parsed;
        }
        config.requested_hostname = Some(
            self.hostname
                .as_ref()
                .filter(|hostname| !hostname.is_empty())
                .cloned()
                .unwrap_or_else(|| "RoveTerm".into()),
        );
        Ok(config)
    }
}

fn is_web_url(url: &url::Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

const CONTROL_GRACE: Duration = Duration::from_secs(6);
const CONTROL_REBUILD: Duration = Duration::from_secs(25);
const RECOVERY_INITIAL_DELAY: Duration = Duration::from_secs(1);

fn wake_jump_threshold(tick: Duration) -> Duration {
    (tick * 5).max(Duration::from_secs(10))
}

fn detect_sleep_jump(mono_elapsed: Duration, wall_elapsed: Duration, threshold: Duration) -> bool {
    mono_elapsed >= threshold || wall_elapsed >= threshold
}

fn next_recovery_delay(delay: Duration) -> Duration {
    let cap = if delay < Duration::from_secs(15) {
        15
    } else {
        30
    };
    delay.saturating_mul(2).min(Duration::from_secs(cap))
}

async fn shutdown_device(device: &Device, limit: Duration) {
    let _ = AssertUnwindSafe(device.shutdown_ref(Some(limit)))
        .catch_unwind()
        .await;
}

async fn run(
    current: &Shared,
    params: ConnectParams,
    mut config: tailscale::Config,
    mut auth_key: Option<Zeroizing<String>>,
    connect_timeout: Duration,
    peer_refresh: Duration,
    cancelled: &mut oneshot::Receiver<()>,
) -> Result<(), TailscaleError> {
    let mut was_running = false;
    let mut recovery_delay = RECOVERY_INITIAL_DELAY;
    loop {
        let result = tokio::select! {
            biased;
            _ = &mut *cancelled => return Ok(()),
            result = async {
                let device = timeout(connect_timeout, Device::new_with_secret(&config, auth_key.take()))
                    .await
                    .map_err(|_| TailscaleError::Timeout)?
                    .map_err(|error| match error {
                        tailscale::Error::AuthKeyRejected => TailscaleError::KeyRejected,
                        _ => TailscaleError::Device,
                    })?;
                drop(config);
                let device = Arc::new(device);
                current.lock().device = Some(device.clone());
                monitor_device(
                    current, &device, connect_timeout, peer_refresh,
                    &mut was_running, &mut recovery_delay,
                ).await
            } => result,
        };

        match result {
            Err(error) if !was_running => return Err(error),
            Err(error @ (TailscaleError::Timeout | TailscaleError::Device)) => {
                tracing::info!(%error, "rebuilding Tailscale device after runtime failure");
            }
            Err(error) => return Err(error),
            Ok(()) => {}
        }

        let device = {
            let mut current = current.lock();
            let device = current.device.take();
            current.publish(true, TailnetState::Starting, None, None, None, Vec::new());
            device
        };
        if let Some(device) = device {
            shutdown_device(&device, Duration::from_secs(3)).await;
        }

        tokio::select! {
            biased;
            _ = &mut *cancelled => return Ok(()),
            _ = sleep(recovery_delay) => {}
        }
        recovery_delay = next_recovery_delay(recovery_delay);
        config = params.resolve()?;
    }
}

async fn monitor_device(
    current: &Shared,
    device: &Device,
    connect_timeout: Duration,
    peer_refresh: Duration,
    was_running: &mut bool,
    recovery_delay: &mut Duration,
) -> Result<(), TailscaleError> {
    let mut auth = timeout(connect_timeout, device.subscribe_auth())
        .await
        .map_err(|_| TailscaleError::Timeout)?
        .map_err(|_| TailscaleError::Device)?;
    let mut control = timeout(connect_timeout, device.subscribe_control_link())
        .await
        .map_err(|_| TailscaleError::Timeout)?
        .map_err(|_| TailscaleError::Device)?;

    let tick = peer_refresh.min(Duration::from_secs(2));
    let mut heartbeat = interval(tick);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let threshold = wake_jump_threshold(tick);
    let (mut last_mono, mut last_wall) = (Instant::now(), SystemTime::now());
    let mut starting_since = None;
    let mut reconnecting_since = None;
    let mut next_refresh = None;
    let mut ready = false;

    loop {
        let now = TokioInstant::now();
        let status = auth.borrow_and_update().clone();
        let link = *control.borrow_and_update();
        let (mut deadline, mut refresh_at) = (None, None);
        if !matches!(status, AuthStatus::Authorized) {
            reconnecting_since = None;
            next_refresh = None;
            ready = false;
        }

        match status {
            AuthStatus::Authorized if link == ControlLinkState::Connected => {
                starting_since = None;
                if reconnecting_since.take().is_some() {
                    next_refresh = None;
                }
                refresh_at = Some(next_refresh.unwrap_or(now));
            }
            AuthStatus::Authorized if ready => {
                let since = *reconnecting_since.get_or_insert(now);
                let elapsed = now.duration_since(since);
                if elapsed >= CONTROL_REBUILD {
                    tracing::info!("rebuilding Tailscale device after prolonged control outage");
                    return Ok(());
                }
                if elapsed >= CONTROL_GRACE {
                    current.lock().publish(
                        true,
                        TailnetState::Starting,
                        None,
                        None,
                        None,
                        Vec::new(),
                    );
                    deadline = Some(since + CONTROL_REBUILD);
                } else {
                    deadline = Some(since + CONTROL_GRACE);
                }
            }
            AuthStatus::Starting | AuthStatus::Authorized => {
                current
                    .lock()
                    .publish(true, TailnetState::Starting, None, None, None, Vec::new());
                let expires = *starting_since.get_or_insert(now) + connect_timeout;
                if now >= expires {
                    return Err(TailscaleError::Timeout);
                }
                deadline = Some(expires);
            }
            AuthStatus::InteractiveAuthRequired(url) => {
                if !is_web_url(&url) {
                    return Err(TailscaleError::InvalidStatus("invalid browser login URL"));
                }
                starting_since = None;
                current.lock().publish(
                    true,
                    TailnetState::NeedsLogin,
                    Some(url.into()),
                    None,
                    None,
                    Vec::new(),
                );
            }
            AuthStatus::KeyRejected => return Err(TailscaleError::KeyRejected),
            AuthStatus::NeedsMachineAuth => {
                starting_since = None;
                current.lock().publish(
                    true,
                    TailnetState::NeedsMachineAuth,
                    None,
                    None,
                    None,
                    Vec::new(),
                );
            }
        }

        let refresh = async {
            wait_until(refresh_at).await;
            timeout(connect_timeout, read_nodes(device))
                .await
                .map_err(|_| TailscaleError::Timeout)?
        };
        tokio::pin!(refresh);
        loop {
            tokio::select! {
                biased;
                _ = heartbeat.tick() => {
                    let (now_mono, now_wall) = (Instant::now(), SystemTime::now());
                    let mono_elapsed = now_mono.duration_since(last_mono);
                    let wall_elapsed = now_wall.duration_since(last_wall).unwrap_or(Duration::ZERO);
                    (last_mono, last_wall) = (now_mono, now_wall);
                    if *was_running && detect_sleep_jump(mono_elapsed, wall_elapsed, threshold) {
                        tracing::info!(?mono_elapsed, ?wall_elapsed, "rebuilding Tailscale device after wake");
                        return Ok(());
                    }
                }
                result = auth.changed() => {
                    result.map_err(|_| TailscaleError::Device)?;
                    break;
                }
                result = control.changed() => {
                    result.map_err(|_| TailscaleError::Device)?;
                    if *control.borrow() == ControlLinkState::Connected {
                        next_refresh = None;
                    }
                    break;
                }
                _ = wait_until(deadline) => break,
                result = &mut refresh => {
                    let (self_node, peers) = result?;
                    current.lock().publish(
                        true, TailnetState::Running, None, None, Some(self_node), peers,
                    );
                    *was_running = true;
                    *recovery_delay = RECOVERY_INITIAL_DELAY;
                    ready = true;
                    next_refresh = Some(refresh_at.unwrap_or(now) + peer_refresh);
                    if next_refresh.is_some_and(|next| next <= TokioInstant::now()) {
                        next_refresh = Some(TokioInstant::now() + peer_refresh);
                    }
                    break;
                }
            }
        }
    }
}

async fn wait_until(deadline: Option<TokioInstant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn read_nodes(
    device: &Device,
) -> Result<(TailnetSelfNode, Vec<TailnetPeer>), TailscaleError> {
    let node = device
        .self_node()
        .await
        .map_err(|_| TailscaleError::Device)?;
    let peers = device.peers().await.map_err(|_| TailscaleError::Device)?;
    let home_derp = device
        .home_derp()
        .await
        .map_err(|_| TailscaleError::Device)?
        .and_then(derp_region);
    let self_node = TailnetSelfNode {
        home_derp,
        dns_name: node.fqdn(false),
        addresses: addresses(&node).to_vec(),
        id: TailscalePeerId::new(node.stable_id.0)?,
        name: node.hostname,
        tailnet: node.tailnet,
    };
    Ok((self_node, sorted_peers(peers)))
}

fn sorted_peers(nodes: Vec<NodeInfo>) -> Vec<TailnetPeer> {
    let mut peers: Vec<_> = nodes
        .into_iter()
        .filter_map(|node| {
            let id = TailscalePeerId::new(node.stable_id.0.clone()).ok()?;
            Some(TailnetPeer {
                id,
                dns_name: node.fqdn(false),
                addresses: addresses(&node).to_vec(),
                name: node.hostname,
                os: None,
                online: matches!(node.status, NodeStatus::Online),
            })
        })
        .collect();
    peers.sort_by_cached_key(|peer| (peer.name.to_lowercase(), peer.id.clone()));
    peers
}

fn addresses(node: &NodeInfo) -> [IpAddr; 2] {
    [
        node.tailnet_address.ipv4.addr().into(),
        node.tailnet_address.ipv6.addr().into(),
    ]
}

fn parse_host_ip(host: &str) -> Option<IpAddr> {
    let trimmed = host.trim();
    let bare = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    bare.parse().ok()
}

fn select_peer<'a>(
    peers: &'a [NodeInfo],
    query: &str,
    peer_id: Option<&str>,
) -> Option<&'a NodeInfo> {
    if let Some(id) = peer_id {
        return peers.iter().find(|node| node.stable_id.0 == id);
    }
    let normalized = query.trim().trim_end_matches('.');
    if normalized.is_empty() {
        return None;
    }
    peers
        .iter()
        .find(|node| {
            let fqdn = node.fqdn(false);
            !fqdn.is_empty() && fqdn.eq_ignore_ascii_case(normalized)
        })
        .or_else(|| {
            peers
                .iter()
                .find(|node| node.hostname.eq_ignore_ascii_case(normalized))
        })
}

async fn resolve_peer(
    device: &Device,
    host: &str,
    peer_id: Option<&str>,
) -> Result<NodeInfo, TailscaleError> {
    let query = host.trim().trim_end_matches('.');
    let named = if query.is_empty() {
        None
    } else {
        device
            .peer_by_name(query)
            .await
            .map_err(|_| TailscaleError::Device)?
    };
    match named {
        Some(node) if peer_id.is_none_or(|id| node.stable_id.0 == id) => Ok(node),
        _ => {
            let peers = device.peers().await.map_err(|_| TailscaleError::Device)?;
            select_peer(&peers, query, peer_id)
                .cloned()
                .ok_or(TailscaleError::PeerNotFound)
        }
    }
}

fn peer_address(node: &NodeInfo) -> IpAddr {
    let [ipv4, ipv6] = addresses(node);
    if ipv4.is_unspecified() { ipv6 } else { ipv4 }
}

fn tailnet_path(path: ProbePath) -> TailnetPath {
    match path {
        ProbePath::Direct(endpoint) => TailnetPath::Direct {
            endpoint: endpoint.to_string(),
        },
        ProbePath::Relay(region) => {
            derp_region(region).map_or(TailnetPath::Unknown, |region| TailnetPath::Relay {
                region_id: region.id,
                region_code: region.code,
                region_name: region.name,
            })
        }
        ProbePath::Unknown => TailnetPath::Unknown,
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_threshold_is_five_ticks_or_at_least_ten_seconds() {
        assert_eq!(
            wake_jump_threshold(Duration::from_millis(250)),
            Duration::from_secs(10)
        );
        assert_eq!(
            wake_jump_threshold(Duration::from_secs(2)),
            Duration::from_secs(10)
        );
        assert_eq!(
            wake_jump_threshold(Duration::from_secs(3)),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn sleep_jump_uses_either_clock() {
        let threshold = Duration::from_secs(10);
        let short = Duration::from_secs(2);
        assert!(!detect_sleep_jump(short, short, threshold));
        assert!(!detect_sleep_jump(short, Duration::ZERO, threshold));
        assert!(detect_sleep_jump(threshold, short, threshold));
        assert!(detect_sleep_jump(short, threshold, threshold));
    }

    #[test]
    fn recovery_delay_increases_to_thirty_seconds() {
        let mut delay = RECOVERY_INITIAL_DELAY;
        for seconds in [1, 2, 4, 8, 15, 30, 30] {
            assert_eq!(delay, Duration::from_secs(seconds));
            delay = next_recovery_delay(delay);
        }
    }

    #[test]
    fn fresh_identity_round_trips_and_is_unique() {
        let state = generate_key_state().unwrap();
        let restored: PersistState = serde_json::from_slice(&state).unwrap();
        let mut serialized = Zeroizing::new(Vec::new());
        serde_json::to_writer(&mut *serialized, &restored).unwrap();
        assert!(serialized.as_slice() == state.as_slice());
        assert!(generate_key_state().unwrap().as_slice() != state.as_slice());
    }

    fn node(id: &str, name: &str, status: NodeStatus) -> NodeInfo {
        NodeInfo {
            id: 1,
            stable_id: ts_control::StableNodeId(id.into()),
            hostname: name.into(),
            capability_version: ts_capabilityversion::CapabilityVersion::CURRENT,
            status,
            tailnet: Some("example.ts.net".into()),
            node_capabilities: Default::default(),
            tags: Vec::new(),
            tailnet_address: ts_control::TailnetAddress {
                ipv4: "100.64.0.1/32".parse().unwrap(),
                ipv6: "fd7a:115c:a1e0::1/128".parse().unwrap(),
            },
            node_key: tailscale::keys::NodeKeyPair::random().public,
            node_key_expiry: None,
            machine_key: None,
            disco_key: None,
            tailnet_lock_key_signature: None,
            accepted_routes: Vec::new(),
            underlay_addresses: Vec::new(),
            derp_region: None,
        }
    }

    #[test]
    fn maps_peers_and_sorts_case_insensitively_then_by_id() {
        let peers = sorted_peers(vec![
            node("n1", "Zulu", NodeStatus::Unknown),
            node("n3", "alpha", NodeStatus::Unknown),
            node("n2", "Alpha", NodeStatus::Online),
            node("", "Ignored", NodeStatus::Unknown),
        ]);
        let ids: Vec<_> = peers.iter().map(|peer| peer.id.as_str()).collect();
        assert_eq!(ids, ["n2", "n3", "n1"]);
        assert_eq!(peers[0].name, "Alpha");
        assert_eq!(peers[0].dns_name, "Alpha.example.ts.net");
        assert_eq!(
            peers[0].addresses,
            [
                "100.64.0.1".parse::<IpAddr>().unwrap(),
                "fd7a:115c:a1e0::1".parse::<IpAddr>().unwrap(),
            ]
        );
        assert!(peers[0].online);
        assert!(!peers[1].online);
        assert!(peers[0].os.is_none());
    }

    #[test]
    fn maps_probe_paths_to_tailnet_paths() {
        let endpoint = "[::1]:41641";
        assert_eq!(
            tailnet_path(ProbePath::Direct(endpoint.parse().unwrap())),
            TailnetPath::Direct {
                endpoint: endpoint.to_owned()
            },
        );
        assert_eq!(
            tailnet_path(ProbePath::Relay(tailscale::DerpRegion {
                id: 1,
                code: "nyc".into(),
                name: "New York".into(),
            })),
            TailnetPath::Relay {
                region_id: 1,
                region_code: "nyc".into(),
                region_name: "New York".into(),
            },
        );
        assert_eq!(tailnet_path(ProbePath::Unknown), TailnetPath::Unknown);
    }

    #[test]
    fn parses_plain_and_bracketed_ips_and_resolves_peers_case_insensitively() {
        assert_eq!(
            parse_host_ip(" 100.64.0.1 "),
            Some("100.64.0.1".parse().unwrap())
        );
        assert_eq!(
            parse_host_ip("[fd7a:115c:a1e0::1]"),
            Some("fd7a:115c:a1e0::1".parse().unwrap())
        );
        assert_eq!(parse_host_ip("Alpha.example.ts.net"), None);

        let peers = vec![
            node("n1", "Zulu", NodeStatus::Online),
            node("n2", "Alpha", NodeStatus::Online),
        ];
        assert_eq!(
            select_peer(&peers, "alpha", None).map(|n| n.stable_id.0.as_str()),
            Some("n2")
        );
        assert_eq!(
            select_peer(&peers, "ALPHA.EXAMPLE.TS.NET.", None).map(|n| n.stable_id.0.as_str()),
            Some("n2")
        );
        assert_eq!(
            select_peer(&peers, "renamed-host", Some("n1")).map(|n| n.stable_id.0.as_str()),
            Some("n1")
        );
        assert!(select_peer(&peers, "missing", None).is_none());
        assert!(select_peer(&peers, "alpha", Some("missing")).is_none());
        assert_eq!(
            peer_address(&peers[0]),
            "100.64.0.1".parse::<IpAddr>().unwrap()
        );
        let mut ipv6_only = peers[0].clone();
        ipv6_only.tailnet_address.ipv4 = "0.0.0.0/32".parse().unwrap();
        assert_eq!(
            peer_address(&ipv6_only),
            "fd7a:115c:a1e0::1".parse::<IpAddr>().unwrap()
        );
    }

    #[tokio::test]
    async fn publishes_lifecycle_changes_and_rejects_dials_until_running() {
        let service = EmbeddedTailnet::new(TailscaleConfig {
            connect_timeout_ms: 100,
            ..TailscaleConfig::default()
        })
        .unwrap();
        let mut statuses = service.subscribe();
        assert_eq!(
            statuses.borrow_and_update().state,
            TailnetState::Unconfigured
        );
        assert!(matches!(
            service.dial_tcp("100.64.0.1", 22, None).await,
            Err(TailscaleError::NotConnected)
        ));

        service.set_configured(true);
        assert!(statuses.has_changed().unwrap());
        assert_eq!(
            statuses.borrow_and_update().state,
            TailnetState::Disconnected
        );
        assert!(matches!(
            service.dial_tcp("100.64.0.1", 22, None).await,
            Err(TailscaleError::NotConnected)
        ));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let starting = service
            .connect(ConnectRequest {
                key_state_json: generate_key_state().unwrap(),
                auth_key: None,
                control_url: Some(format!("http://{}", listener.local_addr().unwrap())),
                hostname: Some("rove-test".into()),
            })
            .await
            .unwrap();
        assert_eq!(starting.state, TailnetState::Starting);
        assert_eq!(statuses.borrow_and_update().state, TailnetState::Starting);

        let failed = timeout(
            Duration::from_secs(5),
            statuses.wait_for(|status| status.state == TailnetState::Error),
        )
        .await
        .unwrap()
        .unwrap()
        .clone();
        assert!(failed.configured);
        assert!(failed.message.is_some());
        assert!(failed.auth_url.is_none());

        let disconnected = service.disconnect(true).await.unwrap();
        assert_eq!(disconnected.state, TailnetState::Disconnected);
        let logged_out = service.disconnect(false).await.unwrap();
        assert_eq!(logged_out.state, TailnetState::Unconfigured);
        assert!(service.snapshot().await.unwrap().peers.is_empty());
    }
}
