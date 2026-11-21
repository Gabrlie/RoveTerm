#![doc = include_str!("../README.md")]

extern crate ts_disco_protocol as disco;

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use ts_bart::RoutingTable;
use ts_packet::PacketMut;
use ts_packetfilter::{FilterExt, IpProto};
use ts_time::{Handle, Scheduler, TimeRange};
use ts_transport::PeerId;
use ts_tunnel::{Endpoint, NodeKeyPair};

pub mod async_tokio;
pub mod overlay_router;
mod packet_ident;
pub mod underlay_router;

pub use packet_ident::{PacketIdent, PacketType};

/// Duration without traffic after which to remove a peer from the active map.
const PEER_EXPIRATION: Duration = Duration::from_secs(45);

const FLOW_CAPACITY: usize = 4096;
const FLOW_EXPIRATION: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Flow {
    source: (IpAddr, u16),
    destination: (IpAddr, u16),
    protocol: u8,
}

impl Flow {
    fn reverse(self) -> Self {
        Self {
            source: self.destination,
            destination: self.source,
            protocol: self.protocol,
        }
    }
}

struct PacketFlow {
    info: ts_packetfilter::PacketInfo,
    flow: Option<Flow>,
    initial_syn: bool,
}

impl PacketFlow {
    fn parse(packet: &[u8]) -> Option<Self> {
        use etherparse::{NetSlice, SlicedPacket, TransportSlice};

        let packet = SlicedPacket::from_ip(packet).ok()?;
        let (protocol, src, dst) = match packet.net? {
            NetSlice::Ipv4(ip) => (
                ip.payload().ip_number.0,
                IpAddr::from(ip.header().source_addr()),
                IpAddr::from(ip.header().destination_addr()),
            ),
            NetSlice::Ipv6(ip) => (
                ip.payload().ip_number.0,
                IpAddr::from(ip.header().source_addr()),
                IpAddr::from(ip.header().destination_addr()),
            ),
            _ => return None,
        };

        let (source_port, destination_port, initial_syn, tracked) = match packet.transport {
            Some(TransportSlice::Tcp(ref tcp)) => (
                tcp.source_port(),
                tcp.destination_port(),
                tcp.syn() && !tcp.ack(),
                true,
            ),
            Some(TransportSlice::Udp(ref udp)) => {
                (udp.source_port(), udp.destination_port(), false, true)
            }
            _ => (0, 0, false, false),
        };
        Some(Self {
            info: ts_packetfilter::PacketInfo {
                ip_proto: IpProto::new(protocol.into()),
                port: destination_port,
                src,
                dst,
            },
            flow: tracked.then_some(Flow {
                source: (src, source_port),
                destination: (dst, destination_port),
                protocol,
            }),
            initial_syn,
        })
    }
}

/// A data plane subsystem that can be the subject of timer events.
pub enum Subsystem {
    /// The wireguard component.
    Wireguard,

    /// Peer activity garbage collection.
    PeerGc,
}

/// Transforms packets to make tailscale happen.
pub struct DataPlane {
    /// Wireguard encryption/decryption.
    pub wireguard: Endpoint,

    /// Outbound overlay router.
    pub or_out: overlay_router::outbound::Router,
    /// Outbound underlay router.
    pub ur_out: underlay_router::outbound::Router,

    /// Inbound source filter.
    pub src_filter_in: Arc<ts_bart::Table<PeerId>>,
    /// Inbound overlay router.
    pub or_in: overlay_router::inbound::Router,

    /// The packet filter.
    pub packet_filter: Arc<dyn ts_packetfilter::Filter + Send + Sync>,

    /// Per-peer timestamps of last outgoing data packets.
    ///
    /// Used to determine whether we should be running path discovery for this peer.
    pub active_peers: HashMap<PeerId, Instant>,

    outbound_flows: HashMap<(PeerId, Flow), Instant>,

    /// Events queued for future processing.
    pub events: Scheduler<Subsystem>,

    /// Next event for the wireguard subsystem.
    pub wg_next: Option<Handle<Subsystem>>,

    /// Next event for the peer gc subsystem.
    pub peer_gc_next: Option<Handle<Subsystem>>,
}

impl DataPlane {
    /// Creates a new data plane for a wireguard node key.
    pub fn new(my_key: NodeKeyPair) -> Self {
        DataPlane {
            wireguard: Endpoint::new(my_key),
            or_out: Default::default(),
            ur_out: Default::default(),
            src_filter_in: Default::default(),
            or_in: Default::default(),
            events: Default::default(),
            packet_filter: Arc::new(ts_packetfilter::DropAllFilter),
            active_peers: Default::default(),
            outbound_flows: Default::default(),
            wg_next: None,
            peer_gc_next: None,
        }
    }

    /// Pad `packets` to a 16-byte boundary, as required by `ts_tunnel`.
    fn pad_packets(packets: &mut Vec<PacketMut>) {
        const PAD_ALIGN: usize = 16;
        for p in packets {
            let m = p.len() % PAD_ALIGN;
            if m != 0 {
                p.grow_back(PAD_ALIGN - m);
            }
        }
    }

    /// Remove padding from `packets`.
    ///
    /// Packets which do not superficially match the structure of an IPv4 or IPv6 packet are
    /// removed from `packets`.
    fn unpad_packets(packets: &mut Vec<PacketMut>) {
        packets.retain_mut(|packet| {
            if let Some(sz) = packet.get_ip_len() {
                packet.resize(sz);
                true
            } else {
                false
            }
        });
    }

    /// Processes packets originating from the local device.
    #[tracing::instrument(skip_all, fields(n_packets = packets.len()))]
    pub fn process_outbound(&mut self, packets: Vec<PacketMut>) -> OutboundResult {
        let overlay_router::outbound::Result {
            to_wireguard,
            loopback,
        } = self.or_out.route(packets);

        let now = Instant::now();

        let to_wireguard = to_wireguard
            .into_iter()
            .map(|(id, mut packets)| {
                self.active_peers.insert(id, now);
                for packet in &packets {
                    self.record_outbound_flow(id, packet.as_ref(), now);
                }
                Self::pad_packets(&mut packets);
                (ts_tunnel::PeerId(id.0), packets)
            })
            .collect::<Vec<_>>();

        let ts_tunnel::SendResult {
            to_peers: encrypted,
        } = self.wireguard.send(Instant::now(), to_wireguard);

        let to_peers = self
            .ur_out
            .route(encrypted.into_iter().map(|(k, v)| (PeerId(k.0), v)));

        self.ensure_events(now);

        OutboundResult { to_peers, loopback }
    }

    /// Processes packets received from elsewhere.
    pub fn process_inbound(
        &mut self,
        packets: impl IntoIterator<Item = PacketMut>,
    ) -> InboundResult {
        let (wireguard, disco, stun) = packets.into_iter().fold(
            (vec![], vec![], vec![]),
            |(mut wg, mut disco, mut stun), pkt| {
                let ident = PacketIdent::identify(pkt.as_ref());

                match ident.ty {
                    PacketType::Disco => {
                        disco.push(pkt);
                    }
                    PacketType::StunBinding => {
                        stun.push(pkt);
                    }
                    PacketType::Wireguard | PacketType::Unknown => wg.push(pkt),
                }

                (wg, disco, stun)
            },
        );

        let ts_tunnel::RecvResult { to_local, to_peers } =
            self.wireguard.recv(Instant::now(), wireguard);

        let to_local = to_local
            .into_iter()
            .filter_map(
                |(peer_id, mut packets)| -> Option<(ts_tunnel::PeerId, Vec<PacketMut>)> {
                    Self::unpad_packets(&mut packets);
                    if packets.is_empty() {
                        None
                    } else {
                        Some((peer_id, packets))
                    }
                },
            )
            .map(|(peer_id, mut packets)| {
                let peer = PeerId(peer_id.0);
                let now = Instant::now();
                packets.retain(|packet| {
                    let src = packet.get_src_addr();
                    if src.and_then(|ip| self.src_filter_in.lookup(ip)) != Some(&peer) {
                        tracing::debug!(?peer, ?src, "inbound source filter dropped packet");
                        return false;
                    }
                    self.allows_inbound(peer, packet.as_ref(), now)
                });
                packets
            });

        let now = Instant::now();

        let to_local = self.or_in.route(to_local.flatten());

        let to_peers = to_peers
            .into_iter()
            .map(|(k, v)| (ts_transport::PeerId(k.0), v))
            .inspect(|(id, _)| {
                self.active_peers.insert(*id, now);
            });

        let to_peers = self.ur_out.route(to_peers);

        self.ensure_events(now);

        InboundResult {
            to_local,
            to_peers,
            disco,
            stun,
        }
    }

    fn prune_flows(&mut self, now: Instant) {
        self.outbound_flows
            .retain(|_, last_sent| now.duration_since(*last_sent) < FLOW_EXPIRATION);
    }

    fn record_outbound_flow(&mut self, peer: PeerId, packet: &[u8], now: Instant) {
        let Some(flow) = PacketFlow::parse(packet).and_then(|packet| packet.flow) else {
            return;
        };
        let key = (peer, flow);
        if let Some(last_sent) = self.outbound_flows.get_mut(&key) {
            *last_sent = now;
            return;
        }
        if self.outbound_flows.len() >= FLOW_CAPACITY {
            self.prune_flows(now);
            if self.outbound_flows.len() >= FLOW_CAPACITY
                && let Some((&oldest, _)) = self.outbound_flows.iter().min_by_key(|(_, sent)| *sent)
            {
                self.outbound_flows.remove(&oldest);
            }
        }
        self.outbound_flows.insert(key, now);
    }

    fn allows_inbound(&self, peer: PeerId, packet: &[u8], now: Instant) -> bool {
        let Some(PacketFlow {
            info,
            flow,
            initial_syn,
        }) = PacketFlow::parse(packet)
        else {
            tracing::debug!(?peer, "inbound packet filter dropped malformed packet");
            return false;
        };

        if !initial_syn
            && let Some(flow) = flow
            && self
                .outbound_flows
                .get(&(peer, flow.reverse()))
                .is_some_and(|last_sent| now.duration_since(*last_sent) < FLOW_EXPIRATION)
        {
            return true;
        }

        // TODO(npry): wire in nodecaps
        let allowed = self.packet_filter.can_access(&info, []);
        if !allowed {
            tracing::debug!(?peer, ?info, "inbound packet filter dropped packet");
        }
        allowed
    }

    /// Return the next time at which [`DataPlane::process_events`] must be called.
    ///
    /// [`DataPlane::process_outbound`], [`DataPlane::process_inbound`] and
    /// [`DataPlane::process_events`] may all update the next event time. Callers should prefer
    /// calling `next_event` as needed to get a correct result, rather than store the returned
    /// value.
    pub fn next_event(&self) -> Option<Instant> {
        self.events.next_dispatch()
    }

    /// Process all queued events that are due for processing.
    ///
    /// Must be called at least as often as dictated by [`DataPlane::next_event`] for the
    /// data plane to function correctly. It is harmless to call it more frequently.
    pub fn process_events(&mut self) -> underlay_router::outbound::Result {
        let mut to_peers = HashMap::new();
        let now = Instant::now();
        let mut should_gc = false;

        for event in self.events.dispatch(now) {
            match event {
                Subsystem::Wireguard => {
                    let res = self.wireguard.dispatch_events(now);
                    to_peers.extend(
                        res.to_peers
                            .into_iter()
                            .map(|(id, pkts)| (ts_transport::PeerId(id.0), pkts))
                            .inspect(|(id, _)| {
                                self.active_peers.insert(*id, now);
                            }),
                    );
                }
                Subsystem::PeerGc => should_gc = true,
            }
        }

        if should_gc {
            self.prune_flows(now);
            let _span =
                tracing::trace_span!("peer gc", n_active_peers_pre = self.active_peers.len())
                    .entered();

            self.active_peers.retain(|id, &mut last_traffic| {
                let elapsed = now - last_traffic;
                let keep = elapsed < PEER_EXPIRATION;

                if !keep {
                    tracing::trace!(peer_id = %id, ?elapsed, "peer expired");
                }

                keep
            });

            tracing::trace!(n_active_peers_post = self.active_peers.len());

            // remove so ensure_peer_gc sees there's no pending event and reschedules it
            self.peer_gc_next = None;
        }

        let to_peers = self.ur_out.route(to_peers);
        self.ensure_events(now);

        to_peers
    }

    fn ensure_events(&mut self, now: Instant) {
        self.ensure_wg();
        self.ensure_peer_gc(now);
    }

    fn ensure_wg(&mut self) {
        self.wg_next = self
            .wireguard
            .next_event()
            .map(|tr| self.events.add(tr, Subsystem::Wireguard));
    }

    fn ensure_peer_gc(&mut self, now: Instant) {
        if self.active_peers.is_empty() && self.outbound_flows.is_empty() {
            self.peer_gc_next = None;
        } else if self.peer_gc_next.is_none() {
            self.peer_gc_next = Some(self.events.add(
                TimeRange::new_around(now + Duration::from_secs(10), Duration::from_millis(2500)),
                Subsystem::PeerGc,
            ));
        }
    }
}

/// The result of processing outbound packets.
pub struct OutboundResult {
    /// Packets to be sent into underlay transports for transmission.
    pub to_peers: underlay_router::outbound::Result,

    /// Packets to be looped back and delivered to overlay transports.
    pub loopback: overlay_router::inbound::Result,
}

/// The result of processing inbound packets.
pub struct InboundResult {
    /// Decrypted packets to be delivered to overlay transports.
    pub to_local: overlay_router::inbound::Result,
    /// Encrypted packets to be sent to wireguard peers by the underlay.
    pub to_peers: underlay_router::outbound::Result,

    /// Encrypted disco packets to be handled externally.
    pub disco: Vec<PacketMut>,

    /// STUN packets to be handled externally.
    pub stun: Vec<PacketMut>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use etherparse::PacketBuilder;
    use overlay_router::outbound::RouteAction;

    const LOCAL: [u8; 4] = [100, 64, 0, 1];
    const REMOTE: [u8; 4] = [100, 64, 0, 2];
    const PEER: PeerId = PeerId(1);

    fn tcp_packet(
        src: [u8; 4],
        dst: [u8; 4],
        sport: u16,
        dport: u16,
        syn: bool,
        ack: bool,
    ) -> Vec<u8> {
        let mut builder = PacketBuilder::ipv4(src, dst, 64).tcp(sport, dport, 1, 65535);
        if syn {
            builder = builder.syn();
        }
        if ack {
            builder = builder.ack(2);
        }
        let mut packet = Vec::new();
        builder.write(&mut packet, &[]).unwrap();
        packet
    }

    #[test]
    fn drop_all_allows_outbound_tcp_replies() {
        let mut plane = DataPlane::new(NodeKeyPair::random());
        let mut routes = ts_bart::Table::default();
        routes.insert(IpAddr::from(REMOTE).into(), RouteAction::Wireguard(PEER));
        plane.or_out.swap(routes);
        plane.process_outbound(vec![
            tcp_packet(LOCAL, REMOTE, 49152, 22, true, false).into(),
        ]);

        for syn in [true, false] {
            let reply = tcp_packet(REMOTE, LOCAL, 22, 49152, syn, true);
            assert!(plane.allows_inbound(PEER, &reply, Instant::now()));
            assert!(!plane.allows_inbound(PeerId(2), &reply, Instant::now()));
            assert!(!plane.allows_inbound(PEER, &reply, Instant::now() + FLOW_EXPIRATION));
        }

        let syn = tcp_packet(REMOTE, LOCAL, 22, 49152, true, false);
        assert!(!plane.allows_inbound(PEER, &syn, Instant::now()));
    }

    #[test]
    fn drop_all_rejects_unsolicited_tcp_syn() {
        let plane = DataPlane::new(NodeKeyPair::random());
        let syn = tcp_packet(REMOTE, LOCAL, 49152, 22, true, false);
        assert!(!plane.allows_inbound(PEER, &syn, Instant::now()));
    }
}
