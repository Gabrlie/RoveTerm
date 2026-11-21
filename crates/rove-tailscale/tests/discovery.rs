use std::sync::Arc;

use rove_kernel::{Profile, Runtime};
use rove_tailscale::{
    CliDiscovery, TailnetDiscovery, TailnetState, TailscaleConfig, TailscaleError, TailscalePeerId,
    TailscalePlugin,
};

#[test]
fn configuration_is_validated_before_provider_registration() {
    for config in [
        TailscaleConfig {
            program: Some("relative".into()),
            ..TailscaleConfig::default()
        },
        TailscaleConfig {
            timeout_ms: 0,
            ..TailscaleConfig::default()
        },
        TailscaleConfig {
            timeout_ms: 60_001,
            ..TailscaleConfig::default()
        },
        TailscaleConfig {
            max_output_bytes: 1,
            ..TailscaleConfig::default()
        },
        TailscaleConfig {
            max_output_bytes: 64 * 1024 * 1024 + 1,
            ..TailscaleConfig::default()
        },
    ] {
        assert!(CliDiscovery::new(config).is_err());
    }
    assert!(CliDiscovery::new(TailscaleConfig::default()).is_ok());
    let profile = Profile::from_layers([
        "[profile]\nname='test'\nplugins=['tailscale']\n[plugin.tailscale]\ntimeout_ms=0",
    ])
    .unwrap();
    assert!(Runtime::boot(&profile, &[Arc::new(TailscalePlugin)]).is_err());
}

#[test]
fn registration_is_revoked_without_running_the_optional_client() {
    let profile = Profile::from_layers(["[profile]\nname='test'\nplugins=['tailscale']"]).unwrap();
    let runtime = Runtime::boot(&profile, &[Arc::new(TailscalePlugin)]).unwrap();
    let ctx = runtime.context().clone();
    assert!(ctx.get::<dyn TailnetDiscovery>().is_some());
    drop(runtime);
    assert!(ctx.get::<dyn TailnetDiscovery>().is_none());
}

#[test]
fn stable_peer_identity_validation_applies_to_deserialization() {
    assert!(serde_json::from_str::<TailscalePeerId>("\"\"").is_err());
    assert!(serde_json::from_str::<TailscalePeerId>("\"bad id\"").is_err());
    let id = TailscalePeerId::new("nExampleNode".into()).unwrap();
    assert_eq!(id.to_string(), "nExampleNode");
    assert_eq!(serde_json::to_string(&id).unwrap(), "\"nExampleNode\"");
}

#[tokio::test]
async fn configured_missing_client_is_a_nonfatal_state() {
    let directory = tempfile::tempdir().unwrap();
    let provider = CliDiscovery::new(TailscaleConfig {
        program: Some(directory.path().join("not-installed")),
        ..TailscaleConfig::default()
    })
    .unwrap();
    let snapshot = provider.snapshot().await.unwrap();
    assert_eq!(snapshot.state, TailnetState::NotInstalled);
    assert!(snapshot.peers.is_empty());
    assert!(snapshot.captured_at > 0);
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn fake(script: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::Builder::new()
            .prefix("rove tailscale ")
            .tempdir()
            .unwrap();
        let program = directory.path().join("client with spaces");
        std::fs::write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, program)
    }

    fn provider(program: PathBuf) -> CliDiscovery {
        CliDiscovery::new(TailscaleConfig {
            program: Some(program),
            ..TailscaleConfig::default()
        })
        .unwrap()
    }

    #[tokio::test]
    async fn fixed_arguments_and_cli_environment_work_with_spaces_in_the_path() {
        let (_directory, program) = fake(
            r#"
[ "$#" = 2 ] && [ "$1" = status ] && [ "$2" = --json ] && [ "$TAILSCALE_BE_CLI" = 1 ] || exit 2
printf '%s' '{"BackendState":"Running","Peer":{"changing-key":{"ID":"stable-node","HostName":"node","DNSName":"node.example.ts.net.","TailscaleIPs":["100.64.0.1"],"Online":true}}}'
"#,
        );
        let snapshot = provider(program).snapshot().await.unwrap();
        assert_eq!(snapshot.state, TailnetState::Running);
        assert_eq!(snapshot.peers[0].id.as_str(), "stable-node");
        assert_eq!(snapshot.peers[0].dns_name, "node.example.ts.net");
    }

    #[tokio::test]
    async fn nonzero_exit_can_still_return_a_structured_login_state() {
        let (_directory, program) = fake(
            r#"printf '%s' '{"BackendState":"NeedsLogin","AuthURL":"https://example.invalid/secret"}'; exit 1"#,
        );
        let snapshot = provider(program).snapshot().await.unwrap();
        assert_eq!(snapshot.state, TailnetState::NeedsLogin);
        assert!(!serde_json::to_string(&snapshot).unwrap().contains("secret"));
    }

    #[tokio::test]
    async fn permission_and_service_failures_do_not_expose_stderr() {
        for (script, expected) in [
            (
                "printf '%s' 'permission denied: account@example.invalid https://example.invalid/login' >&2; exit 1",
                TailnetState::PermissionDenied,
            ),
            (
                "printf '%s' 'failed to connect to local tailscaled: account@example.invalid' >&2; exit 1",
                TailnetState::ServiceUnavailable,
            ),
        ] {
            let (_directory, program) = fake(script);
            let snapshot = provider(program).snapshot().await.unwrap();
            assert_eq!(snapshot.state, expected);
            let serialized = serde_json::to_string(&snapshot).unwrap();
            assert!(!serialized.contains("account@example.invalid"));
            assert!(!serialized.contains("https://"));
        }
    }

    #[tokio::test]
    async fn an_unexecutable_configured_client_reports_permission_denied() {
        let (_directory, program) = fake("exit 0");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            provider(program).snapshot().await.unwrap().state,
            TailnetState::PermissionDenied
        );
    }

    #[tokio::test]
    async fn success_without_valid_json_is_an_error_without_output_contents() {
        let (_directory, program) = fake("printf '%s' 'account@example.invalid private-output'");
        let error = provider(program).snapshot().await.unwrap_err();
        assert!(matches!(error, TailscaleError::InvalidJson));
        assert!(!error.to_string().contains("private-output"));
    }

    #[tokio::test]
    async fn timeout_ends_the_client_instead_of_waiting_for_pipe_eof() {
        let (_directory, program) = fake("exec sleep 5");
        let provider = CliDiscovery::new(TailscaleConfig {
            program: Some(program),
            timeout_ms: 100,
            ..TailscaleConfig::default()
        })
        .unwrap();
        let start = Instant::now();
        assert!(matches!(
            provider.snapshot().await,
            Err(TailscaleError::Timeout)
        ));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn output_bound_covers_stdout_and_stderr_without_deadlock() {
        let (_directory, program) = fake(
            "while :; do printf '%s' '0123456789012345678901234567890123456789'; printf '%s' '0123456789012345678901234567890123456789' >&2; done",
        );
        let provider = CliDiscovery::new(TailscaleConfig {
            program: Some(program),
            max_output_bytes: 1024,
            ..TailscaleConfig::default()
        })
        .unwrap();
        assert!(matches!(
            provider.snapshot().await,
            Err(TailscaleError::OutputLimit)
        ));
    }

    #[tokio::test]
    async fn cancelling_a_snapshot_drops_and_kills_its_child() {
        let (directory, program) = fake("printf '%s' $$ > \"$0.pid\"; exec sleep 5");
        let pid_file = program.with_file_name("client with spaces.pid");
        let provider = provider(program);
        let task = tokio::spawn(async move { provider.snapshot().await });
        for _ in 0..100 {
            if pid_file.is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let mut gone = false;
        for _ in 0..100 {
            let status = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            if !status.success() {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(gone, "the aborted status child is still running");
        drop(directory);
    }
}
