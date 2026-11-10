//! Host operations on a remote host, against devsshd.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use devsshd::{DevConfig, DevServer};
use rove_ai::host::{
    ConnectionSource, Host, HostError, Limits, RemoteHost, list_dir, read_text, write_text,
};
use rove_kernel::EventBus;
use rove_ssh::{
    AuthSpec, HostKeyAnswer, HostKeyPrompt, HostKeyPrompts, SshConfig, SshConnection, SshConnector,
    SshTarget, VaultVerifier,
};
use rove_vault::{MemoryKeyCache, Vault};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

/// Hands out a connection, or none once `close` has been called.
struct Source(Mutex<Option<Arc<SshConnection>>>);

#[async_trait]
impl ConnectionSource for Source {
    async fn connection(&self) -> Option<Arc<SshConnection>> {
        self.0.lock().await.clone()
    }
}

async fn connect(root: &std::path::Path) -> (DevServer, Arc<SshConnection>, rove_kernel::Disposer) {
    let config = DevConfig {
        sftp_root: Some(root.to_path_buf()),
        ..DevConfig::demo()
    };
    let server = DevServer::start("127.0.0.1:0", config).await.unwrap();
    let vault = Arc::new(Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap());
    let events = EventBus::default();
    let prompts = Arc::new(HostKeyPrompts::new(events.clone(), Duration::from_secs(10)));
    let responder = prompts.clone();
    let listener = events.on::<HostKeyPrompt>(move |prompt| {
        responder.respond(&prompt.request_id, HostKeyAnswer::Trust);
    });
    let connector = SshConnector::new(
        &SshConfig::default(),
        Arc::new(VaultVerifier::new(vault, prompts)),
    );
    let target = SshTarget {
        hostname: "127.0.0.1".into(),
        port: server.addr().port(),
        username: "demo".into(),
        host_id: None,
        title: None,
        route: String::new(),
    };
    let connection = connector
        .connect(&target, &AuthSpec::Password(Zeroizing::new("demo".into())))
        .await
        .unwrap();
    (server, connection, listener)
}

#[tokio::test]
async fn runs_commands_and_edits_files_on_the_remote_host() {
    let root = tempfile::tempdir().unwrap();
    let (_server, connection, _listener) = connect(root.path()).await;
    let source = Arc::new(Source(Mutex::new(Some(connection))));
    let host = RemoteHost::new("demo@127.0.0.1", source.clone());
    let limits = Limits::default();
    let dir = root.path().canonicalize().unwrap();
    let dir = dir.to_str().unwrap();

    let output = host
        .run("echo hi; echo oops >&2; pwd; exit 2", Some(dir), &limits)
        .await
        .unwrap();
    assert_eq!(output.stdout, format!("hi\n{dir}\n"));
    assert_eq!(output.stderr, "oops\n");
    assert_eq!(output.exit_code, Some(2));

    let path = format!("{dir}/app.conf");
    write_text(&host, &path, "port = 80\n", &limits)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("app.conf")).unwrap(),
        "port = 80\n"
    );
    assert_eq!(
        read_text(&host, &path, &limits).await.unwrap().content,
        "port = 80\n"
    );
    let (entries, _) = list_dir(&host, dir, &limits).await.unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["app.conf"]
    );

    // Without a connection the host says so instead of connecting.
    *source.0.lock().await = None;
    assert_eq!(
        host.run("true", None, &limits).await,
        Err(HostError::NotConnected)
    );
}
