mod common;

use std::time::Duration;

use common::Harness;
use devsshd::DevConfig;
use rove_ssh::{ForwardState, ForwardStatus, Forwards, SshError};
use rove_vault::{ForwardId, ForwardSpec, HostId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Starts a TCP server that echoes everything back; returns its port.
async fn echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    port
}

async fn round_trip(stream: &mut TcpStream, payload: &[u8]) {
    stream.write_all(payload).await.unwrap();
    let mut back = vec![0; payload.len()];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut back))
        .await
        .expect("echo in time")
        .unwrap();
    assert_eq!(back, payload);
}

async fn connect(port: u16) -> TcpStream {
    TcpStream::connect(("127.0.0.1", port)).await.unwrap()
}

async fn wait_for(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ForwardStatus>,
    check: impl Fn(&ForwardStatus) -> bool,
) -> ForwardStatus {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = rx.recv().await.unwrap();
            if check(&status) {
                return status;
            }
        }
    })
    .await
    .expect("status in time")
}

#[tokio::test]
async fn local_forward_reaches_the_destination_and_keeps_the_connection_open() {
    let harness = Harness::new(DevConfig::demo()).await;
    let echo = echo_server().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _listener = harness.events.on::<ForwardStatus>(move |status| {
        let _ = tx.send(status.clone());
    });
    let forwards = Forwards::new(harness.events.clone());
    let connection = harness.connect().await;
    let id = ForwardId::new();
    let spec = ForwardSpec::Local {
        bind_host: "127.0.0.1".into(),
        bind_port: 0,
        dest_host: "127.0.0.1".into(),
        dest_port: echo,
    };
    let status = forwards
        .start(id, HostId::new(), connection.clone(), spec.clone())
        .await
        .unwrap();
    assert_eq!(status.state, ForwardState::Running);
    assert_ne!(status.bound_port, 0);
    assert!(connection.open_channels() >= 1, "the forward holds a lease");

    let again = forwards
        .start(id, HostId::new(), connection.clone(), spec)
        .await
        .unwrap();
    assert_eq!(
        again.bound_port, status.bound_port,
        "starting twice is a no-op"
    );

    let mut first = connect(status.bound_port).await;
    let mut second = connect(status.bound_port).await;
    round_trip(&mut first, b"hello through -L").await;
    round_trip(&mut second, b"second stream").await;
    let counted = wait_for(&mut rx, |status| status.active == 2).await;
    assert_eq!(counted.total, 2);
    assert_eq!(forwards.list().len(), 1);

    assert!(forwards.stop(id));
    let stopped = wait_for(&mut rx, |status| status.state == ForwardState::Stopped).await;
    assert_eq!(stopped.id, id);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        TcpStream::connect(("127.0.0.1", status.bound_port))
            .await
            .is_err()
    );
    assert!(forwards.list().is_empty());
    assert!(!forwards.stop(id));
}

#[tokio::test]
async fn dynamic_forward_speaks_socks5() {
    let harness = Harness::new(DevConfig::demo()).await;
    let echo = echo_server().await;
    let forwards = Forwards::new(harness.events.clone());
    let connection = harness.connect().await;
    let status = forwards
        .start(
            ForwardId::new(),
            HostId::new(),
            connection,
            ForwardSpec::Dynamic {
                bind_host: "127.0.0.1".into(),
                bind_port: 0,
            },
        )
        .await
        .unwrap();
    let mut stream = connect(status.bound_port).await;
    stream.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0u8; 2];
    stream.read_exact(&mut method).await.unwrap();
    assert_eq!(method, [5, 0]);
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend_from_slice(b"127.0.0.1");
    request.extend_from_slice(&echo.to_be_bytes());
    stream.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0, "SOCKS reply succeeded");
    round_trip(&mut stream, b"through socks").await;

    // An unreachable destination is reported to the SOCKS client.
    let mut refused = connect(status.bound_port).await;
    refused.write_all(&[5, 1, 0]).await.unwrap();
    refused.read_exact(&mut method).await.unwrap();
    refused
        .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0, 1])
        .await
        .unwrap();
    refused.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 4, "host unreachable");
}

#[tokio::test]
async fn remote_forward_brings_server_connections_here() {
    let harness = Harness::new(DevConfig::demo()).await;
    let echo = echo_server().await;
    let forwards = Forwards::new(harness.events.clone());
    let connection = harness.connect().await;
    let id = ForwardId::new();
    let status = forwards
        .start(
            id,
            HostId::new(),
            connection,
            ForwardSpec::Remote {
                bind_host: "localhost".into(),
                bind_port: 0,
                dest_host: "127.0.0.1".into(),
                dest_port: echo,
            },
        )
        .await
        .unwrap();
    assert_ne!(status.bound_port, 0, "the server allocated a port");
    // devsshd listens on loopback on the server side, which is this machine.
    let mut stream = connect(status.bound_port).await;
    round_trip(&mut stream, b"hello through -R").await;
    drop(stream);

    assert!(forwards.stop(id));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        TcpStream::connect(("127.0.0.1", status.bound_port))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn busy_local_port_fails_to_start() {
    let harness = Harness::new(DevConfig::demo()).await;
    let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = taken.local_addr().unwrap().port();
    let forwards = Forwards::new(harness.events.clone());
    let result = forwards
        .start(
            ForwardId::new(),
            HostId::new(),
            harness.connect().await,
            ForwardSpec::Dynamic {
                bind_host: "127.0.0.1".into(),
                bind_port: port,
            },
        )
        .await;
    let Err(error) = result else {
        panic!("binding a busy port must fail")
    };
    assert!(matches!(error, SshError::ForwardBind(_)));
    assert!(forwards.list().is_empty());

    let id = ForwardId::new();
    let spec = ForwardSpec::Dynamic {
        bind_host: "127.0.0.1".into(),
        bind_port: port,
    };
    forwards.record_failure(id, HostId::new(), spec, &error);
    let failed = forwards.status(id).unwrap();
    assert_eq!(failed.state, ForwardState::Failed);
    assert!(failed.error.unwrap().contains("cannot listen"));
    assert!(forwards.stop(id));
}

#[tokio::test]
async fn forward_fails_when_the_connection_closes() {
    let harness = Harness::new(DevConfig::demo()).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _listener = harness.events.on::<ForwardStatus>(move |status| {
        let _ = tx.send(status.clone());
    });
    let forwards = Forwards::new(harness.events.clone());
    let connection = harness.connect().await;
    let id = ForwardId::new();
    forwards
        .start(
            id,
            HostId::new(),
            connection.clone(),
            ForwardSpec::Dynamic {
                bind_host: "127.0.0.1".into(),
                bind_port: 0,
            },
        )
        .await
        .unwrap();
    connection.disconnect().await;
    let failed = wait_for(&mut rx, |status| status.state == ForwardState::Failed).await;
    assert_eq!(failed.error.as_deref(), Some("connection closed"));
    assert_eq!(forwards.status(id).unwrap().state, ForwardState::Failed);
}
