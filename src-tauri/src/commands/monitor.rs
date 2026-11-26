//! Server status for the status panel.

use std::time::Duration;

use rove_session::ServerStats;
use rove_ssh::sample_server;
use tauri::State;

use crate::connect::{host_target, quick_target};
use crate::error::CommandError;
use crate::state::AppState;

/// Time allowed for one sample.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerStatsResponse {
    #[serde(flatten)]
    pub stats: ServerStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailscale_path: Option<rove_tailscale::TailnetPath>,
}

#[derive(Debug, PartialEq, Eq)]
enum MonitorTarget<'a> {
    SavedHost(&'a str),
    Address(&'a str),
    Local,
}

fn monitor_target<'a>(
    host_id: Option<&'a str>,
    address: Option<&'a str>,
    local: Option<bool>,
) -> Result<MonitorTarget<'a>, CommandError> {
    match (host_id, address, local == Some(true)) {
        (Some(id), None, false) => Ok(MonitorTarget::SavedHost(id)),
        (None, Some(address), false) => Ok(MonitorTarget::Address(address)),
        (None, None, true) => Ok(MonitorTarget::Local),
        _ => Err(CommandError::new(
            "input.invalid",
            "pass exactly one of host_id, address, and local",
        )),
    }
}

/// Samples the local desktop or an SSH host with a live shared connection.
#[tauri::command]
pub async fn server_stats(
    state: State<'_, AppState>,
    host_id: Option<String>,
    address: Option<String>,
    local: Option<bool>,
) -> Result<ServerStatsResponse, CommandError> {
    let (target, host) = match monitor_target(host_id.as_deref(), address.as_deref(), local)? {
        MonitorTarget::SavedHost(id) => {
            let host = state.vault.host(id.parse()?)?;
            (host_target(&state, &host)?, Some(host))
        }
        MonitorTarget::Address(address) => (quick_target(address)?, None),
        MonitorTarget::Local => {
            #[cfg(desktop)]
            {
                let monitor = state.local_monitor.clone();
                let stats = tokio::time::timeout(
                    SAMPLE_TIMEOUT,
                    tauri::async_runtime::spawn_blocking(move || monitor.sample()),
                )
                .await
                .map_err(|_| CommandError::new("monitor.timeout", "local status timed out"))?
                .map_err(|error| CommandError::new("app.task", error.to_string()))?;
                return Ok(ServerStatsResponse {
                    stats,
                    tailscale_path: None,
                });
            }
            #[cfg(mobile)]
            return Err(CommandError::new(
                "monitor.unsupported",
                "local status is available on desktop",
            ));
        }
    };
    let connection = state
        .ssh
        .live(&target)
        .await
        .ok_or_else(|| CommandError::new("ssh.not_connected", "no live connection"))?;
    let sample = async {
        tokio::time::timeout(SAMPLE_TIMEOUT, sample_server(&connection))
            .await
            .map_err(|_| CommandError::new("ssh.timeout", "server status timed out"))?
            .map_err(|error| match error {
                rove_ssh::SshError::Protocol(message) => {
                    CommandError::new("monitor.unsupported", message)
                }
                other => other.into(),
            })
    };
    let (stats, tailscale_path) = tokio::join!(sample, async {
        let host = host.as_ref()?;
        let rove_vault::HostRoute::Tailscale { peer_id } = &host.route else {
            return None;
        };
        tokio::time::timeout(
            Duration::from_millis(500),
            state
                .tailscale
                .peer_path(&host.hostname, peer_id.as_deref()),
        )
        .await
        .ok()
        .flatten()
    });
    Ok(ServerStatsResponse {
        stats: stats?,
        tailscale_path,
    })
}

/// Detects the operating system of saved host `id` over its live shared connection and stores it
/// for the host's icon. Returns `None` without storing anything when no connection is open; a
/// connected server whose system is not recognised clears the stored one.
#[tauri::command]
pub async fn host_detect_os(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<rove_vault::HostOs>, CommandError> {
    let id: rove_vault::HostId = id.parse()?;
    let target = host_target(&state, &state.vault.host(id)?)?;
    let Some(connection) = state.ssh.live(&target).await else {
        return Ok(None);
    };
    let os = tokio::time::timeout(SAMPLE_TIMEOUT, rove_ssh::probe_os(&connection))
        .await
        .map_err(|_| CommandError::new("ssh.timeout", "system detection timed out"))??;
    state.vault.set_host_os(id, os.clone())?;
    Ok(os)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_each_status_target() {
        assert_eq!(
            monitor_target(Some("saved"), None, None).unwrap(),
            MonitorTarget::SavedHost("saved")
        );
        assert_eq!(
            monitor_target(None, Some("user@example.com"), Some(false)).unwrap(),
            MonitorTarget::Address("user@example.com")
        );
        assert_eq!(
            monitor_target(None, None, Some(true)).unwrap(),
            MonitorTarget::Local
        );
    }

    #[test]
    fn rejects_missing_and_conflicting_status_targets() {
        for (host, address, local) in [
            (None, None, None),
            (None, None, Some(false)),
            (Some("saved"), Some("address"), None),
            (Some("saved"), None, Some(true)),
            (None, Some("address"), Some(true)),
            (Some("saved"), Some("address"), Some(true)),
        ] {
            let error = monitor_target(host, address, local).unwrap_err();
            assert_eq!(error.code, "input.invalid");
            assert_eq!(
                error.message,
                "pass exactly one of host_id, address, and local"
            );
        }
    }
}
