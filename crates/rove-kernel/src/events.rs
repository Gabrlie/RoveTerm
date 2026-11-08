//! Typed in-process event bus: broadcast, serial and waterfall dispatch.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use parking_lot::RwLock;

use crate::Disposer;

/// A boxed, sendable future.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A live event broadcast to listeners in-process. Live events are never persisted.
pub trait Event: Send + Sync + 'static {}

/// A capability interception point. Interceptors form a chain in front of the capability.
///
/// Implement it on a marker type; `Input` is the request and `Output` the result returned by the
/// chain.
pub trait Waterfall: 'static {
    /// The request passed down the chain.
    type Input: Send + 'static;
    /// The result returned up the chain.
    type Output: Send + 'static;
}

type BroadcastFn<E> = dyn Fn(&E) + Send + Sync;
type SerialFn<E> = dyn Fn(Arc<E>) -> BoxFuture<()> + Send + Sync;
type InterceptFn<W> =
    dyn Fn(<W as Waterfall>::Input, Next<W>) -> BoxFuture<<W as Waterfall>::Output> + Send + Sync;
type TerminalFn<W> =
    dyn Fn(<W as Waterfall>::Input) -> BoxFuture<<W as Waterfall>::Output> + Send + Sync;

struct Entry {
    id: u64,
    priority: i32,
    handler: Box<dyn Any + Send + Sync>,
}

#[derive(Default)]
struct Tables {
    broadcast: HashMap<TypeId, Vec<Entry>>,
    serial: HashMap<TypeId, Vec<Entry>>,
    waterfall: HashMap<TypeId, Vec<Entry>>,
}

#[derive(Clone, Copy)]
enum Table {
    Broadcast,
    Serial,
    Waterfall,
}

impl Tables {
    fn get(&self, table: Table) -> &HashMap<TypeId, Vec<Entry>> {
        match table {
            Table::Broadcast => &self.broadcast,
            Table::Serial => &self.serial,
            Table::Waterfall => &self.waterfall,
        }
    }

    fn get_mut(&mut self, table: Table) -> &mut HashMap<TypeId, Vec<Entry>> {
        match table {
            Table::Broadcast => &mut self.broadcast,
            Table::Serial => &mut self.serial,
            Table::Waterfall => &mut self.waterfall,
        }
    }
}

struct BusInner {
    next_id: AtomicU64,
    tables: RwLock<Tables>,
}

/// The event bus owned by a [`Context`](crate::Context).
///
/// Handlers are snapshotted before dispatch, so a handler may register or dispose other handlers
/// without deadlocking; such changes apply to the next dispatch.
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<BusInner>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            inner: Arc::new(BusInner {
                next_id: AtomicU64::new(1),
                tables: RwLock::new(Tables::default()),
            }),
        }
    }
}

impl EventBus {
    fn insert(
        &self,
        table: Table,
        key: TypeId,
        priority: i32,
        handler: Box<dyn Any + Send + Sync>,
    ) -> Disposer {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut tables = self.inner.tables.write();
            let entries = tables.get_mut(table).entry(key).or_default();
            // Higher priority first; equal priority keeps registration order.
            let index = entries.partition_point(|entry| entry.priority >= priority);
            entries.insert(
                index,
                Entry {
                    id,
                    priority,
                    handler,
                },
            );
        }
        let weak: Weak<BusInner> = Arc::downgrade(&self.inner);
        Disposer::new(move || {
            if let Some(inner) = weak.upgrade() {
                let mut tables = inner.tables.write();
                if let Some(entries) = tables.get_mut(table).get_mut(&key) {
                    entries.retain(|entry| entry.id != id);
                }
            }
        })
    }

    fn snapshot<H: ?Sized + Send + Sync + 'static>(
        &self,
        table: Table,
        key: TypeId,
    ) -> Vec<Arc<H>> {
        let tables = self.inner.tables.read();
        tables
            .get(table)
            .get(&key)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.handler.downcast_ref::<Arc<H>>().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Registers a broadcast listener for `E`. Listeners run synchronously on the emitting thread
    /// and must not block; offload slow work to a task.
    pub fn on<E: Event>(&self, handler: impl Fn(&E) + Send + Sync + 'static) -> Disposer {
        let handler: Arc<BroadcastFn<E>> = Arc::new(handler);
        self.insert(Table::Broadcast, TypeId::of::<E>(), 0, Box::new(handler))
    }

    /// Delivers `event` to every broadcast listener of `E` in registration order.
    pub fn emit<E: Event>(&self, event: &E) {
        for handler in self.snapshot::<BroadcastFn<E>>(Table::Broadcast, TypeId::of::<E>()) {
            handler(event);
        }
    }

    /// Registers a serial listener for `E`. Serial listeners are awaited one after another.
    pub fn on_serial<E, F, Fut>(&self, handler: F) -> Disposer
    where
        E: Event,
        F: Fn(Arc<E>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler: Arc<SerialFn<E>> = Arc::new(move |event| Box::pin(handler(event)));
        self.insert(Table::Serial, TypeId::of::<E>(), 0, Box::new(handler))
    }

    /// Runs every serial listener of `E` in registration order, awaiting each before the next.
    pub async fn emit_serial<E: Event>(&self, event: E) {
        let event = Arc::new(event);
        for handler in self.snapshot::<SerialFn<E>>(Table::Serial, TypeId::of::<E>()) {
            handler(event.clone()).await;
        }
    }

    /// Registers an interceptor on waterfall `W`. Higher `priority` runs earlier; equal priorities
    /// run in registration order.
    ///
    /// The interceptor receives the request and a [`Next`]. It delegates by awaiting
    /// [`Next::run`]; returning without calling it short-circuits the rest of the chain and the
    /// capability itself.
    pub fn intercept<W, F, Fut>(&self, priority: i32, interceptor: F) -> Disposer
    where
        W: Waterfall,
        F: Fn(W::Input, Next<W>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = W::Output> + Send + 'static,
    {
        let handler: Arc<InterceptFn<W>> =
            Arc::new(move |input, next| Box::pin(interceptor(input, next)));
        self.insert(
            Table::Waterfall,
            TypeId::of::<W>(),
            priority,
            Box::new(handler),
        )
    }

    /// Passes `input` through the interceptors of `W`, ending in `capability` when every
    /// interceptor delegates.
    pub async fn run<W, F, Fut>(&self, input: W::Input, capability: F) -> W::Output
    where
        W: Waterfall,
        F: Fn(W::Input) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = W::Output> + Send + 'static,
    {
        let chain: Arc<[Arc<InterceptFn<W>>]> = self
            .snapshot::<InterceptFn<W>>(Table::Waterfall, TypeId::of::<W>())
            .into();
        let terminal: Arc<TerminalFn<W>> = Arc::new(move |input| Box::pin(capability(input)));
        Next {
            chain,
            index: 0,
            terminal,
        }
        .run(input)
        .await
    }
}

/// The remainder of a waterfall chain, handed to each interceptor.
pub struct Next<W: Waterfall> {
    chain: Arc<[Arc<InterceptFn<W>>]>,
    index: usize,
    terminal: Arc<TerminalFn<W>>,
}

impl<W: Waterfall> Next<W> {
    /// Hands `input` to the next interceptor, or to the capability when none remain.
    pub fn run(self, input: W::Input) -> BoxFuture<W::Output> {
        match self.chain.get(self.index).cloned() {
            Some(interceptor) => {
                let next = Next {
                    chain: self.chain,
                    index: self.index + 1,
                    terminal: self.terminal,
                };
                interceptor(input, next)
            }
            None => (self.terminal)(input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Ping(u32);
    impl Event for Ping {}

    struct Exec;
    impl Waterfall for Exec {
        type Input = String;
        type Output = Result<String, String>;
    }

    #[test]
    fn broadcast_stops_after_dispose() {
        let bus = EventBus::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let disposer = bus.on::<Ping>(move |ping| sink.lock().unwrap().push(ping.0));
        bus.emit(&Ping(1));
        disposer.dispose();
        bus.emit(&Ping(2));
        assert_eq!(*seen.lock().unwrap(), vec![1]);
    }

    #[test]
    fn listener_may_register_during_dispatch() {
        let bus = EventBus::default();
        let held = Arc::new(Mutex::new(Vec::new()));
        let inner_bus = bus.clone();
        let holder = held.clone();
        let _outer = bus.on::<Ping>(move |_| {
            holder.lock().unwrap().push(inner_bus.on::<Ping>(|_| {}));
        });
        bus.emit(&Ping(1));
        assert_eq!(held.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn serial_listeners_run_in_order() {
        let bus = EventBus::default();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut disposers = Vec::new();
        for i in 0..3 {
            let order = order.clone();
            disposers.push(bus.on_serial::<Ping, _, _>(move |ping| {
                let order = order.clone();
                async move {
                    tokio::task::yield_now().await;
                    order.lock().unwrap().push((i, ping.0));
                }
            }));
        }
        bus.emit_serial(Ping(7)).await;
        assert_eq!(*order.lock().unwrap(), vec![(0, 7), (1, 7), (2, 7)]);
    }

    #[tokio::test]
    async fn waterfall_orders_by_priority_and_reaches_capability() {
        let bus = EventBus::default();
        let _low = bus.intercept::<Exec, _, _>(0, |input, next| next.run(format!("{input}+low")));
        let _high =
            bus.intercept::<Exec, _, _>(10, |input, next| next.run(format!("{input}+high")));
        let result = bus
            .run::<Exec, _, _>("cmd".into(), |input| async move { Ok(input) })
            .await;
        assert_eq!(result, Ok("cmd+high+low".into()));
    }

    #[tokio::test]
    async fn waterfall_short_circuits_without_next() {
        let bus = EventBus::default();
        let reached = Arc::new(Mutex::new(false));
        let _deny =
            bus.intercept::<Exec, _, _>(0, |_input, _next| async { Err("denied".to_string()) });
        let flag = reached.clone();
        let result = bus
            .run::<Exec, _, _>("rm -rf /".into(), move |input| {
                let flag = flag.clone();
                async move {
                    *flag.lock().unwrap() = true;
                    Ok(input)
                }
            })
            .await;
        assert_eq!(result, Err("denied".into()));
        assert!(!*reached.lock().unwrap());
    }
}
