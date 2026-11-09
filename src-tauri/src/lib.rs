//! RoveTerm application shell: assembles the profile's plugins and exposes them to the WebView.

mod commands;
mod connect;
mod error;
mod profile;
mod sink;
mod state;

use std::path::PathBuf;

use rove_kernel::{Context, Runtime};
use rove_ssh::HostKeyPrompt;
use tauri::{Emitter, Manager, RunEvent};

use crate::state::AppState;

/// Event name of forwarded [`HostKeyPrompt`]s.
const HOSTKEY_PROMPT_EVENT: &str = "hostkey-prompt";
/// Event name of vault lock-state changes made outside a command.
const VAULT_STATE_EVENT: &str = "vault-state";
/// Event name of forwarded [`TransferUpdate`](rove_fs::TransferUpdate)s.
const TRANSFER_EVENT: &str = "transfer-update";
/// Event name of forwarded [`ForwardStatus`](rove_ssh::ForwardStatus) changes.
const FORWARD_EVENT: &str = "forward-update";

fn data_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(dir) = std::env::var_os("ROVETERM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(app.path().app_data_dir()?)
}

fn boot(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = data_dir(app)?;
    let user_file = app.path().app_config_dir()?.join("roveterm.toml");
    let profile = profile::load(&data_dir, &user_file)?;
    let ctx = Context::new();
    // Host-provided services; kept for the application lifetime.
    #[cfg_attr(not(any(mobile, debug_assertions)), allow(unused_mut))]
    let mut host_services = Vec::new();
    #[cfg(mobile)]
    host_services.push(
        ctx.provide::<dyn rove_vault::KeyCache>(std::sync::Arc::new(mobile::UnavailableKeyCache))?,
    );
    // Development builds can run without a platform key store (CI, headless X servers); the key
    // lives only in memory, so stored secrets become unreadable after a restart.
    #[cfg(debug_assertions)]
    if std::env::var_os("ROVETERM_DEV_MEMORY_KEYCACHE").is_some() {
        tracing::warn!("using the in-memory development key cache");
        host_services.push(ctx.provide::<dyn rove_vault::KeyCache>(std::sync::Arc::new(
            rove_vault::MemoryKeyCache::default(),
        ))?);
    }
    let runtime = Runtime::boot_in(ctx.clone(), &profile, &profile::catalog())?;
    tracing::info!(profile = profile.name(), data_dir = %data_dir.display(), "runtime booted");

    let state = AppState::new(runtime).map_err(|error| error.message)?;
    match state.vault.device_preferences() {
        Ok(device) => {
            let transfers = state.transfers.clone();
            tauri::async_runtime::spawn(async move {
                transfers.set_max_concurrent(usize::from(device.max_transfers));
            });
        }
        Err(error) => tracing::warn!(%error, "device preferences unavailable, using defaults"),
    }
    let handle = app.handle().clone();
    let forward = ctx.events().on::<HostKeyPrompt>(move |prompt| {
        if let Err(error) = handle.emit(HOSTKEY_PROMPT_EVENT, prompt) {
            tracing::error!(%error, "forwarding host key prompt failed");
        }
    });
    let handle = app.handle().clone();
    let transfers = ctx.events().on::<rove_fs::TransferUpdate>(move |update| {
        if let Err(error) = handle.emit(TRANSFER_EVENT, update) {
            tracing::error!(%error, "forwarding transfer update failed");
        }
    });
    let handle = app.handle().clone();
    let forwards = ctx.events().on::<rove_ssh::ForwardStatus>(move |status| {
        if let Err(error) = handle.emit(FORWARD_EVENT, status) {
            tracing::error!(%error, "forwarding port forward status failed");
        }
    });
    if let Some((_, disposers)) = state.lifetime.lock().expect("lifetime lock").as_mut() {
        disposers.push(forward);
        disposers.push(transfers);
        disposers.push(forwards);
        disposers.extend(host_services);
    }

    let vault = state.vault.clone();
    let handle = app.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let lock_state = vault.try_unlock();
        if let Err(error) = handle.emit(VAULT_STATE_EVENT, lock_state) {
            tracing::error!(%error, "emitting vault state failed");
        }
    });
    app.manage(state);
    Ok(())
}

fn shutdown(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    state.transfers.cancel_all();
    state.forwards.stop_all();
    state.fs_sessions.clear();
    tauri::async_runtime::block_on(async {
        state.sessions.close_all().await;
        state.ssh.disconnect_all().await;
    });
    let lifetime = state.lifetime.lock().map(|mut guard| guard.take());
    if let Ok(Some((runtime, disposers))) = lifetime {
        drop(disposers);
        drop(runtime);
    }
}

/// Starts the Tauri application. Shared by the desktop binary and the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(boot)
        .invoke_handler(tauri::generate_handler![
            commands::session::session_open_local,
            commands::session::session_open_ssh,
            commands::session::session_open_quick,
            commands::session::session_write,
            commands::session::session_resize,
            commands::session::session_close,
            commands::session::session_reattach,
            commands::session::session_list,
            commands::sftp::sftp_open,
            commands::sftp::sftp_list,
            commands::sftp::sftp_mkdir,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_remove,
            commands::sftp::sftp_close,
            commands::sftp::transfer_upload,
            commands::sftp::transfer_download,
            commands::sftp::transfer_local_clashes,
            commands::sftp::transfer_list,
            commands::sftp::transfer_cancel,
            commands::sftp::transfer_clear,
            commands::forward::forward_list,
            commands::forward::forward_create,
            commands::forward::forward_update,
            commands::forward::forward_delete,
            commands::forward::forward_start,
            commands::forward::forward_stop,
            commands::host::host_list,
            commands::host::host_create,
            commands::host::host_update,
            commands::host::host_delete,
            commands::host::host_import_ssh_config,
            commands::host::hostkey_respond,
            commands::vault::vault_status,
            commands::vault::vault_unlock,
            commands::vault::vault_reset_secrets,
            commands::keys::key_list,
            commands::keys::key_generate,
            commands::keys::key_import,
            commands::keys::key_read_file,
            commands::keys::key_update,
            commands::keys::key_delete,
            commands::keys::key_reveal,
            commands::keys::key_install,
            commands::monitor::server_stats,
            commands::snippets::snippet_list,
            commands::snippets::snippet_create,
            commands::snippets::snippet_update,
            commands::snippets::snippet_delete,
            commands::window::window_set_material,
            commands::workspace::workspace_save,
            commands::workspace::workspace_load,
            commands::prefs::prefs_get,
            commands::prefs::device_prefs_get,
            commands::prefs::device_prefs_set,
            commands::prefs::prefs_set,
            commands::prefs::fonts_monospace,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");
    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            shutdown(handle);
        }
    });
}

#[cfg(mobile)]
mod mobile {
    use rove_vault::{Interaction, KeyCache, KeyCacheError, MasterKey};

    /// Placeholder until a Keystore-backed cache exists: secrets stay locked on mobile.
    pub struct UnavailableKeyCache;

    impl KeyCache for UnavailableKeyCache {
        fn load(&self, _interaction: Interaction) -> Result<Option<MasterKey>, KeyCacheError> {
            Err(KeyCacheError::Unavailable(
                "no key store on this platform yet".into(),
            ))
        }

        fn store(&self, _key: &MasterKey) -> Result<(), KeyCacheError> {
            Err(KeyCacheError::Unavailable(
                "no key store on this platform yet".into(),
            ))
        }
    }
}
