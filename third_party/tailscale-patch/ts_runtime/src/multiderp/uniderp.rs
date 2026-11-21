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

        args.env
            .register(Some(Self::name(args.region_id)), &slf)
            .await?;

        Ok(Self {
            runner_state: runner,
            transport_id,
            home_derp_tx,
            task,
            env: args.env,
        })
    }

    async fn on_stop(
        &mut self,
        _: WeakActorRef<Self>,
        _: ActorStopReason,
    ) -> Result<(), Self::Error> {
        self.env
            .tell::<Multiderp, _>(
                None,
                SetRegionTransportId(self.runner_state.region_id, None),
            )
            .await?;

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

#[derive(Clone)]
struct Runner {
    region_id: RegionId,
    region: DerpRegion,
    transport_id: UnderlayTransportId,
    home_derp_rx: watch::Receiver<bool>,
    to_dataplane: Tx<FromUnderlay>,
    from_dataplane: Arc<Mutex<Rx<ToUnderlay>>>,
    peer_db: Arc<RwLock<Option<Arc<PeerDb>>>>,
    keys: NodeKeyPair,
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
                pending = self.wait_for_activity().await;
            }
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
                Ok(()) => {
                    retry = Duration::from_secs(1);
                    reconnect = false;
                }
                Err(error) => {
                    reconnect = true;
                    tracing::warn!(%error, retry_in_secs = retry.as_secs(), "DERP disconnected, retrying");
                    tokio::time::sleep(retry).await;
                    retry = (retry * 2).min(Duration::from_secs(15));
                }
            }
        }
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn wait_for_activity(&mut self) -> Option<(DynEndpoint, Vec<PacketMut>)> {
        tracing::trace!("waiting for packet activity or for this to become home derp");

        let mut from_dataplane = self.from_dataplane.lock().await;

        while !*self.home_derp_rx.borrow_and_update() {
            tokio::select! {
                changed = self.home_derp_rx.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                },

                from_net = from_dataplane.recv() => {
                    tracing::trace!("received packet to send");
                    return from_net;
                }
            }
        }

        None
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
    ) -> Result<(), ts_derp::Error> {
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
                            tracing::error!(parent: &span, "underlay receive channel closed");
                            break;
                        };
                    }
                },

                from_net = from_dataplane.recv() => {
                    last_activity = Instant::now();

                    let Some((ep, pkts)) = from_net else {
                        tracing::warn!(parent: &span, "transport queue closed");
                        return Ok(());
                    };

                    tracing::trace!(parent: &span, ?ep, packets = pkts.len(), "packets to derp server");

                    transport.send([(ep, pkts)]).await?;
                },

                _ = option_timeout(inactivity_timeout) => {
                    if !*self.home_derp_rx.borrow() {
                        tracing::trace!(parent: &span, "timed out and not home derp, closing derp conn");
                        return Ok(());
                    }
                },

                changed = self.home_derp_rx.changed() => {
                    if changed.is_err() {
                        return Ok(());
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
