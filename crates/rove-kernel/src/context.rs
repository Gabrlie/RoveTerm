//! Typed service registry shared by every plugin.

use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use parking_lot::RwLock;

use crate::{Disposer, EventBus, KernelError};

struct ServiceEntry {
    id: u64,
    value: Box<dyn Any + Send + Sync>,
}

struct Inner {
    next_id: AtomicU64,
    services: RwLock<HashMap<TypeId, ServiceEntry>>,
    events: EventBus,
}

/// The shared context: a typed service registry plus the [`EventBus`].
///
/// Cloning is cheap; every clone refers to the same registry. Services are keyed by their type,
/// usually a trait object such as `dyn KeyCache`, so a Consumer depends only on the Definition.
#[derive(Clone)]
pub struct Context {
    inner: Arc<Inner>,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    /// Creates an empty context.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                next_id: AtomicU64::new(1),
                services: RwLock::new(HashMap::new()),
                events: EventBus::default(),
            }),
        }
    }

    /// Registers `service` as the provider of `S`.
    ///
    /// Returns [`KernelError::ServiceConflict`] when `S` already has a provider. Dropping the
    /// returned disposer removes this provider; it never removes a provider registered later.
    pub fn provide<S>(&self, service: Arc<S>) -> Result<Disposer, KernelError>
    where
        S: ?Sized + Send + Sync + 'static,
    {
        let key = TypeId::of::<S>();
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut services = self.inner.services.write();
            if services.contains_key(&key) {
                return Err(KernelError::ServiceConflict(type_name::<S>()));
            }
            services.insert(
                key,
                ServiceEntry {
                    id,
                    value: Box::new(service),
                },
            );
        }
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        Ok(Disposer::new(move || {
            if let Some(inner) = weak.upgrade() {
                let mut services = inner.services.write();
                if services.get(&key).is_some_and(|entry| entry.id == id) {
                    services.remove(&key);
                }
            }
        }))
    }

    /// Returns the provider of `S`, if one is registered.
    pub fn get<S>(&self) -> Option<Arc<S>>
    where
        S: ?Sized + Send + Sync + 'static,
    {
        self.inner
            .services
            .read()
            .get(&TypeId::of::<S>())
            .and_then(|entry| entry.value.downcast_ref::<Arc<S>>().cloned())
    }

    /// Returns the provider of `S`, or [`KernelError::MissingService`] when none is registered.
    pub fn require<S>(&self) -> Result<Arc<S>, KernelError>
    where
        S: ?Sized + Send + Sync + 'static,
    {
        self.get::<S>()
            .ok_or(KernelError::MissingService(type_name::<S>()))
    }

    /// Returns the event bus.
    pub fn events(&self) -> &EventBus {
        &self.inner.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Greeter: Send + Sync {
        fn greet(&self) -> &'static str;
    }

    struct English;
    impl Greeter for English {
        fn greet(&self) -> &'static str {
            "hello"
        }
    }

    #[test]
    fn provides_and_resolves_trait_objects() {
        let ctx = Context::new();
        let _disposer = ctx.provide::<dyn Greeter>(Arc::new(English)).unwrap();
        assert_eq!(ctx.require::<dyn Greeter>().unwrap().greet(), "hello");
    }

    #[test]
    fn disposer_revokes_service() {
        let ctx = Context::new();
        let disposer = ctx.provide::<dyn Greeter>(Arc::new(English)).unwrap();
        disposer.dispose();
        assert!(ctx.get::<dyn Greeter>().is_none());
        assert!(matches!(
            ctx.require::<dyn Greeter>(),
            Err(KernelError::MissingService(_))
        ));
    }

    #[test]
    fn duplicate_provider_is_rejected() {
        let ctx = Context::new();
        let _first = ctx.provide::<dyn Greeter>(Arc::new(English)).unwrap();
        assert!(matches!(
            ctx.provide::<dyn Greeter>(Arc::new(English)),
            Err(KernelError::ServiceConflict(_))
        ));
    }
}
