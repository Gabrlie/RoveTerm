//! Server status for the status panel.

use std::time::Duration;

use rove_ssh::{ServerStats, sample_server};
use tauri::State;

use crate::connect::{host_target, quick_target};
use crate::error::CommandError;
use crate::state::AppState;

/// Time allowed for one sample.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(8);

/// Samples the server of a saved host (`host_id`) or quick-connect `address` over its live shared
/// connection. Never connects or asks for credentials: fails with `ssh.not_connected` when no
/// terminal or file panel holds a connection.
#[tauri::command]
pub async fn server_stats(
    state: State<'_, AppState>,
    host_id: Option<String>,
    address: Option<String>,
) -> Result<ServerStats, CommandError> {
    let target = match (host_id, address) {
        (Some(id), None) => host_target(&state, &state.vault.host(id.parse()?)?)?,
        (None, Some(address)) => quick_target(&address)?,
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
    tokio::time::timeout(SAMPLE_TIMEOUT, sample_server(&connection))
        .await
        .map_err(|_| CommandError::new("ssh.timeout", "server status timed out"))?
        .map_err(|error| match error {
            rove_ssh::SshError::Protocol(message) => {
                CommandError::new("monitor.unsupported", message)
            }
            other => other.into(),
        })
}
