//! Shares one SSH connection among every terminal opened to the same `user@host:port`.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::{SshConnection, SshTarget};

/// Identity of a shareable connection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PoolKey {
    hostname: String,
    port: u16,
    username: String,
}

impl PoolKey {
    pub(crate) fn of(target: &SshTarget) -> Self {
        Self {
            hostname: target.hostname.to_ascii_lowercase(),
            port: target.port,
            username: target.username.clone(),
        }
    }
}

/// One pool entry. The async mutex serializes connection attempts, so concurrent opens to the same
/// target produce a single connection.
#[derive(Default)]
pub(crate) struct Slot {
    pub(crate) connection: tokio::sync::Mutex<Option<Arc<SshConnection>>>,
}

#[derive(Default)]
pub(crate) struct Pool {
    slots: Mutex<HashMap<PoolKey, Arc<Slot>>>,
}

impl Pool {
    pub(crate) fn slot(&self, key: PoolKey) -> Arc<Slot> {
        self.slots.lock().entry(key).or_default().clone()
    }

    /// Returns live pooled connections.
    pub(crate) fn live(&self) -> Vec<Arc<SshConnection>> {
        let slots: Vec<_> = self.slots.lock().values().cloned().collect();
        slots
            .iter()
            .filter_map(|slot| {
                slot.connection
                    .try_lock()
                    .ok()
                    .and_then(|guard| guard.clone())
            })
            .filter(|connection| !connection.is_closed())
            .collect()
    }
}
