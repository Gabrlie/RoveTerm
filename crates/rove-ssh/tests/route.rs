mod common;

use std::sync::Arc;
use std::time::Duration;

use common::Harness;
use devsshd::{DevConfig, DevServer};
use rove_session::{TermSize, TerminalOutput};
use rove_ssh::{AuthSpec, ForwardState, Forwards, SshConnection, SshError, SshTarget, Via};
use rove_vault::{ForwardId, ForwardSpec, HostId, ProxyProtocol};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

fn demo() -> AuthSpec {
    AuthSpec::Password(Zeroizing::new("demo".into()))
}

fn target_of(server: &DevServer) -> SshTarget {
    SshTarget {
        hostname: "127.0.0.1".into(),
        port: server.addr().port(),
        username: "demo".into(),
        host_id: None,
        title: None,
    }
}

/// Runs `script` in a shell on `connection` and returns the output.
async fn shell(connection: &Arc<SshConnection>, script: &[u8]) -> String {
    let mut opened = connection
        .open_shell(TermSize::new(80, 20).unwrap())
        .await
        .unwrap();
    opened.session.write(script).await.unwrap();
    let mut text = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), opened.output.recv())
            .await
            .expect("shell output in time")
        {
            Some(TerminalOutput::Data(data)) => text.extend_from_slice(&data),
            Some(TerminalOutput::Exit(_)) | None => {
                return String::from_utf8_lossy(&text).into_owned();
            }
        }
    }
}

#[tokio::test]
async fn jump_host_tunnels_to_the_destination() {
    let bastion = Harness::new(DevConfig::demo()).await;
    let inner = DevServer::start("127.0.0.1:0", DevConfig::demo())
        .await
        .unwrap();
    let jump = bastion.connect().await;

    let hop = bastion
        .connector
        .connect_via(&target_of(&inner), &demo(), &Via::Jump(jump.clone()), None)
        .await
        .unwrap();
    // The tunnel counts as a channel of the jump connection.
    assert_eq!(jump.open_channels(), 1);
    let output = shell(&hop, b"echo via-$((40+2)); exit\n").await;
    assert!(output.contains("via-42"), "{output}");
    // Only the bastion saw a TCP connection from this process; the inner server was reached
    // through it.
    assert_eq!(bastion.server.connections(), 1);
}

#[tokio::test]
async fn jump_host_reports_an_unreachable_destination() {
    let bastion = Harness::new(DevConfig::demo()).await;
    let jump = bastion.connect().await;
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let target = SshTarget {
        port,
        ..bastion.target()
    };
    let result = bastion
        .connector
        .connect_via(&target, &demo(), &Via::Jump(jump), None)
        .await;
    assert!(
        matches!(result, Err(SshError::Jump(_))),
        "{:?}",
        result.err()
    );
}

/// A minimal HTTP CONNECT proxy for one tunnel at a time.
async fn http_proxy() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(client.read_u8().await.unwrap());
                }
                let head = String::from_utf8(head).unwrap();
                let authority = head.split_whitespace().nth(1).unwrap().to_owned();
                let mut upstream = tokio::net::TcpStream::connect(authority).await.unwrap();
                client
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .unwrap();
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn http_proxy_route_reaches_the_server() {
    let harness = Harness::new(DevConfig::demo()).await;
    let proxy = http_proxy().await;
    let via = Via::Proxy {
        protocol: ProxyProtocol::Http,
        host: "127.0.0.1".into(),
        port: proxy,
    };
    let connection = harness
        .connector
        .connect_via(&harness.target(), &demo(), &via, None)
        .await
        .unwrap();
    let output = shell(&connection, b"echo http-ok; exit\n").await;
    assert!(output.contains("http-ok"), "{output}");
}

#[tokio::test]
async fn socks5_proxy_route_reaches_the_server() {
    // RoveTerm's own dynamic forward serves as the SOCKS5 proxy.
    let bastion = Harness::new(DevConfig::demo()).await;
    let inner = DevServer::start("127.0.0.1:0", DevConfig::demo())
        .await
        .unwrap();
    let forwards = Forwards::new(bastion.events.clone());
    let id = ForwardId::new();
    let status = forwards
        .start(
            id,
            HostId::new(),
            bastion.connect().await,
            ForwardSpec::Dynamic {
                bind_host: "127.0.0.1".into(),
                bind_port: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(status.state, ForwardState::Running);
    let socks_port = status.bound_port;
    let via = Via::Proxy {
        protocol: ProxyProtocol::Socks5,
        host: "127.0.0.1".into(),
        port: socks_port,
    };
    let connection = bastion
        .connector
        .connect_via(&target_of(&inner), &demo(), &via, None)
        .await
        .unwrap();
    let output = shell(&connection, b"echo socks-ok; exit\n").await;
    assert!(output.contains("socks-ok"), "{output}");
    forwards.stop(id);
}

#[tokio::test]
async fn progress_reports_each_step_and_the_rejected_credential() {
    use rove_ssh::{AuthKind, ConnectStep};
    let harness = Harness::new(DevConfig::demo()).await;
    let steps = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sink = steps.clone();
    let progress: rove_ssh::Progress = Arc::new(move |step| sink.lock().push(step));

    let wrong = AuthSpec::Password(Zeroizing::new("nope".into()));
    let result = harness
        .connector
        .connect_via(
            &harness.target(),
            &wrong,
            &Via::Direct,
            Some(progress.clone()),
        )
        .await;
    assert!(matches!(result, Err(SshError::AuthFailed)));
    {
        let seen = steps.lock();
        assert!(matches!(seen[0], ConnectStep::Connecting { via: None, .. }));
        assert_eq!(seen[1], ConnectStep::Handshake);
        assert!(matches!(seen[2], ConnectStep::HostKeyAccepted { .. }));
        assert!(seen.contains(&ConnectStep::Rejected {
            method: AuthKind::Password,
            label: None
        }));
        assert!(!seen.contains(&ConnectStep::Authenticated));
    }
    steps.lock().clear();
    harness
        .connector
        .connect_via(&harness.target(), &demo(), &Via::Direct, Some(progress))
        .await
        .unwrap();
    assert_eq!(steps.lock().last(), Some(&ConnectStep::Authenticated));
}

#[tokio::test]
async fn exec_collects_output_status_and_truncates() {
    let harness = Harness::new(DevConfig::demo()).await;
    let connection = harness.connect().await;
    let output = connection
        .exec("echo out; echo err >&2; exit 3", 1024)
        .await
        .unwrap();
    assert_eq!(output.stdout, b"out\n");
    assert_eq!(output.stderr, b"err\n");
    assert_eq!(output.code, Some(3));
    assert!(!output.truncated);
    let long = connection.exec("seq 1 1000", 16).await.unwrap();
    assert_eq!(long.stdout.len(), 16);
    assert!(long.truncated);
}
