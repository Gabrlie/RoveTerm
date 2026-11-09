//! Port forwarding over shared connections: local (`-L`), remote (`-R`) and dynamic SOCKS5 (`-D`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use rove_kernel::{Event, EventBus};
use rove_vault::{ForwardId, ForwardSpec, HostId};
use russh::client::Msg;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::socks::{self, Reply};
use crate::{SshConnection, SshError};

/// How often a running forward checks its connection and publishes changed counters.
const TICK: Duration = Duration::from_secs(1);

/// Lifecycle of a forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardState {
    /// Listening and forwarding.
    Running,
    /// Stopped by [`Forwards::stop`].
    Stopped,
    /// Ended because the connection closed.
    Failed,
}

/// Live event: a forward started, stopped, failed or its counters changed. Also the row type of
/// [`Forwards::list`].
#[derive(Debug, Clone, Serialize)]
pub struct ForwardStatus {
    /// Rule identifier.
    pub id: ForwardId,
    /// Host whose connection carries the forward.
    pub host_id: HostId,
    /// What is forwarded.
    pub spec: ForwardSpec,
    /// Current state.
    pub state: ForwardState,
    /// The port actually listened on (local port, or the server port for remote forwards).
    pub bound_port: u16,
    /// Connections currently open.
    pub active: u32,
    /// Connections handled since start.
    pub total: u64,
    /// Why the forward failed.
    pub error: Option<String>,
}
impl Event for ForwardStatus {}

/// Connection counters shared by a forward's tasks.
#[derive(Default)]
pub(crate) struct Counters {
    active: AtomicU32,
    total: AtomicU64,
}

impl Counters {
    fn open(self: &Arc<Self>) -> CounterGuard {
        self.active.fetch_add(1, Ordering::SeqCst);
        self.total.fetch_add(1, Ordering::SeqCst);
        CounterGuard(self.clone())
    }

    fn snapshot(&self) -> (u32, u64) {
        (
            self.active.load(Ordering::SeqCst),
            self.total.load(Ordering::SeqCst),
        )
    }
}

struct CounterGuard(Arc<Counters>);

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Where connections arriving on a remote forward go.
#[derive(Clone)]
pub(crate) struct RemoteRoute {
    dest_host: String,
    dest_port: u16,
    counters: Arc<Counters>,
    cancel: CancellationToken,
}

impl RemoteRoute {
    /// Connects to the destination and copies bytes until either side closes.
    pub(crate) async fn serve(self, channel: russh::Channel<Msg>) {
        let _guard = self.counters.open();
        let stream = match TcpStream::connect((self.dest_host.as_str(), self.dest_port)).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::debug!(%error, dest = %self.dest_host, "remote forward destination unreachable");
                return;
            }
        };
        bridge(channel.into_stream(), stream, &self.cancel).await;
    }
}

/// Remote forward routes of one connection, keyed by the server-side `(address, port)`.
#[derive(Default)]
pub(crate) struct RemoteRoutes(Mutex<HashMap<(String, u32), RemoteRoute>>);

impl RemoteRoutes {
    fn insert(&self, address: &str, port: u32, route: RemoteRoute) {
        self.0.lock().insert((address.to_owned(), port), route);
    }

    fn remove(&self, address: &str, port: u32) {
        self.0.lock().remove(&(address.to_owned(), port));
    }

    /// Finds the route for a forwarded connection. Servers may report the bound address
    /// differently from the request, so a unique port match is accepted too.
    pub(crate) fn resolve(&self, address: &str, port: u32) -> Option<RemoteRoute> {
        let routes = self.0.lock();
        if let Some(route) = routes.get(&(address.to_owned(), port)) {
            return Some(route.clone());
        }
        let mut by_port = routes.iter().filter(|((_, bound), _)| *bound == port);
        match (by_port.next(), by_port.next()) {
            (Some((_, route)), None) => Some(route.clone()),
            _ => None,
        }
    }
}

async fn bridge<A, B>(mut a: A, mut b: B, cancel: &CancellationToken)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    tokio::select! {
        () = cancel.cancelled() => {}
        result = tokio::io::copy_bidirectional(&mut a, &mut b) => {
            if let Err(error) = result {
                tracing::debug!(%error, "forwarded stream ended with an error");
            }
        }
    }
}

struct Running {
    status: Arc<Mutex<ForwardStatus>>,
    cancel: CancellationToken,
}

/// Runs forwards, keyed by rule. Each running forward holds a channel lease, so its connection
/// stays open until the forward stops.
pub struct Forwards {
    events: EventBus,
    running: Arc<Mutex<HashMap<ForwardId, Running>>>,
}

impl Forwards {
    /// Creates the registry; status changes are emitted on `events`.
    pub fn new(events: EventBus) -> Self {
        Self {
            events,
            running: Arc::default(),
        }
    }

    /// Starts forwarding `spec` over `connection`. Starting a forward that is already running
    /// returns its status; a failed one is replaced.
    pub async fn start(
        &self,
        id: ForwardId,
        host_id: HostId,
        connection: Arc<SshConnection>,
        spec: ForwardSpec,
    ) -> Result<ForwardStatus, SshError> {
        if let Some(status) = self.status(id)
            && status.state == ForwardState::Running
        {
            return Ok(status);
        }
        let cancel = CancellationToken::new();
        let counters = Arc::new(Counters::default());
        let lease = connection.lease();
        let (bound_port, remote) = match &spec {
            ForwardSpec::Local {
                bind_host,
                bind_port,
                dest_host,
                dest_port,
            } => {
                let listener = bind(bind_host, *bind_port).await?;
                let port = listener
                    .local_addr()
                    .map(|addr| addr.port())
                    .unwrap_or(*bind_port);
                let target = Target::Fixed(dest_host.clone(), *dest_port);
                tokio::spawn(accept_loop(
                    listener,
                    connection.clone(),
                    target,
                    counters.clone(),
                    cancel.clone(),
                ));
                (port, None)
            }
            ForwardSpec::Dynamic {
                bind_host,
                bind_port,
            } => {
                let listener = bind(bind_host, *bind_port).await?;
                let port = listener
                    .local_addr()
                    .map(|addr| addr.port())
                    .unwrap_or(*bind_port);
                tokio::spawn(accept_loop(
                    listener,
                    connection.clone(),
                    Target::Socks,
                    counters.clone(),
                    cancel.clone(),
                ));
                (port, None)
            }
            ForwardSpec::Remote {
                bind_host,
                bind_port,
                dest_host,
                dest_port,
            } => {
                let allocated = connection
                    .request_remote_forward(bind_host, u32::from(*bind_port))
                    .await?;
                let port = if allocated == 0 {
                    u32::from(*bind_port)
                } else {
                    allocated
                };
                connection.remote_routes().insert(
                    bind_host,
                    port,
                    RemoteRoute {
                        dest_host: dest_host.clone(),
                        dest_port: *dest_port,
                        counters: counters.clone(),
                        cancel: cancel.clone(),
                    },
                );
                (
                    u16::try_from(port).unwrap_or(*bind_port),
                    Some((bind_host.clone(), port)),
                )
            }
        };
        let status = ForwardStatus {
            id,
            host_id,
            spec,
            state: ForwardState::Running,
            bound_port,
            active: 0,
            total: 0,
            error: None,
        };
        let shared = Arc::new(Mutex::new(status.clone()));
        self.running.lock().insert(
            id,
            Running {
                status: shared.clone(),
                cancel: cancel.clone(),
            },
        );
        self.events.emit(&status);
        tokio::spawn(supervise(Supervisor {
            connection,
            counters,
            cancel,
            status: shared,
            events: self.events.clone(),
            remote,
            _lease: lease,
        }));
        Ok(status)
    }

    /// Records that a forward could not start, for callers that start forwards without a user
    /// waiting on the result (auto-start). The entry stays `failed` until started or stopped.
    pub fn record_failure(
        &self,
        id: ForwardId,
        host_id: HostId,
        spec: ForwardSpec,
        error: &SshError,
    ) {
        let status = ForwardStatus {
            id,
            host_id,
            spec,
            state: ForwardState::Failed,
            bound_port: 0,
            active: 0,
            total: 0,
            error: Some(error.to_string()),
        };
        self.running.lock().insert(
            id,
            Running {
                status: Arc::new(Mutex::new(status.clone())),
                cancel: CancellationToken::new(),
            },
        );
        self.events.emit(&status);
    }

    /// Stops a forward and forgets it. Returns `false` when it is unknown.
    pub fn stop(&self, id: ForwardId) -> bool {
        let Some(running) = self.running.lock().remove(&id) else {
            return false;
        };
        running.cancel.cancel();
        let mut status = running.status.lock().clone();
        status.state = ForwardState::Stopped;
        status.active = 0;
        self.events.emit(&status);
        true
    }

    /// Stops every forward.
    pub fn stop_all(&self) {
        let ids: Vec<_> = self.running.lock().keys().copied().collect();
        for id in ids {
            self.stop(id);
        }
    }

    /// Returns the status of a running or failed forward.
    pub fn status(&self, id: ForwardId) -> Option<ForwardStatus> {
        self.running
            .lock()
            .get(&id)
            .map(|running| running.status.lock().clone())
    }

    /// Lists running and failed forwards.
    pub fn list(&self) -> Vec<ForwardStatus> {
        let mut rows: Vec<_> = self
            .running
            .lock()
            .values()
            .map(|running| running.status.lock().clone())
            .collect();
        rows.sort_by_key(|row| row.id);
        rows
    }
}

async fn bind(host: &str, port: u16) -> Result<TcpListener, SshError> {
    TcpListener::bind((host, port))
        .await
        .map_err(|error| SshError::ForwardBind(format!("{host}:{port}: {error}")))
}

#[derive(Clone)]
enum Target {
    Fixed(String, u16),
    Socks,
}

async fn accept_loop(
    listener: TcpListener,
    connection: Arc<SshConnection>,
    target: Target,
    counters: Arc<Counters>,
    cancel: CancellationToken,
) {
    loop {
        let accepted = tokio::select! {
            () = cancel.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let (stream, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::debug!(%error, "accepting a forwarded connection failed");
                continue;
            }
        };
        let connection = connection.clone();
        let target = target.clone();
        let counters = counters.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let _guard = counters.open();
            if let Err(error) = forward_one(stream, peer, &connection, target, &cancel).await {
                tracing::debug!(%error, %peer, "forwarded connection failed");
            }
        });
    }
}

async fn forward_one(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    connection: &Arc<SshConnection>,
    target: Target,
    cancel: &CancellationToken,
) -> Result<(), String> {
    if let Err(error) = stream.set_nodelay(true) {
        tracing::debug!(%error, "set_nodelay failed");
    }
    let socks = matches!(target, Target::Socks);
    let (host, port) = match target {
        Target::Fixed(host, port) => (host, port),
        Target::Socks => socks::accept(&mut stream)
            .await
            .map_err(|error| error.to_string())?,
    };
    let opened = connection
        .open_direct_tcpip(&host, port, &peer.ip().to_string(), peer.port())
        .await;
    let (channel, _lease) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            if socks {
                let _client_may_be_gone = socks::reply(&mut stream, Reply::HostUnreachable).await;
            }
            return Err(error.to_string());
        }
    };
    if socks {
        socks::reply(&mut stream, Reply::Succeeded)
            .await
            .map_err(|error| error.to_string())?;
    }
    bridge(channel.into_stream(), stream, cancel).await;
    Ok(())
}

struct Supervisor {
    connection: Arc<SshConnection>,
    counters: Arc<Counters>,
    cancel: CancellationToken,
    status: Arc<Mutex<ForwardStatus>>,
    events: EventBus,
    /// Server-side `(address, port)` of a remote forward.
    remote: Option<(String, u32)>,
    _lease: crate::connection::ChannelLease,
}

/// Publishes counter changes, detects a closed connection, and releases the forward's resources
/// once it stops. Emits only while running; [`Forwards::stop`] emits the final status itself.
async fn supervise(supervisor: Supervisor) {
    let mut ticker = tokio::time::interval(TICK);
    let mut last = (0, 0);
    loop {
        tokio::select! {
            () = supervisor.cancel.cancelled() => break,
            _ = ticker.tick() => {}
        }
        if supervisor.connection.is_closed() {
            let failed = {
                let mut status = supervisor.status.lock();
                status.state = ForwardState::Failed;
                status.active = 0;
                status.error = Some("connection closed".into());
                status.clone()
            };
            supervisor.cancel.cancel();
            supervisor.events.emit(&failed);
            break;
        }
        let counts = supervisor.counters.snapshot();
        if counts != last {
            last = counts;
            let snapshot = {
                let mut status = supervisor.status.lock();
                (status.active, status.total) = counts;
                status.clone()
            };
            supervisor.events.emit(&snapshot);
        }
    }
    if let Some((address, port)) = &supervisor.remote {
        supervisor.connection.remote_routes().remove(address, *port);
        if !supervisor.connection.is_closed()
            && let Err(error) = supervisor
                .connection
                .cancel_remote_forward(address, *port)
                .await
        {
            tracing::debug!(%error, "cancelling remote forward failed");
        }
    }
}
