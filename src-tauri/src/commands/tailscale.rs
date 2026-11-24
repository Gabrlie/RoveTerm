//! Desktop tailnet lifecycle and browser authorization.

use crate::error::CommandError;

#[cfg(desktop)]
use {
    crate::state::AppState,
    rove_tailscale::{
        ConnectRequest, TailnetPingResult, TailnetSnapshot, TailnetState, TailnetStatus,
        TailscaleError, TailscalePeerId, generate_key_state,
    },
    rove_vault::{LockState, TailscalePrefs},
    tauri::{Manager, State},
    zeroize::Zeroizing,
};

#[cfg(desktop)]
impl From<TailscaleError> for CommandError {
    fn from(error: TailscaleError) -> Self {
        let code = match error {
            TailscaleError::NeedsLogin => "tailscale.needs_login",
            TailscaleError::NotConnected => "tailscale.not_connected",
            TailscaleError::Timeout => "tailscale.timeout",
            TailscaleError::InvalidConfig(_) => "input.invalid",
            TailscaleError::KeyRejected => "tailscale.key_rejected",
            TailscaleError::PeerNotFound => "tailscale.peer_not_found",
            _ => "tailscale.failed",
        };
        Self::new(code, error.to_string())
    }
}

#[cfg(desktop)]
#[tauri::command]
pub fn tailscale_status(state: State<'_, AppState>) -> TailnetStatus {
    state.tailscale.status()
}

#[cfg(desktop)]
#[tauri::command]
pub async fn tailscale_snapshot(
    state: State<'_, AppState>,
) -> Result<TailnetSnapshot, CommandError> {
    Ok(state.tailscale.snapshot().await?)
}

#[cfg(desktop)]
#[tauri::command]
pub async fn tailscale_ping_peer(
    state: State<'_, AppState>,
    peer_id: String,
) -> Result<TailnetPingResult, CommandError> {
    let peer_id = TailscalePeerId::new(peer_id)
        .map_err(|error| CommandError::new("input.invalid", error.to_string()))?;
    Ok(state.tailscale.ping_peer(&peer_id).await?)
}

#[cfg(desktop)]
#[tauri::command]
pub async fn tailscale_connect(
    state: State<'_, AppState>,
    auth_key: Option<String>,
    control_url: Option<String>,
    hostname: Option<String>,
) -> Result<TailnetStatus, CommandError> {
    let auth_key = auth_key
        .map(Zeroizing::new)
        .map(validate_auth_key)
        .transpose()?;
    let _guard = state.tailscale_lock.lock().await;
    state.ensure_unlocked().await?;
    let (key_state_json, prefs) = state.vault.transaction(|| {
        let mut prefs = state.vault.device_preferences()?;
        prefs.tailscale.configured = true;
        if let Some(control_url) = control_url {
            prefs.tailscale.control_url = control_url;
        }
        if let Some(hostname) = hostname {
            prefs.tailscale.hostname = hostname;
        }
        let prefs = state.vault.set_device_preferences(prefs)?;
        let key_state_json = match state.vault.tailscale_state()? {
            Some(bytes) => bytes,
            None => {
                let bytes = generate_key_state().map_err(|_| {
                    rove_vault::VaultError::Invalid("could not encode Tailscale identity".into())
                })?;
                state.vault.set_tailscale_state(Some(&bytes))?;
                bytes
            }
        };
        Ok((key_state_json, prefs.tailscale))
    })?;
    state.tailscale.set_configured(true);
    Ok(state
        .tailscale
        .connect(request(prefs, key_state_json, auth_key))
        .await?)
}

#[cfg(desktop)]
fn validate_auth_key(key: Zeroizing<String>) -> Result<Zeroizing<String>, CommandError> {
    let trimmed = key.trim();
    if trimmed.is_empty()
        || trimmed.len() > 4096
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(CommandError::new(
            "input.invalid",
            "invalid Tailscale auth key",
        ));
    }
    if trimmed.len() == key.len() {
        Ok(key)
    } else {
        Ok(Zeroizing::new(trimmed.to_owned()))
    }
}

#[cfg(desktop)]
#[tauri::command]
pub async fn tailscale_disconnect(
    state: State<'_, AppState>,
) -> Result<TailnetStatus, CommandError> {
    let _guard = state.tailscale_lock.lock().await;
    let configured = state.vault.device_preferences()?.tailscale.configured;
    Ok(state.tailscale.disconnect(configured).await?)
}

#[cfg(desktop)]
#[tauri::command]
pub async fn tailscale_logout(state: State<'_, AppState>) -> Result<TailnetStatus, CommandError> {
    let _guard = state.tailscale_lock.lock().await;
    state.ensure_unlocked().await?;
    let configured = state.vault.device_preferences()?.tailscale.configured;
    state.tailscale.disconnect(configured).await?;
    state.vault.transaction(|| {
        let mut prefs = state.vault.device_preferences()?;
        prefs.tailscale.configured = false;
        state.vault.set_device_preferences(prefs)?;
        state.vault.set_tailscale_state(None)
    })?;
    state.tailscale.set_configured(false);
    Ok(state.tailscale.status())
}

#[cfg(desktop)]
#[tauri::command]
pub fn tailscale_open_auth_url(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let status = state.tailscale.status();
    let url = status
        .auth_url
        .as_deref()
        .filter(|_| status.state == TailnetState::NeedsLogin)
        .and_then(|url| url::Url::parse(url).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
        .ok_or_else(|| {
            CommandError::new("tailscale.no_auth_url", "there is no active browser login")
        })?;
    crate::reveal::open_url(&app, url.as_str())
        .map_err(|_| CommandError::new("app.io", "could not open the system browser"))
}

/// Reconnects the stored identity after unlock, if this installation enables auto-connect.
#[cfg(desktop)]
pub async fn auto_connect(app: &tauri::AppHandle) {
    if let Err(error) = auto_connect_state(&app.state::<AppState>()).await {
        tracing::warn!(code = error.code, "could not start Tailscale automatically");
    }
}

#[cfg(desktop)]
pub(crate) async fn auto_connect_state(state: &AppState) -> Result<(), CommandError> {
    let _guard = state.tailscale_lock.lock().await;
    if !matches!(
        state.tailscale.status().state,
        TailnetState::Disconnected | TailnetState::Unconfigured
    ) || state.vault.lock_state() != LockState::Unlocked
    {
        return Ok(());
    }
    let prefs = state.vault.device_preferences()?.tailscale;
    if prefs.configured
        && prefs.auto_connect
        && let Some(key_state_json) = state.vault.tailscale_state()?
    {
        state
            .tailscale
            .connect(request(prefs, key_state_json, None))
            .await?;
    }
    Ok(())
}

#[cfg(desktop)]
fn request(
    prefs: TailscalePrefs,
    key_state_json: Zeroizing<Vec<u8>>,
    auth_key: Option<Zeroizing<String>>,
) -> ConnectRequest {
    ConnectRequest {
        key_state_json,
        auth_key,
        control_url: (!prefs.control_url.is_empty()).then_some(prefs.control_url),
        hostname: (!prefs.hostname.is_empty()).then_some(prefs.hostname),
    }
}

#[cfg(mobile)]
pub(crate) fn unsupported() -> CommandError {
    CommandError::new(
        "tailscale.unsupported",
        "embedded Tailscale is available on desktop",
    )
}

#[cfg(mobile)]
macro_rules! unsupported_command {
    ($($name:ident),+ $(,)?) => {$(
        #[tauri::command]
        pub fn $name() -> Result<(), CommandError> {
            Err(unsupported())
        }
    )+};
}

#[cfg(mobile)]
unsupported_command!(
    tailscale_status,
    tailscale_snapshot,
    tailscale_ping_peer,
    tailscale_connect,
    tailscale_disconnect,
    tailscale_logout,
    tailscale_open_auth_url,
);
