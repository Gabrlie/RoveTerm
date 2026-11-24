use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use futures::FutureExt;
use kameo::{
    actor::{ActorRef, Spawn, WeakActorRef},
    error::ActorStopReason,
    message::{Context, Message},
};
use smol_str::SmolStr;
use tokio::sync::{Mutex, watch};
use ts_control::DerpRegion;
use ts_dataplane::async_tokio::{FromUnderlay, Rx, ToUnderlay, Tx};
use ts_derp::{DefaultClient, RegionId, frame::NotePreferred};
use ts_http_util::Client as _;
use ts_keys::NodeKeyPair;
use ts_packet::PacketMut;
use ts_transport::{BatchRecvIter, DynEndpoint, UnderlayTransport, UnderlayTransportId};

use crate::{
    Task,
    dataplane::{DataplaneActor, NewUnderlayTransport},
    derp_latency::DerpLatencyMeasurement,
    env::Env,
    multiderp::{Multiderp, SetRegionTransportId},
    netmon,
    peer_tracker::{PeerDb, PeerState},
    task::ErasedTask,
};

#[derive(Clone)]
pub struct Args {
    pub region_id: RegionId,
    pub region: DerpRegion,
    pub env: Env,
}

/// Single-region derp client.
pub struct Uniderp {
    /// Current state to spawn a runner task.
    ///
    /// Retained here because we may need to kill and restart the runner if we get updated region
    /// info.
    runner_state: Runner,

    /// The transport id this region is responsible for.
    transport_id: UnderlayTransportId,

    /// The task runner handling the transport for this region.
    task: ActorRef<ErasedTask>,

    home_derp_tx: watch::Sender<bool>,
    net_gen_tx: watch::Sender<u64>,
    network: Option<netmon::NetworkFingerprint>,

    env: Env,
}

impl Uniderp {
    pub fn name(region_id: RegionId) -> SmolStr {
        smol_str::format_smolstr!("derp_region:{region_id}")
    }
}

#[kameo::messages]
impl Uniderp {
    #[message]
    pub fn transport_id(&self) -> Option<UnderlayTransportId> {
        Some(self.transport_id)
    }
}

async fn start_runner(mut runner: Runner) {
    runner.run().await;
}

impl kameo::Actor for Uniderp {
    type Args = Args;
    type Error = crate::Error;

    async fn on_start(args: Self::Args, slf: ActorRef<Self>) -> Result<Self, Self::Error> {
        let (home_derp_tx, home_derp_rx) = watch::channel(false);
        let (net_gen_tx, net_gen_rx) = watch::channel(0);

        let (transport_id, from_dataplane, to_dataplane) = args
            .env
            .ask::<DataplaneActor, _>(Option::<SmolStr>::None, NewUnderlayTransport, true)
            .await?;

        args.env
            .ask::<Multiderp, _>(
                None,
                SetRegionTransportId(args.region_id, Some(transport_id)),
                true,
            )
            .await?;

        let runner = Runner {
            region_id: args.region_id,
            region: args.region,
            transport_id,
            peer_db: Arc::new(RwLock::new(None)),
            home_derp_rx,
            net_gen_rx,
            to_dataplane,
            keys: args.env.keys.node_keys.clone(),
            from_dataplane: Arc::new(Mutex::new(from_dataplane)),
        };

        let task = Task::spawn_link(&slf, start_runner(runner.clone()).boxed()).await;

        args.env
            .subscribe::<Arc<ts_control::StateUpdate>>(&slf)
            .await?;
        args.env.subscribe::<Arc<PeerState>>(&slf).await?;
        args.env.subscribe::<DerpLatencyMeasurement>(&slf).await?;
        args.env.subscribe::<Arc<netmon::State>>(&slf).await?;

        args.env
            .register(Some(Self::name(args.region_id)), &slf)
            .await?;

        Ok(Self {
            runner_state: runner,
            transport_id,
            home_derp_tx,
            net_gen_tx,
            network: None,
            task,
            env: args.env,
        })
    }

    async fn on_stop(
        &mut self,
        _: WeakActorRef<Self>,
        _: ActorStopReason,
    ) -> Result<(), Self::Error> {
        // Multiderp may have already stopped during shutdown.
        let _ = self
            .env
            .tell::<Multiderp, _>(
                None,
                SetRegionTransportId(self.runner_state.region_id, None),
            )
            .await;

        Ok(())
    }
}

impl Message<Arc<ts_control::StateUpdate>> for Uniderp {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: Arc<ts_control::StateUpdate>,
        ctx: &mut Context<Self, Self::Reply>,
    ) {
        let Some(derp) = &msg.derp else {
            return;
        };

        let Some(region) = derp.get(&self.runner_state.region_id) else {
            tracing::debug!(
                region_id = ?self.runner_state.region_id,
                "derp region disappeared from map, stopping"
            );
            ctx.stop();
            return;
        };

        if &self.runner_state.region == region {
            return;
        }

        tracing::debug!(id = %self.runner_state.region_id, "region changed, restarting task");
        self.runner_state.region = region.clone();

        self.task.unlink(ctx.actor_ref()).await;
        self.task.kill();
        self.task
            .wait_for_shutdown_with_result(|e| {
                if let Err(e) = e {
                    tracing::error!(error = ?e, "shutting down derp task on region update");
                }
            })
            .await;

        self.task = Task::spawn_link(
            ctx.actor_ref(),
            start_runner(self.runner_state.clone()).boxed(),
        )
        .await;
    }
}

impl Message<Arc<PeerState>> for Uniderp {
    type Reply = ();

    async fn handle(&mut self, msg: Arc<PeerState>, _ctx: &mut Context<Self, Self::Reply>) {
        let mut db = self.runner_state.peer_db.write().unwrap();
        *db = Some(msg.peers.clone());
    }
}

impl Message<DerpLatencyMeasurement> for Uniderp {
    type Reply = ();

    async fn handle(&mut self, msg: DerpLatencyMeasurement, _ctx: &mut Context<Self, Self::Reply>) {
        let Some(result) = msg.measurement.as_ref().first() else {
            tracing::trace!("received home derp measurement message but none was set");
            return;
        };

        self.home_derp_tx.send_if_modified(|x| {
            let new_val = result.id == self.runner_state.region_id;
            let changed = new_val != *x;
            *x = new_val;

            changed
        });
    }
}

impl Message<Arc<netmon::State>> for Uniderp {
    type Reply = ();

    async fn handle(&mut self, state: Arc<netmon::State>, _ctx: &mut Context<Self, Self::Reply>) {
        let next = state.fingerprint();
        let changed = self
            .network
            .as_ref()
            .is_some_and(|previous| previous != &next);
        self.network = Some(next);
        if changed {
            self.net_gen_tx
                .send_modify(|generation| *generation = generation.wrapping_add(1));
        }
    }
}

#[derive(Clone)]
struct Runner {
    region_id: RegionId,
    region: DerpRegion,
    transport_id: UnderlayTransportId,
    home_derp_rx: watch::Receiver<bool>,
    net_gen_rx: watch::Receiver<u64>,
    to_dataplane: Tx<FromUnderlay>,
    from_dataplane: Arc<Mutex<Rx<ToUnderlay>>>,
    peer_db: Arc<RwLock<Option<Arc<PeerDb>>>>,
    keys: NodeKeyPair,
}

enum Activity {
    Packet(DynEndpoint, Vec<PacketMut>),
    HomeDerp,
    Closed,
}

enum TransportExit {
    Idle,
    Rebind,
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
enum Retry {
    Reconnect,
    Rebind,
    Wait,
    Closed,
}

impl Runner {
    const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(10);

    #[tracing::instrument(skip_all, fields(region_id = %self.region_id))]
    async fn run(&mut self) {
        let mut retry = Duration::from_secs(1);
        let mut pending = None;
        let mut reconnect = false;
        loop {
            if !reconnect {
                pending = match self.wait_for_activity().await {
                    Activity::Packet(ep, packets) => Some((ep, packets)),
                    Activity::HomeDerp => None,
                    Activity::Closed => break,
                };
            }
            self.net_gen_rx.borrow_and_update();
            let result = match self.connect().await {
                Ok(transport) => {
                    let connected = Instant::now();
                    let result = self.run_transport(transport, pending.take()).await;
                    if connected.elapsed() >= Self::INACTIVITY_TIMEOUT {
                        retry = Duration::from_secs(1);
                    }
                    result
                }
                Err(error) => Err(error),
            };

            match result {
                Ok(TransportExit::Closed) => break,
                Ok(TransportExit::Idle) => reconnect = false,
                Ok(TransportExit::Rebind) => reconnect = *self.home_derp_rx.borrow_and_update(),
                Err(error) => {
                    let home = *self.home_derp_rx.borrow_and_update();
                    if home {
                        tracing::warn!(%error, retry_in_secs = retry.as_secs(), "DERP disconnected, retrying");
                    } else {
                        tracing::debug!(%error, "DERP disconnected, waiting for traffic");
                    }
                    match self.retry_after_error(home, retry).await {
                        Retry::Reconnect => {
                            reconnect = true;
                            retry = (retry * 2).min(Duration::from_secs(15));
                            continue;
                        }
                        Retry::Rebind => reconnect = true,
                        Retry::Wait => reconnect = false,
                        Retry::Closed => break,
                    }
                }
            }
            retry = Duration::from_secs(1);
        }
    }

    async fn retry_after_error(&mut self, home: bool, retry: Duration) -> Retry {
        if self.home_derp_rx.has_changed().is_err()
            || self.from_dataplane.lock().await.is_closed()
            || self.to_dataplane.is_closed()
        {
            return Retry::Closed;
        }
        if home {
            tokio::select! {
                biased;
                changed = self.home_derp_rx.changed() => {
                    if changed.is_err() {
                        return Retry::Closed;
                    }
                }
                _ = self.to_dataplane.closed() => return Retry::Closed,
                changed = self.net_gen_rx.changed() => {
                    return if changed.is_ok() { Retry::Rebind } else { Retry::Closed };
                }
                _ = tokio::time::sleep(retry) => return Retry::Reconnect,
            }
        }
        Retry::Wait
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn wait_for_activity(&mut self) -> Activity {
        tracing::trace!("waiting for packet activity or for this to become home derp");

        let mut from_dataplane = self.from_dataplane.lock().await;

        loop {
            if self.home_derp_rx.has_changed().is_err()
                || self.to_dataplane.is_closed()
                || from_dataplane.is_closed()
            {
                return Activity::Closed;
            }
            if *self.home_derp_rx.borrow_and_update() {
                return Activity::HomeDerp;
            }
            tokio::select! {
                changed = self.home_derp_rx.changed() => {
                    if changed.is_err() {
                        return Activity::Closed;
                    }
                }
                _ = self.to_dataplane.closed() => return Activity::Closed,
                from_net = from_dataplane.recv() => {
                    return match from_net {
                        Some((ep, packets)) => Activity::Packet(ep, packets),
                        None => Activity::Closed,
                    };
                }
            }
        }
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn connect(&self) -> Result<DefaultClient, ts_derp::Error> {
        tokio::time::timeout(Duration::from_secs(5), async {
            // Use the fallible TLS dial directly because ts_derp::client::connect unwraps it.
            let Some((conn, _, addr)) = ts_derp::dial::dial_region_tls(&self.region.servers)
                .await
                .map_err(std::io::Error::other)?
            else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "no reachable DERP server",
                )
                .into());
            };
            let url = url::Url::parse(&format!("https://{addr}/derp"))?;
            let client = ts_http_util::http1::connect(conn).await?;
            let response = client
                .send(ts_http_util::make_upgrade_req(&url, "DERP", None)?)
                .await?;
            let upgraded = ts_http_util::do_upgrade(response)
                .await
                .map_err(std::io::Error::other)?;
            DefaultClient::handshake(upgraded, &self.keys).await
        })
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "DERP connection timed out")
        })?
    }

    #[tracing::instrument(skip_all, fields(transport_id = ?self.transport_id), level = "trace")]
    async fn run_transport(
        &mut self,
        transport: DefaultClient,
        pending: Option<(DynEndpoint, Vec<PacketMut>)>,
    ) -> Result<TransportExit, ts_derp::Error> {
        let mut net_gen_rx = self.net_gen_rx.clone();
        tokio::select! {
            biased;
            changed = net_gen_rx.changed() => {
                Ok(if changed.is_ok() { TransportExit::Rebind } else { TransportExit::Closed })
            }
            result = self.forward_packets(transport, pending) => result,
        }
    }

    async fn forward_packets(
        &mut self,
        transport: DefaultClient,
        pending: Option<(DynEndpoint, Vec<PacketMut>)>,
    ) -> Result<TransportExit, ts_derp::Error> {
        let is_home = u8::from(*self.home_derp_rx.borrow_and_update());
        transport.send_frame(&NotePreferred { is_home }).await?;
        if let Some(pending) = pending {
            transport.send([pending]).await?;
        }
        let mut last_activity = Instant::now();
        let mut from_dataplane = self.from_dataplane.lock().await;

        loop {
            let span = tracing::trace_span!("derp_loop");

            let inactivity_timeout =
                (!*self.home_derp_rx.borrow()).then(|| last_activity + Self::INACTIVITY_TIMEOUT);

            tokio::select! {
                from_derp = transport.recv() => {
                    last_activity = Instant::now();

                    for ret in from_derp.batch_iter() {
                        let (ep, pkts) = ret?;
                        let pkts = pkts.into_iter().collect::<Vec<_>>();

                        tracing::trace!(parent: &span, ?ep, len = pkts.len(), "packets from derp server");

                        let Ok(()) = self.to_dataplane.send((self.transport_id, ep, pkts)) else {
                            return Ok(TransportExit::Closed);
                        };
                    }
                },

                from_net = from_dataplane.recv() => {
                    last_activity = Instant::now();

                    let Some((ep, pkts)) = from_net else {
                        return Ok(TransportExit::Closed);
                    };

                    tracing::trace!(parent: &span, ?ep, packets = pkts.len(), "packets to derp server");

                    transport.send([(ep, pkts)]).await?;
                },

                _ = option_timeout(inactivity_timeout) => {
                    if !*self.home_derp_rx.borrow() {
                        tracing::trace!(parent: &span, "timed out and not home derp, closing derp conn");
                        return Ok(TransportExit::Idle);
                    }
                },

                _ = self.to_dataplane.closed() => return Ok(TransportExit::Closed),

                changed = self.home_derp_rx.changed() => {
                    if changed.is_err() {
                        return Ok(TransportExit::Closed);
                    }
                    let is_home = u8::from(*self.home_derp_rx.borrow_and_update());
                    transport.send_frame(&NotePreferred { is_home }).await?;
                    tracing::trace!(is_home, "updated DERP preference");
                },
            }
        }
    }
}

async fn option_timeout(duration: Option<Instant>) {
    match duration {
        Some(dur) => tokio::time::sleep_until(dur.into()).await,
        None => core::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn runner(
        home: bool,
    ) -> (
        Runner,
        watch::Sender<bool>,
        watch::Sender<u64>,
        Tx<ToUnderlay>,
        Rx<FromUnderlay>,
    ) {
        let (home_tx, home_rx) = watch::channel(home);
        let (net_gen_tx, net_gen_rx) = watch::channel(0);
        let (from_tx, from_rx) = mpsc::unbounded_channel();
        let (to_tx, to_rx) = mpsc::unbounded_channel();
        let runner = Runner {
            region_id: RegionId(std::num::NonZeroU32::MIN),
            region: DerpRegion {
                info: ts_derp::RegionInfo {
                    name: "Test".into(),
                    code: "test".into(),
                    no_measure_no_home: false,
                },
                servers: Vec::new(),
            },
            transport_id: UnderlayTransportId(1),
            home_derp_rx: home_rx,
            net_gen_rx,
            to_dataplane: to_tx,
            from_dataplane: Arc::new(Mutex::new(from_rx)),
            peer_db: Arc::new(RwLock::new(None)),
            keys: NodeKeyPair::random(),
        };
        (runner, home_tx, net_gen_tx, from_tx, to_rx)
    }

    #[tokio::test]
    async fn exits_when_dataplane_queue_closes() {
        for home in [false, true] {
            let (mut runner, _home, _net_gen, from, _to) = runner(home);
            tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(runner.run(), async { drop(from) });
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn network_change_interrupts_home_derp_backoff() {
        let (mut runner, _home, net_gen, _from, _to) = runner(true);
        net_gen.send(1).unwrap();
        let retry = tokio::time::timeout(
            Duration::from_secs(1),
            runner.retry_after_error(true, Duration::from_secs(30)),
        )
        .await
        .unwrap();
        assert_eq!(retry, Retry::Rebind);
    }

    #[tokio::test]
    async fn non_home_failure_waits_for_new_activity() {
        let (mut runner, home, _net_gen, from, _to) = runner(false);
        assert!(runner.connect().await.is_err());
        assert_eq!(
            runner
                .retry_after_error(false, Duration::from_secs(1))
                .await,
            Retry::Wait
        );
        tokio::select! {
            biased;
            _ = runner.wait_for_activity() => panic!("idle region has no new activity"),
            _ = std::future::ready(()) => {}
        }
        from.send((
            DynEndpoint::udp("127.0.0.1:12345".parse().unwrap()),
            Vec::new(),
        ))
        .unwrap();
        assert!(matches!(
            runner.wait_for_activity().await,
            Activity::Packet(..)
        ));

        home.send(true).unwrap();
        assert!(matches!(
            runner.wait_for_activity().await,
            Activity::HomeDerp
        ));
        let (retry, ()) = tokio::join!(
            runner.retry_after_error(true, Duration::from_secs(30)),
            async { home.send(false).unwrap() },
        );
        assert_eq!(retry, Retry::Wait);
        drop(home);
        assert!(matches!(runner.wait_for_activity().await, Activity::Closed));
    }
}
