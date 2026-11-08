use std::sync::Arc;
use std::time::Duration;

use devsshd::{DevConfig, DevServer};
use rove_kernel::EventBus;
use rove_session::{TermSize, TerminalOutput};
use rove_ssh::*;
use rove_vault::{MemoryKeyCache, Vault};
use zeroize::Zeroizing;

async fn setup(idle_secs: u64) -> (DevServer, SshConnector, rove_kernel::Disposer) {
    let server = DevServer::start("127.0.0.1:0", DevConfig::demo())
        .await
        .unwrap();
    let vault = Arc::new(Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap());
    let events = EventBus::default();
    let prompts = Arc::new(HostKeyPrompts::new(events.clone(), Duration::from_secs(10)));
    let responder = prompts.clone();
    let listener = events.on::<HostKeyPrompt>(move |prompt| {
        responder.respond(&prompt.request_id, HostKeyAnswer::Trust);
    });
    let config = SshConfig {
        idle_disconnect_secs: idle_secs,
        ..SshConfig::default()
    };
    let connector = SshConnector::new(&config, Arc::new(VaultVerifier::new(vault, prompts)));
    (server, connector, listener)
}

fn target(server: &DevServer) -> SshTarget {
    SshTarget {
        hostname: "127.0.0.1".into(),
        port: server.addr().port(),
        username: "demo".into(),
        host_id: None,
        title: None,
    }
}

fn auth() -> AuthSpec {
    AuthSpec::Password(Zeroizing::new("demo".into()))
}

async fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn terminals_to_same_target_share_one_connection() {
    let (server, connector, _listener) = setup(60).await;
    let size = TermSize::default();
    let (to, credentials) = (target(&server), auth());
    let (a, b) = tokio::join!(
        connector.open_terminal(&to, &credentials, size),
        connector.open_terminal(&to, &credentials, size)
    );
    let (mut a, mut b) = (a.unwrap(), b.unwrap());
    let c = connector
        .open_terminal(&target(&server), &auth(), size)
        .await
        .unwrap();
    assert_eq!(server.connections(), 1);
    assert_eq!(connector.live_connections(), 1);

    // Channels are independent: closing one leaves the others usable.
    c.session.close().await.unwrap();
    a.session.write(b"echo from-a; exit\n").await.unwrap();
    b.session.write(b"echo from-b; exit\n").await.unwrap();
    for (opened, marker) in [(&mut a, "from-a"), (&mut b, "from-b")] {
        let mut text = Vec::new();
        while let Some(TerminalOutput::Data(data)) = opened.output.recv().await {
            text.extend_from_slice(&data);
        }
        assert!(String::from_utf8_lossy(&text).contains(marker));
    }
}

#[tokio::test]
async fn idle_connection_closes_and_next_open_reconnects() {
    let (server, connector, _listener) = setup(0).await;
    let opened = connector
        .open_terminal(&target(&server), &auth(), TermSize::default())
        .await
        .unwrap();
    opened.session.close().await.unwrap();
    drop(opened);
    assert!(
        wait_for(|| connector.live_connections() == 0).await,
        "idle connection was not closed"
    );

    let _again = connector
        .open_terminal(&target(&server), &auth(), TermSize::default())
        .await
        .unwrap();
    assert_eq!(server.connections(), 2);
}

#[tokio::test]
async fn open_live_reuses_connection_without_credentials() {
    let (server, connector, _listener) = setup(60).await;
    assert!(
        connector
            .open_live(&target(&server), TermSize::default())
            .await
            .unwrap()
            .is_none()
    );
    let _first = connector
        .open_terminal(&target(&server), &auth(), TermSize::default())
        .await
        .unwrap();
    let second = connector
        .open_live(&target(&server), TermSize::default())
        .await
        .unwrap();
    assert!(second.is_some());
    assert_eq!(server.connections(), 1);
}
