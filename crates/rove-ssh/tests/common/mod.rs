//! Shared harness: a devsshd server and a connector that trusts every host key.

use std::sync::Arc;
use std::time::Duration;

use devsshd::{DevConfig, DevServer};
use rove_kernel::{Disposer, EventBus};
use rove_ssh::*;
use rove_vault::{MemoryKeyCache, Vault};
use zeroize::Zeroizing;

pub struct Harness {
    pub server: DevServer,
    pub connector: Arc<SshConnector>,
    pub events: EventBus,
    _trust: Disposer,
}

impl Harness {
    pub async fn new(config: DevConfig) -> Self {
        let server = DevServer::start("127.0.0.1:0", config).await.unwrap();
        let vault = Arc::new(Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap());
        let events = EventBus::default();
        let prompts = Arc::new(HostKeyPrompts::new(events.clone(), Duration::from_secs(10)));
        let verifier = Arc::new(VaultVerifier::new(vault, prompts.clone()));
        let connector = Arc::new(SshConnector::new(&SshConfig::default(), verifier));
        let trust = events.on::<HostKeyPrompt>(move |prompt| {
            prompts.respond(&prompt.request_id, HostKeyAnswer::Trust);
        });
        Self {
            server,
            connector,
            events,
            _trust: trust,
        }
    }

    pub fn target(&self) -> SshTarget {
        SshTarget {
            hostname: "127.0.0.1".into(),
            port: self.server.addr().port(),
            username: "demo".into(),
            host_id: None,
            title: None,
            route: String::new(),
        }
    }

    pub async fn connect(&self) -> Arc<SshConnection> {
        let auth = AuthSpec::Password(Zeroizing::new("demo".into()));
        self.connector.shared(&self.target(), &auth).await.unwrap()
    }
}
