use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use futures::StreamExt;
use kameo::{
    Actor,
    actor::ActorRef,
    message::{Context, Message, StreamMessage},
};
use smol_str::ToSmolStr;
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use ts_netmon::{Event, Family, Interface, InterfaceId, MonType, Netmon, Route};

use crate::env::Env;

/// An IP address assigned to an interface in a host network snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalNetworkAddr {
    pub ip: IpAddr,
    pub prefix_len: u8,
}

/// An active host network interface. Its ID identifies the current network attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalInterface {
    pub id: u64,
    pub name: String,
    pub addrs: Vec<ExternalNetworkAddr>,
    pub default_v4: bool,
    pub default_v6: bool,
}

/// The current set of active host network interfaces. An empty set signals disconnection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalNetworkSnapshot {
    pub interfaces: Vec<ExternalInterface>,
}

const HOST_BRIDGE: MonType = MonType::new_static("host_bridge");

/// Network monitor backed by snapshots supplied by the host application.
pub struct ExternalNetmon(pub watch::Receiver<Option<ExternalNetworkSnapshot>>);

impl Netmon for ExternalNetmon {
    fn ty(&self) -> MonType {
        HOST_BRIDGE
    }

    fn event_stream(&self) -> io::Result<ts_netmon::BoxStream<io::Result<Event>>> {
        let mut previous = ExternalNetworkState::default();
        Ok(WatchStream::new(self.0.clone())
            .filter_map(|snapshot| async { snapshot.map(ExternalNetworkState::from) })
            .flat_map(move |snapshot| {
                let events = previous.diff(&snapshot);
                previous = snapshot;
                futures::stream::iter(events.into_iter().map(Ok))
            })
            .boxed())
    }

    fn strong_delete_consistency(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct ExternalNetworkState {
    interfaces: BTreeMap<InterfaceId, Interface>,
    addrs: BTreeSet<(InterfaceId, ipnet::IpNet)>,
    default_routes: BTreeSet<(InterfaceId, IpAddr)>,
}

impl From<ExternalNetworkSnapshot> for ExternalNetworkState {
    fn from(snapshot: ExternalNetworkSnapshot) -> Self {
        let mut state = Self::default();
        for interface in snapshot.interfaces {
            let id = InterfaceId::new(HOST_BRIDGE, interface.id);
            state
                .addrs
                .extend(interface.addrs.into_iter().filter_map(|addr| {
                    ipnet::IpNet::new(addr.ip, addr.prefix_len)
                        .ok()
                        .map(|addr| (id.clone(), addr))
                }));
            if interface.default_v4 {
                state
                    .default_routes
                    .insert((id.clone(), Ipv4Addr::UNSPECIFIED.into()));
            }
            if interface.default_v6 {
                state
                    .default_routes
                    .insert((id.clone(), Ipv6Addr::UNSPECIFIED.into()));
            }
            state.interfaces.insert(
                id.clone(),
                Interface {
                    id,
                    up: true,
                    name: interface.name,
                    mtu: None,
                    hardware_addr: None,
                    metric_v4: 0,
                    metric_v6: 0,
                },
            );
        }
        state
    }
}

impl ExternalNetworkState {
    fn diff(&self, next: &Self) -> Vec<Event> {
        let mut events = Vec::new();
        events.extend(
            self.default_routes
                .difference(&next.default_routes)
                .map(|(id, ip)| Event::RouteRemoved(id.clone(), external_default_route(*ip))),
        );
        events.extend(
            self.addrs
                .difference(&next.addrs)
                .map(|(id, addr)| Event::AddrRemoved(id.clone(), *addr)),
        );
        for id in self.interfaces.keys() {
            if !next.interfaces.contains_key(id) {
                events.push(Event::InterfaceRemoved(id.clone()));
            }
        }

        for (id, interface) in &next.interfaces {
            if self.interfaces.get(id) != Some(interface) {
                events.push(Event::InterfaceUpsert(interface.clone()));
            }
        }
        events.extend(
            next.addrs
                .difference(&self.addrs)
                .map(|(id, addr)| Event::AddrUpsert(id.clone(), *addr)),
        );
        events.extend(
            next.default_routes
                .difference(&self.default_routes)
                .map(|(id, ip)| Event::RouteUpsert(id.clone(), external_default_route(*ip))),
        );
        events
    }
}

fn external_default_route(ip: IpAddr) -> Route {
    Route {
        dst: ipnet::IpNet::new_assert(ip, 0),
        // The host supplies route ownership only. Netmon requires a gateway entry
        // to recognize a default route.
        gateway: smallvec::smallvec![ip],
        metric: 0,
    }
}

pub struct NetmonActor {
    mon: Arc<dyn Netmon>,
    state: State,
    env: Env,
    _id: Id,
}

/// Map from the unique part of a route to its metric.
pub type Routes = HashMap<ts_netmon::RouteUnique, usize>;

/// The unique id of a [`NetmonActor`] instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id {
    pub ty: ts_netmon::MonType,
    pub id: usize,
}

impl Id {
    fn new(ty: ts_netmon::MonType) -> Self {
        static ID: AtomicUsize = AtomicUsize::new(0);

        Self {
            ty,
            id: ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// Netmon state at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The id of the [`NetmonActor`] that generated this [`State`].
    pub id: Id,

    /// Interfaces detected on a [`Netmon`].
    pub interfaces: HashMap<InterfaceId, ts_netmon::Interface>,

    /// Routes per interface.
    pub routes: HashMap<InterfaceId, Routes>,

    /// Addresses per interface.
    ///
    /// To recover the full address with the prefix len, use key's `addr()` as the address
    /// and the value as the prefix len.
    ///
    /// The representation is split this way to support [`Netmon::interface_unique_addrs`].
    pub addrs: HashMap<InterfaceId, HashMap<ipnet::IpNet, u8>>,

    /// The interface that has the IPv4 default route, if any.
    pub default_route_interface_v4: Option<InterfaceId>,

    /// The interface that has the IPv6 default route, if any.
    pub default_route_interface_v6: Option<InterfaceId>,
}

/// Local addresses and default routes that affect network connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkFingerprint {
    default_route_interface_v4: Option<InterfaceId>,
    default_route_interface_v6: Option<InterfaceId>,
    sorted_up_addrs: Vec<ipnet::IpNet>,
}

impl NetworkFingerprint {
    pub fn is_empty(&self) -> bool {
        self.default_route_interface_v4.is_none()
            && self.default_route_interface_v6.is_none()
            && self.sorted_up_addrs.is_empty()
    }
}

impl State {
    pub fn fingerprint(&self) -> NetworkFingerprint {
        let mut sorted_up_addrs: Vec<_> = self.up_addrs().map(|(_, addr)| addr).collect();
        sorted_up_addrs.sort_unstable();
        NetworkFingerprint {
            default_route_interface_v4: self.default_route_interface_v4.clone(),
            default_route_interface_v6: self.default_route_interface_v6.clone(),
            sorted_up_addrs,
        }
    }

    /// Iterate the addresses on all interfaces in the `up` state.
    pub fn up_addrs(&self) -> impl Iterator<Item = (InterfaceId, ipnet::IpNet)> {
        self.addrs
            .iter()
            .filter_map(|(id, addrs)| {
                let interface = self.interfaces.get(id)?;
                if !interface.up {
                    return None;
                }

                Some(
                    addrs
                        .iter()
                        .map(|(ipn, &pfx)| (id.clone(), ipnet::IpNet::new_assert(ipn.addr(), pfx))),
                )
            })
            .flatten()
    }

    /// Iterate the routes on all interfaces in the `up` state.
    ///
    /// Like [`routes`][State::routes], the final element of the tuple is the full per-route
    /// metric including the interface metric (if present).
    #[expect(dead_code)]
    pub fn up_routes(&self) -> impl Iterator<Item = (InterfaceId, ts_netmon::Route, usize)> {
        self.routes()
            .filter(|(iid, ..)| self.interfaces.get(iid).is_some_and(|iface| iface.up))
    }

    /// Iterate all known routes.
    ///
    /// The final element of the tuple is the full per-route metric including the interface
    /// metric (if there is one).
    ///
    /// Includes routes on down interfaces.
    #[expect(dead_code)]
    pub fn routes(&self) -> impl Iterator<Item = (InterfaceId, ts_netmon::Route, usize)> {
        self.routes
            .iter()
            .filter_map(|(id, routes)| {
                let interface = self.interfaces.get(id)?;

                Some(routes.iter().map(|((dst, gws), &metric)| {
                    (
                        id.clone(),
                        ts_netmon::Route {
                            gateway: gws.clone(),
                            metric,
                            dst: *dst,
                        },
                        metric
                            + if dst.addr().is_ipv4() {
                                interface.metric_v4
                            } else {
                                interface.metric_v6
                            },
                    )
                }))
            })
            .flatten()
    }
}

impl Actor for NetmonActor {
    type Args = (Env, Arc<dyn Netmon>);
    type Error = io::Error;

    async fn on_start((env, mon): Self::Args, slf: ActorRef<Self>) -> Result<Self, Self::Error> {
        let id = Id::new(mon.ty());

        slf.attach_stream(
            {
                use tokio_stream::StreamExt;

                mon.with_default_route_events()?
                    .chunks_timeout(256, Duration::from_millis(100))
                    .boxed()
            },
            (),
            (),
        );

        env.register(Some(mon.ty().as_ref().to_smolstr()), &slf)
            .await
            .unwrap();

        Ok(Self {
            env,
            mon,
            state: State {
                id: id.clone(),
                addrs: Default::default(),
                routes: Default::default(),
                interfaces: Default::default(),
                default_route_interface_v4: None,
                default_route_interface_v6: None,
            },
            _id: id,
        })
    }
}

impl Message<StreamMessage<Vec<io::Result<Event>>, (), ()>> for NetmonActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<Vec<io::Result<Event>>, (), ()>,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        let events = match msg {
            StreamMessage::Started(_) | StreamMessage::Finished(_) => {
                return;
            }
            StreamMessage::Next(val) => val,
        };

        if events.is_empty() {
            tracing::warn!("empty netmon event");
            return;
        }

        let mut modified = false;

        for event in &events {
            let event_modified = self.handle_event(event);
            if event_modified {
                tracing::trace!(mutating_netmon_event = ?event);
            }

            modified = modified || event_modified;
        }

        if !modified {
            return;
        }

        tracing::debug!(n_coalesced_events = events.len(), "netmon mutation");

        self.env
            .publish(Arc::new(self.state.clone()))
            .await
            .unwrap();
    }
}

impl NetmonActor {
    fn handle_event(&mut self, event: &io::Result<Event>) -> bool {
        let event = match event {
            Ok(event) => event,
            Err(e) => {
                tracing::error!(error = %e, "netmon error");
                return false;
            }
        };

        let mut modified = false;

        match &event {
            Event::RouteUpsert(interface, route) => {
                let old = self
                    .state
                    .routes
                    .entry(interface.clone())
                    .or_default()
                    .insert(route.unique(), route.metric);

                modified = old.is_none_or(|old| old != route.metric);
            }
            Event::RouteRemoved(interface, route) => {
                let mut empty = false;

                self.state.routes.entry(interface.clone()).and_modify(|r| {
                    modified = r.remove(&route.unique()).is_some();
                    empty = r.is_empty();
                });

                if empty {
                    self.state.routes.remove(interface);
                }
            }
            Event::AddrUpsert(interface, addr) => {
                let (addr, pfx_len) = self.split_interface_addr(addr);

                let old = self
                    .state
                    .addrs
                    .entry(interface.clone())
                    .or_default()
                    .insert(addr, pfx_len);

                modified = old.is_none_or(|old| old != pfx_len);
            }
            Event::AddrRemoved(interface, addr) => {
                let mut empty = false;
                let (addr, _pfx_len) = self.split_interface_addr(addr);

                self.state.addrs.entry(interface.clone()).and_modify(|e| {
                    modified = e.remove(&addr).is_some();
                    empty = e.is_empty();
                });

                if empty {
                    self.state.addrs.remove(interface);
                }
            }
            Event::InterfaceUpsert(interface) => {
                let old = self
                    .state
                    .interfaces
                    .insert(interface.id.clone(), interface.clone());

                modified = old.is_none_or(|old| &old != interface);
            }
            Event::InterfaceRemoved(interface) => {
                self.state.interfaces.retain(|x, _| {
                    let ret = x != interface;
                    if !ret {
                        modified = true;
                    }

                    ret
                });

                // NOTE(npry): the below modification of routes and addrs on link deletion is due to
                // divergence in platform behavior:
                //
                // - rtnetlink has a strongly-consistent event ordering model (as it's a single
                //   reliable socket) that orders route and address deletion events before link
                //   deletions, but it doesn't seem to _guarantee_ individual deletions for all
                //   routes and addresses related to a link when it's deleted; the link deletion
                //   itself seems to be intended to clean those up.
                // - Win32 notify APIs are, on the other hand, free to race between route, address,
                //   and link modifications, but are ordered within each logical resource stream and
                //   appear to exhaustively issue deletion events for all routes and addresses _at
                //   some point_ when a link is deleted. The link deletion event, however, may issue
                //   in an arbitrary order wrt. the other events. The implication of this is that we
                //   technically shouldn't proactively delete the other resources when the link is
                //   removed, since we could be deleting routes and addresses related to a future
                //   generation of the link in a race condition.
                //
                // This logic is meant to rectify that and make our state reflect what the platform
                // has told us it should be regardless of the underlying consistency model.
                if self.mon.strong_delete_consistency() {
                    modified |= self.state.routes.remove(interface).is_some();
                    modified |= self.state.addrs.remove(interface).is_some();
                }
            }
            Event::DefaultRouteInterface(iface, family) => {
                let pre_v4 = self.state.default_route_interface_v4.clone();
                let pre_v6 = self.state.default_route_interface_v6.clone();

                match family {
                    Family::Ipv4 => self.state.default_route_interface_v4 = iface.clone(),
                    Family::Ipv6 => self.state.default_route_interface_v6 = iface.clone(),
                }

                if pre_v4 != self.state.default_route_interface_v4
                    || pre_v6 != self.state.default_route_interface_v6
                {
                    modified = true;
                }
            }
        }

        if !modified {
            tracing::trace!(?event, "netmon event with no delta");
        }

        modified
    }

    /// Split the interface addr according to [`Netmon::interface_unique_addrs`]:
    ///
    /// - If interface addresses are unique, returns (addr/0, prefix_len)
    /// - If interface addresses are not unique, returns (addr/prefix_len, prefix_len)
    fn split_interface_addr(&self, addr: &ipnet::IpNet) -> (ipnet::IpNet, u8) {
        if self.mon.interface_unique_addrs() {
            (ipnet::IpNet::new_assert(addr.addr(), 0), addr.prefix_len())
        } else {
            (*addr, addr.prefix_len())
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;

    use super::*;

    fn addr(ip: &str, prefix_len: u8) -> ExternalNetworkAddr {
        ExternalNetworkAddr {
            ip: ip.parse().unwrap(),
            prefix_len,
        }
    }

    fn interface(id: u64, name: &str, addrs: Vec<ExternalNetworkAddr>) -> ExternalInterface {
        ExternalInterface {
            id,
            name: name.into(),
            addrs,
            default_v4: false,
            default_v6: false,
        }
    }

    fn snapshot(interfaces: Vec<ExternalInterface>) -> Option<ExternalNetworkSnapshot> {
        Some(ExternalNetworkSnapshot { interfaces })
    }

    fn id(value: u64) -> InterfaceId {
        InterfaceId::new(HOST_BRIDGE, value)
    }

    fn available_events(
        stream: &mut (impl futures::Stream<Item = io::Result<Event>> + Unpin),
    ) -> Vec<Event> {
        std::iter::from_fn(|| stream.next().now_or_never().flatten())
            .collect::<io::Result<_>>()
            .unwrap()
    }

    #[tokio::test]
    async fn emits_initial_snapshot_and_diffs_interface_and_address_changes() {
        let mut wifi = interface(
            1,
            "wlan0",
            vec![addr("192.0.2.5", 24), addr("2001:db8::5", 64)],
        );
        let (tx, rx) = watch::channel(snapshot(vec![wifi.clone()]));
        let mut stream = ExternalNetmon(rx).event_stream().unwrap();
        let events = available_events(&mut stream);
        assert!(
            matches!(&events[0], Event::InterfaceUpsert(i) if i.id == id(1) && i.name == "wlan0" && i.up)
        );
        assert_eq!(
            events[1..],
            [
                Event::AddrUpsert(id(1), "192.0.2.5/24".parse().unwrap()),
                Event::AddrUpsert(id(1), "2001:db8::5/64".parse().unwrap()),
            ],
        );

        wifi.name = "wifi".into();
        wifi.addrs = vec![addr("192.0.2.5", 25), addr("192.0.2.6", 25)];
        tx.send_replace(snapshot(vec![wifi.clone()]));
        let events = available_events(&mut stream);
        assert_eq!(events.len(), 5);
        assert_eq!(
            events[..2],
            [
                Event::AddrRemoved(id(1), "192.0.2.5/24".parse().unwrap()),
                Event::AddrRemoved(id(1), "2001:db8::5/64".parse().unwrap()),
            ],
        );
        assert!(
            matches!(&events[2], Event::InterfaceUpsert(i) if i.id == id(1) && i.name == "wifi")
        );
        assert_eq!(
            events[3..],
            [
                Event::AddrUpsert(id(1), "192.0.2.5/25".parse().unwrap()),
                Event::AddrUpsert(id(1), "192.0.2.6/25".parse().unwrap()),
            ],
        );
        wifi.addrs.reverse();
        tx.send_replace(snapshot(vec![wifi]));
        assert!(available_events(&mut stream).is_empty());
    }

    #[tokio::test]
    async fn moves_default_routes_to_cellular_and_clears_disconnected_state() {
        let wifi = interface(1, "wlan0", vec![addr("192.0.2.5", 24)]);
        let mut cell = interface(2, "rmnet0", vec![addr("198.51.100.5", 24)]);
        let (tx, rx) = watch::channel(None);
        let mon = ExternalNetmon(rx);
        let mut stream = Box::pin((&mon as &dyn Netmon).with_default_route_events().unwrap());
        assert!(available_events(&mut stream).is_empty());

        tx.send_replace(snapshot(vec![
            ExternalInterface {
                default_v4: true,
                default_v6: true,
                ..wifi.clone()
            },
            cell.clone(),
        ]));
        let events = available_events(&mut stream);
        assert!(events.contains(&Event::DefaultRouteInterface(Some(id(1)), Family::Ipv4)));
        assert!(events.contains(&Event::DefaultRouteInterface(Some(id(1)), Family::Ipv6)));

        cell.default_v4 = true;
        cell.default_v6 = true;
        tx.send_replace(snapshot(vec![wifi, cell]));
        let events = available_events(&mut stream);
        for (destination, family) in [("0.0.0.0/0", Family::Ipv4), ("::/0", Family::Ipv6)] {
            let destination: ipnet::IpNet = destination.parse().unwrap();
            assert!(events.iter().any(|event|
                matches!(event, Event::RouteRemoved(i, route) if *i == id(1) && route.dst == destination)));
            assert!(events.iter().any(|event|
                matches!(event, Event::RouteUpsert(i, route) if *i == id(2) && route.dst == destination)));
            assert!(events.contains(&Event::DefaultRouteInterface(Some(id(2)), family)));
        }

        tx.send_replace(snapshot(vec![]));
        let events = available_events(&mut stream);
        assert_eq!(events.len(), 8);
        assert!(events.contains(&Event::DefaultRouteInterface(None, Family::Ipv4)));
        assert!(events.contains(&Event::DefaultRouteInterface(None, Family::Ipv6)));
        assert!(events.contains(&Event::AddrRemoved(id(1), "192.0.2.5/24".parse().unwrap())));
        assert!(events.contains(&Event::AddrRemoved(
            id(2),
            "198.51.100.5/24".parse().unwrap()
        )));
        assert!(events.contains(&Event::InterfaceRemoved(id(1))));
        assert!(events.contains(&Event::InterfaceRemoved(id(2))));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::RouteRemoved(i, _) if *i == id(2)))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn replaces_interface_id_when_router_changes_with_the_same_address() {
        let mut wifi = interface(1, "wlan0", vec![addr("192.0.2.5", 24)]);
        wifi.default_v4 = true;
        let (tx, rx) = watch::channel(snapshot(vec![wifi.clone()]));
        let mon = ExternalNetmon(rx);
        let mut stream = Box::pin((&mon as &dyn Netmon).with_default_route_events().unwrap());
        assert!(
            available_events(&mut stream)
                .contains(&Event::DefaultRouteInterface(Some(id(1)), Family::Ipv4))
        );

        wifi.id = 7;
        tx.send_replace(snapshot(vec![wifi]));
        let events = available_events(&mut stream);
        let address = "192.0.2.5/24".parse().unwrap();
        assert!(events.contains(&Event::AddrRemoved(id(1), address)));
        assert!(events.contains(&Event::AddrUpsert(id(7), address)));
        assert!(events.contains(&Event::InterfaceRemoved(id(1))));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::InterfaceUpsert(i) if i.id == id(7)))
        );
        assert_eq!(
            events.last(),
            Some(&Event::DefaultRouteInterface(Some(id(7)), Family::Ipv4))
        );
    }
}
