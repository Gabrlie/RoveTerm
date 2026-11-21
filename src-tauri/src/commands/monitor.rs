//! Server status for the status panel.

use std::time::Duration;

use rove_ssh::{ServerStats, sample_server};
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
    #[cfg(desktop)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailscale_path: Option<rove_tailscale::TailnetPath>,
}

/// Samples the server of a saved host (`host_id`) or quick-connect `address` over its live shared
/// connection. Never connects or asks for credentials: fails with `ssh.not_connected` when no
/// terminal or file panel holds a connection.
#[tauri::command]
pub async fn server_stats(
    state: State<'_, AppState>,
    host_id: Option<String>,
    address: Option<String>,
) -> Result<ServerStatsResponse, CommandError> {
    let (target, _host) = match (host_id, address) {
        (Some(id), None) => {
            let host = state.vault.host(id.parse()?)?;
            (host_target(&state, &host)?, Some(host))
        }
        (None, Some(address)) => (quick_target(&address)?, None),
        _ => {
            return Err(CommandError::new(
                "input.invalid",
                "pass exactly one of host_id and address",
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
    #[cfg(desktop)]
    let (stats, tailscale_path) = tokio::join!(sample, async {
        let host = _host.as_ref()?;
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
    #[cfg(mobile)]
    let stats = sample.await;
    Ok(ServerStatsResponse {
        stats: stats?,
        #[cfg(desktop)]
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
