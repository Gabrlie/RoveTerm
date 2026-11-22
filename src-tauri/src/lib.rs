//! RoveTerm application shell: assembles the profile's plugins and exposes them to the WebView.

mod commands;
mod connect;
mod error;
mod external_edit;
mod logs;
mod ops_runtime;
mod ops_tools;
mod ops_verifier;
mod profile;
mod reveal;
mod schedule;
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
        return Ok(std::path::absolute(dir)?);
    }
    Ok(app.path().app_data_dir()?)
}

fn boot(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = data_dir(app)?;
    // Without log files the app still runs; the terminal keeps the log.
    if let Err(error) = logs::start_files(&data_dir) {
        tracing::warn!(%error, "log files unavailable");
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");
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

    let state = AppState::new(runtime, &data_dir).map_err(|error| error.message)?;
    match state.vault.device_preferences() {
        Ok(device) => {
            commands::prefs::apply_connection(&state, &device.connection);
            #[cfg(desktop)]
            state.tailscale.set_configured(device.tailscale.configured);
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

    #[cfg(desktop)]
    let tailnet_events = {
        let handle = app.handle().clone();
        let mut status = state.tailscale.subscribe();
        let task = tauri::async_runtime::spawn(async move {
            while status.changed().await.is_ok() {
                let status = status.borrow_and_update().clone();
                if let Err(error) = handle.emit("tailscale-status", status) {
                    tracing::warn!(%error, "emitting Tailscale status failed");
                }
            }
        });
        rove_kernel::Disposer::new(move || task.abort())
    };

    #[cfg(desktop)]
    if let Some((_, disposers)) = state.lifetime.lock().expect("lifetime lock").as_mut() {
        disposers.push(tailnet_events);
    }
    let vault = state.vault.clone();
    app.manage(state);
    let handle = app.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let lock_state = vault.try_unlock();
        if let Err(error) = handle.emit(VAULT_STATE_EVENT, &lock_state) {
            tracing::error!(%error, "emitting vault state failed");
        }
        #[cfg(desktop)]
        if lock_state == rove_vault::LockState::Unlocked {
            tauri::async_runtime::spawn(async move {
                commands::tailscale::auto_connect(&handle).await;
            });
        }
    });
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        commands::external::restore(&handle).await;
    });
    schedule::start(app.handle());
    commands::autostart::apply(app);
    Ok(())
}

fn request_edit_exit_confirmation(app: &tauri::AppHandle) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    if !state.external_edits.needs_exit_confirmation()
        && !state
            .external_edit_dirty
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return false;
    }
    match app.emit_to("main", "external-edit-exit-request", ()) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "could not deliver the file edit exit request");
            false
        }
    }
}

fn shutdown(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    tauri::async_runtime::block_on(state.external_edits.shutdown());
    state.transfers.cancel_all();
    state.forwards.stop_all();
    // The config file names a port that is about to close.
    state.fs_sessions.clear();
    tauri::async_runtime::block_on(async {
        state.external.disable().await;
        if let Err(error) = state.ops.shutdown() {
            tracing::error!(%error, "operations shutdown failed");
        }
        state.ai.stop_all().await;
        state.sessions.close_all().await;
        state.ssh.disconnect_all().await;
        #[cfg(desktop)]
        {
            let _guard = state.tailscale_lock.lock().await;
            let configured = state.tailscale.status().configured;
            if let Err(error) = state.tailscale.disconnect(configured).await {
                tracing::warn!(%error, "Tailscale shutdown failed");
            }
        }
    });
    let lifetime = state.lifetime.lock().map(|mut guard| guard.take());
    if let Ok(Some((runtime, disposers))) = lifetime {
        drop(disposers);
        drop(runtime);
    }
}

/// Runs the MCP bridge for an agent: connects stdin and stdout to the running app named by the
/// environment. Returns the process exit code.
pub fn mcp_bridge() -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("roveterm --mcp-bridge: {error}");
            return 1;
        }
    };
    // `--mcp-bridge --mcp-config <file>`: an agent the user runs, through the config file.
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let config = args
        .iter()
        .position(|arg| arg == "--mcp-config")
        .and_then(|at| args.get(at + 1))
        .map(PathBuf::from);
    match runtime.block_on(rove_ai::mcp::bridge(config.as_deref())) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("roveterm --mcp-bridge: {error}");
            1
        }
    }
}

/// Starts the Tauri application. Shared by the desktop binary and the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logs::init();
    rove_http::install_crypto_provider();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec![commands::autostart::MINIMIZED_ARG]),
    ));
    let app = builder
        .setup(boot)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && request_edit_exit_confirmation(window.app_handle())
            {
                api.prevent_close();
            }
        })
        .on_menu_event(commands::menu::on_event)
        .invoke_handler(tauri::generate_handler![
            commands::session::session_open_local,
            commands::tailscale::tailscale_status,
            commands::tailscale::tailscale_snapshot,
            commands::tailscale::tailscale_ping_peer,
            commands::tailscale::tailscale_connect,
            commands::tailscale::tailscale_disconnect,
            commands::tailscale::tailscale_logout,
            commands::tailscale::tailscale_open_auth_url,
            commands::ops::ops_task_list,
            commands::ops::ops_task_get,
            commands::ops::ops_task_input,
            commands::ops::ops_task_cancel,
            commands::ops::ops_task_retry,
            commands::ops::ops_task_evidence,
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
            commands::sftp::sftp_read_text,
            commands::sftp::sftp_save_text,
            commands::sftp::sftp_retain,
            commands::external_edit::external_edit_start,
            commands::external_edit::external_edit_list,
            commands::external_edit::external_edit_open_copy,
            commands::external_edit::external_edit_remote_version,
            commands::external_edit::external_edit_retry,
            commands::external_edit::external_edit_finish,
            commands::external_edit::external_edit_discard,
            commands::external_edit::external_edit_reveal,
            commands::external_edit::external_edit_set_dirty,
            commands::external_edit::external_edit_shutdown,
            commands::sftp::transfer_upload,
            commands::sftp::transfer_download,
            commands::sftp::transfer_local_clashes,
            commands::sftp::transfer_list,
            commands::sftp::transfer_cancel,
            commands::sftp::transfer_clear,
            commands::sftp::transfer_reveal,
            commands::app::log_write,
            commands::autostart::autostart_get,
            commands::autostart::autostart_set,
            commands::host::host_notes_get,
            commands::host::host_notes_set,
            commands::app::logs_reveal,
            commands::app::logs_list,
            commands::app::logs_read,
            commands::schedules::schedule_status,
            commands::schedules::schedule_claim,
            commands::schedules::schedule_device,
            commands::schedules::schedule_run_now,
            commands::external::external_status,
            commands::external::external_set,
            commands::external::external_install,
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
            commands::keys::key_load_file,
            commands::keys::key_default_files,
            commands::keys::key_update,
            commands::keys::key_delete,
            commands::keys::key_reveal,
            commands::keys::key_install,
            commands::monitor::server_stats,
            commands::monitor::host_detect_os,
            commands::snippets::snippet_list,
            commands::snippets::snippet_create,
            commands::snippets::snippet_update,
            commands::snippets::snippet_delete,
            commands::ai::ai_agents,
            commands::ai::ai_provider_test,
            commands::ai::ai_set_key,
            commands::ai::ai_reveal_key,
            commands::ai::ai_start,
            commands::ai::ai_prompt,
            commands::ai::ai_cancel,
            commands::ai::ai_answer,
            commands::ai::ai_set_mode,
            commands::ai::ai_stop,
            commands::ai::ai_sessions,
            commands::ai::ai_records,
            commands::ai::ai_audit_detail,
            commands::ai::ai_rename,
            commands::ai::ai_set_option,
            commands::ai::ai_select_model,
            commands::ai::ai_quota,
            commands::ai::ai_provider_models,
            commands::ai::ai_delete,
            commands::ai::ai_set_review_key,
            commands::ai::ai_reveal_review_key,
            commands::sync::sync_status,
            commands::sync::sync_connect,
            commands::sync::sync_probe,
            commands::sync::sync_setup,
            commands::sync::sync_now,
            commands::sync::sync_reveal_secret,
            commands::sync::sync_disable,
            commands::menu::menu_set,
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
    app.run(|handle, event| match event {
        RunEvent::ExitRequested { api, .. } if request_edit_exit_confirmation(handle) => {
            api.prevent_exit();
        }
        RunEvent::Exit => shutdown(handle),
        _ => {}
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
