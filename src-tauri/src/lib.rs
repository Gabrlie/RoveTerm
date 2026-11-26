//! RoveTerm application shell: assembles the profile's plugins and exposes them to the WebView.

mod commands;
mod connect;
mod error;
mod external_edit;
#[cfg(any(mobile, test))]
mod keepalive;
mod logs;
mod mobile_files;
#[cfg(any(mobile, test))]
mod mobile_key_cache;
mod ops_runtime;
mod ops_tools;
mod profile;
mod reveal;
#[cfg(desktop)]
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

fn resolve_data_dir(
    override_dir: Option<std::ffi::OsString>,
    app_data_dir: PathBuf,
    debug: bool,
    desktop: bool,
) -> std::io::Result<PathBuf> {
    if let Some(dir) = override_dir.filter(|dir| !dir.is_empty()) {
        return std::path::absolute(dir);
    }
    if !debug {
        return Ok(app_data_dir);
    }
    if desktop && let Some(name) = app_data_dir.file_name() {
        let mut dev_name = name.to_os_string();
        dev_name.push(".dev");
        return Ok(app_data_dir.with_file_name(dev_name));
    }
    Ok(app_data_dir.join("dev"))
}

fn data_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(resolve_data_dir(
        std::env::var_os("ROVETERM_DATA_DIR"),
        app.path().app_data_dir()?,
        cfg!(debug_assertions),
        cfg!(desktop),
    )?)
}

#[cfg(any(mobile, test))]
fn host_network_snapshot(
    snapshot: tauri_plugin_rove_mobile::MobileNetworkSnapshot,
) -> rove_tailscale::HostNetworkSnapshot {
    use rove_tailscale::{HostNetworkAddr, HostNetworkInterface, HostNetworkSnapshot};

    HostNetworkSnapshot {
        interfaces: snapshot
            .interfaces
            .into_iter()
            .map(|interface| HostNetworkInterface {
                id: interface.id,
                name: interface.name,
                addrs: interface
                    .addrs
                    .into_iter()
                    .filter_map(|addr| {
                        Some(HostNetworkAddr {
                            ip: addr.ip.parse::<std::net::IpAddr>().ok()?,
                            prefix_len: addr.prefix_len,
                        })
                    })
                    .collect(),
                default_v4: interface.default_v4,
                default_v6: interface.default_v6,
            })
            .collect(),
    }
}

fn boot(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = data_dir(app)?;
    let staging_root = app.path().app_cache_dir();
    #[cfg(desktop)]
    let staging_root = staging_root.unwrap_or_else(|_| std::env::temp_dir());
    #[cfg(mobile)]
    let staging_root = staging_root?;
    let staging_root = staging_root.join("rove-staged");
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
    let mobile_cache = std::sync::Arc::new(mobile_key_cache::MobileKeyCache::native(
        app.handle().clone(),
    ));
    #[cfg(mobile)]
    host_services.push(ctx.provide::<dyn rove_vault::KeyCache>(mobile_cache.clone())?);
    // Development builds can run without a platform key store (CI, headless X servers); the key
    // lives only in memory, so stored secrets become unreadable after a restart.
    #[cfg(all(desktop, debug_assertions))]
    if std::env::var_os("ROVETERM_DEV_MEMORY_KEYCACHE").is_some() {
        tracing::warn!("using the in-memory development key cache");
        host_services.push(ctx.provide::<dyn rove_vault::KeyCache>(std::sync::Arc::new(
            rove_vault::MemoryKeyCache::default(),
        ))?);
    }
    let runtime = Runtime::boot_in(ctx.clone(), &profile, &profile::catalog())?;
    tracing::info!(profile = profile.name(), data_dir = %data_dir.display(), "runtime booted");

    let state = AppState::new(runtime, &data_dir, staging_root).map_err(|error| error.message)?;
    state.mobile_files.sweep_all();
    match state.vault.device_preferences() {
        Ok(device) => {
            #[cfg(mobile)]
            mobile_cache.set_biometric(device.vault_biometric);
            commands::prefs::apply_connection(&state, &device.connection);
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
    let staged_uploads = mobile_files::watch_transfers(ctx.events(), state.mobile_files.clone());
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
        disposers.push(staged_uploads);
        disposers.push(transfers);
        disposers.push(forwards);
        disposers.extend(host_services);
    }

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

    if let Some((_, disposers)) = state.lifetime.lock().expect("lifetime lock").as_mut() {
        disposers.push(tailnet_events);
    }
    #[cfg(mobile)]
    {
        use tauri_plugin_rove_mobile::{LifecycleState, MobileBridgeExt};

        let mobile = app.rove_mobile();
        let network_handler = {
            // Apply the initial snapshot before publishing updates from the callback.
            let updates = std::sync::Arc::new(parking_lot::Mutex::new(()));
            let _initial = updates.lock();
            let updates = updates.clone();
            let tailscale = state.tailscale.clone();
            let id = mobile.on_network_change(move |snapshot| {
                let _update = updates.lock();
                tailscale.update_network(host_network_snapshot(snapshot));
            });
            state
                .tailscale
                .update_network(host_network_snapshot(mobile.get_network()));
            let handle = app.handle().clone();
            rove_kernel::Disposer::new(move || {
                handle.rove_mobile().remove_network_change_handler(id);
            })
        };
        let tailscale = state.tailscale.clone();
        let id = mobile.on_lifecycle(move |event| {
            if event.state == LifecycleState::Foreground {
                tailscale.notify_foreground();
            }
        });
        let handle = app.handle().clone();
        let lifecycle_handler = rove_kernel::Disposer::new(move || {
            handle.rove_mobile().remove_lifecycle_handler(id);
        });
        let keepalive = keepalive::start(app.handle().clone(), &ctx, &state);
        let mut lifetime = state
            .lifetime
            .lock()
            .map_err(|_| std::io::Error::other("application lifetime lock poisoned"))?;
        if let Some((_, disposers)) = lifetime.as_mut() {
            disposers.push(network_handler);
            disposers.push(lifecycle_handler);
            disposers.extend(keepalive);
        }
    }
    let vault = state.vault.clone();
    #[cfg(mobile)]
    app.manage(mobile_cache);
    app.manage(state);
    let handle = app.handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let lock_state = vault.try_unlock();
        if let Err(error) = handle.emit(VAULT_STATE_EVENT, &lock_state) {
            tracing::error!(%error, "emitting vault state failed");
        }
        if lock_state == rove_vault::LockState::Unlocked {
            tauri::async_runtime::spawn(async move {
                commands::tailscale::auto_connect(&handle).await;
            });
        }
    });
    #[cfg(desktop)]
    {
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            commands::external::restore(&handle).await;
        });
        schedule::start(app.handle());
        commands::autostart::apply(app);
    }
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

pub(crate) async fn shutdown_async(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    if state
        .shutting_down
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return;
    }
    state.external_edits.shutdown().await;
    state.transfers.cancel_all();
    state.mobile_files.sweep_all();
    state.forwards.stop_all();
    state.fs_sessions.clear();
    #[cfg(desktop)]
    state.external.disable().await;
    if let Err(error) = state.ops.shutdown() {
        tracing::error!(%error, "operations shutdown failed");
    }
    state.ai.stop_all().await;
    state.sessions.close_all().await;
    state.ssh.disconnect_all().await;
    {
        let _guard = state.tailscale_lock.lock().await;
        let configured = state.tailscale.status().configured;
        if let Err(error) = state.tailscale.disconnect(configured).await {
            tracing::warn!(%error, "Tailscale shutdown failed");
        }
    }
    let lifetime = state.lifetime.lock().map(|mut guard| guard.take());
    if let Ok(Some((runtime, disposers))) = lifetime {
        drop(disposers);
        drop(runtime);
    }
}

fn shutdown(app: &tauri::AppHandle) {
    tauri::async_runtime::block_on(shutdown_async(app));
}

/// Runs the MCP bridge for an agent: connects stdin and stdout to the running app named by the
/// environment. Returns the process exit code.
#[cfg(desktop)]
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

#[cfg(mobile)]
pub fn mcp_bridge() -> i32 {
    eprintln!("roveterm --mcp-bridge: external MCP access is not available on this platform");
    1
}

/// Starts the Tauri application. Shared by the desktop binary and the mobile entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logs::init();
    rove_http::install_crypto_provider();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_rove_mobile::init())
        .plugin(tauri_plugin_clipboard_manager::init());
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![commands::autostart::MINIMIZED_ARG]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_menu_event(commands::menu::on_event);
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
            commands::ops::ops_subtask_cancel,
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
            commands::sftp::transfer_prepare_upload,
            commands::sftp::transfer_discard_staged,
            commands::sftp::transfer_upload,
            commands::sftp::transfer_download,
            commands::sftp::transfer_local_clashes,
            commands::sftp::transfer_list,
            commands::sftp::transfer_cancel,
            commands::sftp::transfer_clear,
            commands::sftp::transfer_reveal,
            commands::sftp::transfer_export,
            commands::app::log_write,
            commands::autostart::autostart_get,
            commands::autostart::autostart_set,
            commands::host::host_notes_get,
            commands::host::host_notes_set,
            commands::app::logs_reveal,
            commands::app::logs_list,
            commands::app::logs_read,
            commands::app::mobile_insets,
            commands::app::mobile_back_default,
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
            commands::vault::vault_biometric_status,
            commands::vault::vault_set_biometric,
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
            commands::ai::ai_acp_auth_methods,
            commands::ai::ai_acp_authenticate,
            commands::ai::ai_acp_cancel_auth,
            commands::ai::ai_acp_login_terminal,
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
            commands::update::update_check,
            commands::update::update_install,
            commands::update::update_restart,
            commands::update::update_open_release,
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

#[cfg(test)]
mod tests {
    use super::*;
    use rove_tailscale::{HostNetworkAddr, HostNetworkInterface};
    use std::ffi::OsString;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use tauri_plugin_rove_mobile::{
        MobileNetworkAddr, MobileNetworkInterface, MobileNetworkSnapshot,
    };

    #[test]
    fn converts_mobile_ipv4_and_ipv6_addresses() {
        let snapshot = host_network_snapshot(MobileNetworkSnapshot {
            interfaces: vec![MobileNetworkInterface {
                id: 7,
                name: "wlan0".into(),
                addrs: vec![
                    MobileNetworkAddr {
                        ip: "192.0.2.42".into(),
                        prefix_len: 24,
                    },
                    MobileNetworkAddr {
                        ip: "2001:db8::42".into(),
                        prefix_len: 64,
                    },
                ],
                default_v4: true,
                default_v6: false,
            }],
        });
        assert_eq!(
            snapshot.interfaces,
            vec![HostNetworkInterface {
                id: 7,
                name: "wlan0".into(),
                addrs: vec![
                    HostNetworkAddr {
                        ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 42)),
                        prefix_len: 24,
                    },
                    HostNetworkAddr {
                        ip: IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x42)),
                        prefix_len: 64,
                    },
                ],
                default_v4: true,
                default_v6: false,
            }]
        );
    }

    #[test]
    fn skips_invalid_mobile_ip_addresses() {
        let snapshot = host_network_snapshot(MobileNetworkSnapshot {
            interfaces: vec![MobileNetworkInterface {
                id: 9,
                name: "rmnet0".into(),
                addrs: vec![
                    MobileNetworkAddr {
                        ip: "invalid-ip".into(),
                        prefix_len: 24,
                    },
                    MobileNetworkAddr {
                        ip: "2001:db8::1".into(),
                        prefix_len: 64,
                    },
                ],
                default_v4: false,
                default_v6: true,
            }],
        });
        assert_eq!(
            snapshot.interfaces[0].addrs,
            vec![HostNetworkAddr {
                ip: IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
                prefix_len: 64,
            }]
        );
        assert!(snapshot.interfaces[0].default_v6);
    }

    #[test]
    fn empty_mobile_snapshot_signals_disconnection() {
        let snapshot = host_network_snapshot(MobileNetworkSnapshot::default());
        assert!(snapshot.interfaces.is_empty());
    }

    #[test]
    fn data_dir_isolates_debug_builds_from_production() {
        let prod = PathBuf::from("/tmp/app-support/dev.roveterm.app");
        assert_eq!(
            resolve_data_dir(None, prod.clone(), false, true).unwrap(),
            prod
        );
        assert_eq!(
            resolve_data_dir(None, prod.clone(), true, true).unwrap(),
            PathBuf::from("/tmp/app-support/dev.roveterm.app.dev")
        );
        assert_eq!(
            resolve_data_dir(Some(OsString::new()), prod.clone(), true, true).unwrap(),
            PathBuf::from("/tmp/app-support/dev.roveterm.app.dev")
        );
        assert_eq!(
            resolve_data_dir(None, PathBuf::from("/data/user/0/dev.roveterm.app/files"), true, false)
                .unwrap(),
            PathBuf::from("/data/user/0/dev.roveterm.app/files/dev")
        );
        assert_eq!(
            resolve_data_dir(Some(OsString::from("/tmp/custom-dir")), prod, true, true).unwrap(),
            PathBuf::from("/tmp/custom-dir")
        );
    }
}
