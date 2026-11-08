use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use devsshd::{DevConfig, DevServer};
use rove_kernel::{Disposer, EventBus};
use rove_session::{TermSize, TerminalOutput};
use rove_ssh::*;
use rove_vault::{HostKey, MemoryKeyCache, Vault};
use tokio::time::timeout;
use zeroize::Zeroizing;

struct Harness {
    server: DevServer,
    vault: Arc<Vault>,
    prompts: Arc<HostKeyPrompts>,
    connector: SshConnector,
    events: EventBus,
}

impl Harness {
    async fn new(config: DevConfig) -> Self {
        let server = DevServer::start("127.0.0.1:0", config).await.unwrap();
        let vault = Arc::new(Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap());
        let events = EventBus::default();
        let prompts = Arc::new(HostKeyPrompts::new(events.clone(), Duration::from_secs(10)));
        let verifier = Arc::new(VaultVerifier::new(vault.clone(), prompts.clone()));
        let connector = SshConnector::new(&SshConfig::default(), verifier);
        Self {
            server,
            vault,
            prompts,
            connector,
            events,
        }
    }

    fn target(&self) -> SshTarget {
        SshTarget {
            hostname: "127.0.0.1".into(),
            port: self.server.addr().port(),
            username: "demo".into(),
            host_id: None,
            title: None,
        }
    }

    /// Answers every host key prompt with `answer` and counts prompts.
    fn auto_answer(&self, answer: HostKeyAnswer) -> (Disposer, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let prompts = self.prompts.clone();
        let disposer = self.events.on::<HostKeyPrompt>(move |prompt| {
            counter.fetch_add(1, Ordering::SeqCst);
            assert!(prompts.respond(&prompt.request_id, answer));
        });
        (disposer, count)
    }
}

fn password(text: &str) -> AuthSpec {
    AuthSpec::Password(Zeroizing::new(text.into()))
}

async fn run_shell(
    connector: &SshConnector,
    target: &SshTarget,
    auth: &AuthSpec,
    script: &[u8],
) -> (String, Option<i32>) {
    let mut opened = connector
        .open_terminal(target, auth, TermSize::new(90, 20).unwrap())
        .await
        .unwrap();
    opened.session.write(script).await.unwrap();
    let mut text = Vec::new();
    loop {
        match timeout(Duration::from_secs(10), opened.output.recv())
            .await
            .unwrap()
        {
            Some(TerminalOutput::Data(data)) => text.extend_from_slice(&data),
            Some(TerminalOutput::Exit(status)) => {
                return (String::from_utf8_lossy(&text).into_owned(), status.code);
            }
            None => return (String::from_utf8_lossy(&text).into_owned(), None),
        }
    }
}

#[tokio::test]
async fn password_login_runs_shell_after_trusting_unknown_key() {
    let harness = Harness::new(DevConfig::demo()).await;
    let (_answer, prompts) = harness.auto_answer(HostKeyAnswer::Trust);
    let (output, code) = run_shell(
        &harness.connector,
        &harness.target(),
        &password("demo"),
        b"stty size; echo ok-$((1+1)); exit 5\n",
    )
    .await;
    assert!(output.contains("20 90"), "{output}");
    assert!(output.contains("ok-2"), "{output}");
    assert_eq!(code, Some(5));
    assert_eq!(prompts.load(Ordering::SeqCst), 1);

    // The key is now remembered: no second prompt.
    run_shell(
        &harness.connector,
        &harness.target(),
        &password("demo"),
        b"exit\n",
    )
    .await;
    assert_eq!(prompts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn public_key_login() {
    let client_key = devsshd::generate_key();
    let mut config = DevConfig::demo();
    config.password = None;
    config.authorized_keys.push(client_key.public_key().clone());
    let harness = Harness::new(config).await;
    let (_answer, _) = harness.auto_answer(HostKeyAnswer::Trust);
    let auth = AuthSpec::PrivateKey {
        key: Zeroizing::new(
            client_key
                .to_openssh(russh::keys::ssh_key::LineEnding::LF)
                .unwrap()
                .to_string(),
        ),
        passphrase: None,
    };
    let (output, code) = run_shell(
        &harness.connector,
        &harness.target(),
        &auth,
        b"echo key-ok; exit\n",
    )
    .await;
    assert!(output.contains("key-ok"), "{output}");
    assert_eq!(code, Some(0));
}

#[tokio::test]
async fn wrong_password_fails_authentication() {
    let harness = Harness::new(DevConfig::demo()).await;
    let (_answer, _) = harness.auto_answer(HostKeyAnswer::Trust);
    let result = harness
        .connector
        .connect(&harness.target(), &password("nope"))
        .await;
    assert!(
        matches!(result, Err(SshError::AuthFailed)),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn declined_host_key_aborts_and_is_not_remembered() {
    let harness = Harness::new(DevConfig::demo()).await;
    let (_answer, _) = harness.auto_answer(HostKeyAnswer::Reject);
    let result = harness
        .connector
        .connect(&harness.target(), &password("demo"))
        .await;
    assert!(
        matches!(result, Err(SshError::HostKey(HostKeyRejection::Declined))),
        "{:?}",
        result.err()
    );
    let probe = HostKey {
        algorithm: "ssh-ed25519".into(),
        public_key: "x".into(),
        fingerprint: "x".into(),
    };
    assert_eq!(
        harness
            .vault
            .check_host_key("127.0.0.1", harness.target().port, &probe)
            .unwrap(),
        rove_vault::KnownHostStatus::Unknown
    );
}

#[tokio::test]
async fn changed_host_key_is_rejected_without_prompt() {
    let harness = Harness::new(DevConfig::demo()).await;
    let other = devsshd::generate_key();
    let recorded = HostKey {
        algorithm: "ssh-ed25519".into(),
        public_key: other.public_key().to_openssh().unwrap(),
        fingerprint: other
            .public_key()
            .fingerprint(Default::default())
            .to_string(),
    };
    harness
        .vault
        .trust_host_key("127.0.0.1", harness.target().port, &recorded)
        .unwrap();
    let (_answer, prompts) = harness.auto_answer(HostKeyAnswer::Trust);
    let result = harness
        .connector
        .connect(&harness.target(), &password("demo"))
        .await;
    assert!(
        matches!(result, Err(SshError::HostKey(HostKeyRejection::Mismatch { ref expected_fingerprint, .. })) if *expected_fingerprint == recorded.fingerprint),
        "{:?}",
        result.err()
    );
    assert_eq!(prompts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn closed_port_reports_connect_error() {
    let harness = Harness::new(DevConfig::demo()).await;
    let mut target = harness.target();
    let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    target.port = unused.local_addr().unwrap().port();
    drop(unused);
    let result = harness.connector.connect(&target, &password("demo")).await;
    assert!(
        matches!(result, Err(SshError::Connect(_))),
        "{:?}",
        result.err()
    );
}
