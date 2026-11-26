use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tauri::{
    AppHandle, Emitter, Runtime,
    ipc::Channel,
    plugin::{PluginApi, PluginHandle, mobile::PluginInvokeError},
};
use zeroize::Zeroizing;

#[cfg(target_os = "android")]
use crate::models::{InstallApkRequest, InstallApkResponse};
use crate::{
    ApkInstallStatus, BACK_EVENT, BiometricStatus, INSETS_EVENT, KeepAliveAction, KeepAliveUpdate,
    LIFECYCLE_EVENT, LifecycleEvent, MobileError, MobileInsets, MobileNetworkSnapshot, StagedFile,
    error::map_invoke_error,
    models::{
        MASTER_KEY_LEN, NativeLifecycleEvent, decode_master_key, encode_base64, parse_external_url,
        parse_stage_uri, sanitize_file_name, validate_local_file_path,
    },
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_rove_mobile);

pub(crate) fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<MobileBridge<R>, MobileError> {
    #[cfg(target_os = "android")]
    let handle = api
        .register_android_plugin("dev.roveterm.mobile", "RoveMobilePlugin")
        .map_err(invoke_error)?;
    #[cfg(target_os = "ios")]
    let handle = api
        .register_ios_plugin(init_plugin_rove_mobile)
        .map_err(invoke_error)?;

    let insets = Arc::new(InsetsState {
        app: app.clone(),
        latest: Mutex::new(MobileInsets::default()),
    });
    let weak_insets = Arc::downgrade(&insets);
    let channel = Channel::<Value>::new(move |payload| {
        let Some(insets) = weak_insets.upgrade() else {
            return Ok(());
        };
        let Ok(value) = payload.deserialize::<MobileInsets>() else {
            tracing::warn!("ignored malformed mobile insets");
            return Ok(());
        };
        let value = value.sanitized();
        *insets.latest.lock().unwrap_or_else(|err| err.into_inner()) = value;
        insets.app.emit(INSETS_EVENT, value)
    });
    handle
        .run_mobile_plugin::<()>("registerInsetsChannel", RegisterChannelArgs { channel })
        .map_err(invoke_error)?;

    // Run bridge callbacks in order, off the native threads that service bridge calls.
    let (dispatch, callbacks) = mpsc::channel::<CallbackTask>();
    std::thread::Builder::new()
        .name("rove-mobile-events".into())
        .spawn(move || {
            for callback in callbacks {
                callback();
            }
        })
        .map_err(|err| MobileError::Unavailable(err.to_string()))?;

    let initial = handle
        .run_mobile_plugin::<MobileNetworkSnapshot>("getNetwork", ())
        .map(MobileNetworkSnapshot::sanitized)
        .unwrap_or_else(|err| {
            tracing::warn!(error = %err, "could not read initial mobile network snapshot");
            MobileNetworkSnapshot::default()
        });
    let network = Arc::new(NetworkChannelState {
        latest: Mutex::new(initial),
        handlers: CallbackHandlers::new(dispatch.clone()),
    });
    let weak_network = Arc::downgrade(&network);
    let channel = Channel::<Value>::new(move |payload| {
        let Some(network) = weak_network.upgrade() else {
            return Ok(());
        };
        let Ok(snapshot) = payload.deserialize::<MobileNetworkSnapshot>() else {
            tracing::warn!("ignored malformed mobile network snapshot");
            return Ok(());
        };
        let snapshot = snapshot.sanitized();
        {
            let mut latest = network.latest.lock().unwrap_or_else(|err| err.into_inner());
            if *latest == snapshot {
                return Ok(());
            }
            *latest = snapshot.clone();
        }
        network.handlers.notify(snapshot);
        Ok(())
    });
    handle
        .run_mobile_plugin::<()>("registerNetworkChannel", RegisterChannelArgs { channel })
        .map_err(invoke_error)?;

    let lifecycle = Arc::new(LifecycleChannelState {
        app: app.clone(),
        keepalive_action_handler: Mutex::new(None),
        handlers: CallbackHandlers::new(dispatch),
    });
    let weak_lifecycle = Arc::downgrade(&lifecycle);
    let channel = Channel::<Value>::new(move |payload| {
        let Some(lifecycle) = weak_lifecycle.upgrade() else {
            return Ok(());
        };
        let Ok(event) = payload.deserialize::<NativeLifecycleEvent>() else {
            tracing::warn!("ignored malformed mobile lifecycle event");
            return Ok(());
        };
        match event {
            NativeLifecycleEvent::Lifecycle { state } => {
                let event = LifecycleEvent { state };
                lifecycle.handlers.notify(event);
                lifecycle.app.emit(LIFECYCLE_EVENT, event)
            }
            NativeLifecycleEvent::Back => lifecycle.app.emit(BACK_EVENT, ()),
            NativeLifecycleEvent::KeepAliveAction { action } => {
                let handler = lifecycle
                    .keepalive_action_handler
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .clone();
                if let Some(handler) = handler {
                    // The handler may call the bridge, which can wait on the native UI thread.
                    tauri::async_runtime::spawn_blocking(move || handler(action));
                }
                Ok(())
            }
        }
    });
    handle
        .run_mobile_plugin::<()>("registerLifecycleChannel", RegisterChannelArgs { channel })
        .map_err(invoke_error)?;

    Ok(MobileBridge {
        handle,
        insets,
        lifecycle,
        network,
    })
}

struct InsetsState<R: Runtime> {
    app: AppHandle<R>,
    latest: Mutex<MobileInsets>,
}

type KeepAliveActionHandler = Arc<dyn Fn(KeepAliveAction) + Send + Sync>;
type CallbackTask = Box<dyn FnOnce() + Send>;
type Callback<T> = Arc<dyn Fn(T) + Send + Sync>;

struct CallbackHandlers<T> {
    dispatch: mpsc::Sender<CallbackTask>,
    handlers: Mutex<BTreeMap<u64, Callback<T>>>,
    next_id: std::sync::atomic::AtomicU64,
}

impl<T: Clone + Send + 'static> CallbackHandlers<T> {
    fn new(dispatch: mpsc::Sender<CallbackTask>) -> Arc<Self> {
        Arc::new(Self {
            dispatch,
            handlers: Mutex::default(),
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    fn insert(&self, handler: impl Fn(T) + Send + Sync + 'static) -> u64 {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.handlers
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(id, Arc::new(handler));
        id
    }

    fn remove(&self, id: u64) {
        let removed = self
            .handlers
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&id);
        drop(removed);
    }

    fn notify(self: &Arc<Self>, event: T) {
        let ids: Vec<_> = self
            .handlers
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .keys()
            .copied()
            .collect();
        if ids.is_empty() {
            return;
        }
        let handlers = Arc::downgrade(self);
        if self
            .dispatch
            .send(Box::new(move || {
                let Some(handlers) = handlers.upgrade() else {
                    return;
                };
                for id in ids {
                    let callback = handlers
                        .handlers
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .get(&id)
                        .cloned();
                    if let Some(callback) = callback {
                        callback(event.clone());
                    }
                }
            }))
            .is_err()
        {
            tracing::warn!("mobile event worker is unavailable");
        }
    }
}

struct NetworkChannelState {
    latest: Mutex<MobileNetworkSnapshot>,
    handlers: Arc<CallbackHandlers<MobileNetworkSnapshot>>,
}

struct LifecycleChannelState<R: Runtime> {
    app: AppHandle<R>,
    keepalive_action_handler: Mutex<Option<KeepAliveActionHandler>>,
    handlers: Arc<CallbackHandlers<LifecycleEvent>>,
}

#[derive(Serialize)]
struct RegisterChannelArgs {
    channel: Channel<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadArgs<'a> {
    service: &'a str,
    silent: bool,
    prompt_title: &'a str,
    prompt_subtitle: &'a str,
}

#[derive(Deserialize)]
struct LoadResponse {
    key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreArgs<'a> {
    service: &'a str,
    key: &'a str,
    biometric: bool,
    prompt_title: &'a str,
    prompt_subtitle: &'a str,
}

#[derive(Serialize)]
struct UrlArgs<'a> {
    url: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StageFileArgs<'a> {
    uri: &'a str,
    dest_dir: &'a str,
    max_bytes: Option<u64>,
}

#[derive(Serialize)]
struct ShareFileArgs<'a> {
    path: &'a str,
    title: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFileArgs<'a> {
    source_path: &'a str,
    suggested_name: String,
}

#[derive(Deserialize)]
struct ExportFileResponse {
    exported: bool,
}

fn invoke_error(error: PluginInvokeError) -> MobileError {
    match error {
        PluginInvokeError::InvokeRejected(response) => {
            map_invoke_error(response.code.as_deref(), response.message.as_deref())
        }
        PluginInvokeError::CannotDeserializeResponse(_) => {
            MobileError::Malformed("invalid native response".into())
        }
        PluginInvokeError::CannotSerializePayload(_) => {
            MobileError::Plugin("cannot serialize native request".into())
        }
        other => MobileError::Plugin(other.to_string()),
    }
}

/// Call operations that can present native UI from a worker thread.
pub struct MobileBridge<R: Runtime> {
    handle: PluginHandle<R>,
    insets: Arc<InsetsState<R>>,
    lifecycle: Arc<LifecycleChannelState<R>>,
    network: Arc<NetworkChannelState>,
}

impl<R: Runtime> MobileBridge<R> {
    pub fn load_master_key(
        &self,
        service: &str,
        silent: bool,
        prompt_title: &str,
        prompt_subtitle: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, MobileError> {
        let response: LoadResponse = self
            .handle
            .run_mobile_plugin(
                "loadMasterKey",
                LoadArgs {
                    service,
                    silent,
                    prompt_title,
                    prompt_subtitle,
                },
            )
            .map_err(invoke_error)?;

        response
            .key
            .map(Zeroizing::new)
            .map(|key| decode_master_key(&key))
            .transpose()
    }

    pub fn store_master_key(
        &self,
        service: &str,
        key: &[u8],
        biometric: bool,
        prompt_title: &str,
        prompt_subtitle: &str,
    ) -> Result<(), MobileError> {
        if key.len() != MASTER_KEY_LEN {
            return Err(MobileError::InvalidInput(
                "invalid master key length".into(),
            ));
        }

        let encoded = encode_base64(key);
        self.handle
            .run_mobile_plugin(
                "storeMasterKey",
                StoreArgs {
                    service,
                    key: &encoded,
                    biometric,
                    prompt_title,
                    prompt_subtitle,
                },
            )
            .map_err(invoke_error)
    }

    pub fn biometric_status(&self) -> Result<BiometricStatus, MobileError> {
        self.handle
            .run_mobile_plugin("biometricStatus", ())
            .map_err(invoke_error)
    }

    pub fn open_external_url(&self, url: &str) -> Result<(), MobileError> {
        let url = parse_external_url(url)?;
        self.handle
            .run_mobile_plugin("openExternalUrl", UrlArgs { url: url.as_str() })
            .map_err(invoke_error)
    }

    #[cfg(target_os = "android")]
    pub fn install_apk(&self, path: &str) -> Result<ApkInstallStatus, MobileError> {
        let args = InstallApkRequest {
            path: validate_local_file_path(path)?,
        };
        let response: InstallApkResponse = self
            .handle
            .run_mobile_plugin("installApk", args)
            .map_err(invoke_error)?;
        Ok(response.status)
    }

    #[cfg(target_os = "ios")]
    pub fn install_apk(&self, _path: &str) -> Result<ApkInstallStatus, MobileError> {
        Err(MobileError::Unsupported)
    }

    pub fn stage_file(
        &self,
        uri: &str,
        dest_dir: &str,
        max_bytes: Option<u64>,
    ) -> Result<StagedFile, MobileError> {
        let args = StageFileArgs {
            uri: parse_stage_uri(uri)?,
            dest_dir: validate_local_file_path(dest_dir)?,
            max_bytes,
        };
        self.handle
            .run_mobile_plugin("stageFile", args)
            .map_err(invoke_error)
    }

    pub fn share_file(&self, path: &str, title: Option<&str>) -> Result<(), MobileError> {
        let args = ShareFileArgs {
            path: validate_local_file_path(path)?,
            title,
        };
        self.handle
            .run_mobile_plugin("shareFile", args)
            .map_err(invoke_error)
    }

    pub fn export_file(
        &self,
        source_path: &str,
        suggested_name: &str,
    ) -> Result<bool, MobileError> {
        let args = ExportFileArgs {
            source_path: validate_local_file_path(source_path)?,
            suggested_name: sanitize_file_name(suggested_name),
        };
        let response: ExportFileResponse = self
            .handle
            .run_mobile_plugin("exportFile", args)
            .map_err(invoke_error)?;
        Ok(response.exported)
    }

    pub fn get_insets(&self) -> Result<MobileInsets, MobileError> {
        Ok(*self
            .insets
            .latest
            .lock()
            .unwrap_or_else(|err| err.into_inner()))
    }

    pub fn get_network(&self) -> MobileNetworkSnapshot {
        self.network
            .latest
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Registers a worker-thread callback for subsequent snapshot changes.
    pub fn on_network_change(
        &self,
        handler: impl Fn(MobileNetworkSnapshot) + Send + Sync + 'static,
    ) -> u64 {
        self.network.handlers.insert(handler)
    }

    pub fn remove_network_change_handler(&self, id: u64) {
        self.network.handlers.remove(id);
    }

    /// Registers a worker-thread callback for subsequent foreground and background events.
    pub fn on_lifecycle(&self, handler: impl Fn(LifecycleEvent) + Send + Sync + 'static) -> u64 {
        self.lifecycle.handlers.insert(handler)
    }

    pub fn remove_lifecycle_handler(&self, id: u64) {
        self.lifecycle.handlers.remove(id);
    }

    pub fn update_keepalive(&self, update: &KeepAliveUpdate) -> Result<(), MobileError> {
        self.handle
            .run_mobile_plugin("updateKeepAlive", update.sanitized())
            .map_err(invoke_error)
    }

    pub fn move_to_background(&self) -> Result<(), MobileError> {
        self.handle
            .run_mobile_plugin("moveToBackground", ())
            .map_err(invoke_error)
    }

    /// Runs notification actions on a worker thread so the handler can call the bridge.
    pub fn set_keepalive_action_handler(&self, handler: Option<KeepAliveActionHandler>) {
        let old = std::mem::replace(
            &mut *self
                .lifecycle
                .keepalive_action_handler
                .lock()
                .unwrap_or_else(|err| err.into_inner()),
            handler,
        );
        drop(old);
    }
}
