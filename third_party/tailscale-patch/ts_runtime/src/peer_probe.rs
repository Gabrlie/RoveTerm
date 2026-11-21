use std::{net::SocketAddr, time::Duration};

use kameo::{
    actor::{ActorRef, Spawn},
    message::{Context, Message},
};
use tokio::{
    sync::mpsc,
    time::{Instant, timeout_at},
};
use ts_control::StableNodeId;
use ts_disco_protocol::{CallMeMaybe, Ping, Pong};
use ts_keys::{DiscoPublicKey, NodePublicKey};
use ts_transport::{DynEndpoint, UnderlayTransportId};

use crate::{
    Env, Error,
    control_runner::{ControlRunner, ProbeConfig},
    dataplane::IncomingDiscoMsg,
    disco::{Disco, SendDisco},
    multiderp::{Multiderp, ProbeTransportMap},
    peer_tracker::{PeerByStableId, PeerTracker},
};

/// The ID and display names of a DERP region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerpRegion {
    pub id: u32,
    pub code: String,
    pub name: String,
}

/// The path on which the peer replied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbePath {
    Direct(SocketAddr),
    Relay(DerpRegion),
    Unknown,
}

/// Outcome of an on-demand Disco probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    Reachable { latency: Duration, path: ProbePath },
    Unreachable(&'static str),
}

/// Runs a probe independently of the runtime actor's mailbox.
pub struct PeerProbe(pub(crate) Env);

impl PeerProbe {
    pub async fn ping(self, stable_id: &str, timeout: Duration) -> Result<ProbeResult, Error> {
        let deadline = Instant::now() + timeout;
        timeout_at(
            deadline,
            self.run(StableNodeId(stable_id.to_owned()), deadline),
        )
        .await
        .unwrap_or(Ok(ProbeResult::Unreachable("peer probe timed out")))
    }

    async fn run(self, stable_id: StableNodeId, deadline: Instant) -> Result<ProbeResult, Error> {
        let env = self.0;
        let Some(peer) = env
            .ask::<PeerTracker, _>(None, PeerByStableId { stable_id }, true)
            .await?
        else {
            return Ok(ProbeResult::Unreachable("peer was not found"));
        };
        let Some(disco_key) = peer.disco_key else {
            return Ok(ProbeResult::Unreachable("peer has no discovery key"));
        };
        let (endpoints, regions) = env.ask::<ControlRunner, _>(None, ProbeConfig, true).await?;
        let (transports,) = env
            .ask::<Multiderp, _>(None, ProbeTransportMap, true)
            .await?;
        let (replies, mut rx) = mpsc::channel(2);
        let tx_id = rand::random();
        let actor = ProbeGuard(ProbeListener::spawn((
            env.clone(),
            ProbeListener {
                disco_key,
                node_key: peer.node_key,
                tx_id,
                replies,
                direct_seen: false,
                relay_seen: false,
            },
        )));
        use crate::error::ResultExt;
        actor
            .0
            .wait_for_startup_result()
            .await
            .with_actor_info(&actor.0)?;

        let relay = DynEndpoint::derp(peer.node_key);
        if !endpoints.is_empty() {
            let buf = Disco::mk_pkt::<CallMeMaybe>(
                CallMeMaybe::size_for_endpoint_count(endpoints.len()),
                &env.keys.disco_keys,
                &disco_key,
                |message| {
                    for (target, endpoint) in message.endpoints.iter_mut().zip(&endpoints) {
                        *target = endpoint.endpoint.into();
                    }
                },
            );
            env.publish_noretain(SendDisco {
                buf,
                ep: relay.clone(),
            })
            .await?;
        }

        let ping = Disco::mk_pkt::<Ping>(
            Ping::size_with_padding(0),
            &env.keys.disco_keys,
            &disco_key,
            |ping| {
                ping.node_key = env.keys.node_keys.public;
                ping.tx_id = tx_id;
            },
        );
        let sent_at = Instant::now();
        for ep in peer
            .underlay_addresses
            .into_iter()
            .map(DynEndpoint::udp)
            .chain([relay])
        {
            env.publish_noretain(SendDisco {
                buf: ping.clone(),
                ep,
            })
            .await?;
        }

        let Ok(Some((path, received_at))) = timeout_at(deadline, rx.recv()).await else {
            return Ok(ProbeResult::Unreachable("peer did not reply"));
        };
        let latency = received_at - sent_at;
        let path = match path {
            ReplyPath::Direct(endpoint) => ProbePath::Direct(endpoint),
            ReplyPath::Relay(transport) => {
                let grace = deadline.min(received_at + Duration::from_millis(450));
                if let Ok(Some((ReplyPath::Direct(endpoint), received_at))) =
                    timeout_at(grace, rx.recv()).await
                {
                    return Ok(ProbeResult::Reachable {
                        latency: received_at - sent_at,
                        path: ProbePath::Direct(endpoint),
                    });
                }
                transports
                    .iter()
                    .find_map(|(region, id)| (*id == transport).then_some(region))
                    .and_then(|id| regions.get(id).cloned())
                    .map_or(ProbePath::Unknown, ProbePath::Relay)
            }
        };
        Ok(ProbeResult::Reachable { latency, path })
    }
}

struct ProbeGuard(ActorRef<ProbeListener>);

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        self.0.kill();
    }
}

enum ReplyPath {
    Direct(SocketAddr),
    Relay(UnderlayTransportId),
}

struct ProbeListener {
    disco_key: DiscoPublicKey,
    node_key: NodePublicKey,
    tx_id: [u8; 12],
    replies: mpsc::Sender<(ReplyPath, Instant)>,
    direct_seen: bool,
    relay_seen: bool,
}

impl kameo::Actor for ProbeListener {
    type Args = (Env, Self);
    type Error = Error;

    async fn on_start((env, listener): Self::Args, slf: ActorRef<Self>) -> Result<Self, Error> {
        env.subscribe::<IncomingDiscoMsg>(&slf).await?;
        Ok(listener)
    }
}

impl Message<IncomingDiscoMsg> for ProbeListener {
    type Reply = ();

    async fn handle(&mut self, message: IncomingDiscoMsg, _: &mut Context<Self, Self::Reply>) {
        let packet = message.packet.get();
        if packet.sender_pubkey() != &self.disco_key
            || !packet
                .as_msg::<Pong>()
                .is_some_and(|pong| pong.tx_id == self.tx_id)
        {
            return;
        }
        let path = if let Some(endpoint) = message.sender.as_udp() {
            if std::mem::replace(&mut self.direct_seen, true) {
                return;
            }
            ReplyPath::Direct(endpoint)
        } else if message.sender.as_derp() == Some(self.node_key) {
            if std::mem::replace(&mut self.relay_seen, true) {
                return;
            }
            ReplyPath::Relay(message.transport)
        } else {
            return;
        };
        // A dropped receiver means the caller has already finished or cancelled the probe.
        let _ = self.replies.try_send((path, Instant::now()));
    }
}
