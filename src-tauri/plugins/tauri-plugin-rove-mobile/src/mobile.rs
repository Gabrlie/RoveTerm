use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tauri::{
    AppHandle, Emitter, Runtime,
    ipc::Channel,
    plugin::{PluginApi, PluginHandle, mobile::PluginInvokeError},
};
use zeroize::Zeroizing;

use crate::{
    BACK_EVENT, BiometricStatus, INSETS_EVENT, KeepAliveAction, KeepAliveUpdate, LIFECYCLE_EVENT,
    LifecycleEvent, MobileError, MobileInsets, StagedFile,
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

    let lifecycle = Arc::new(LifecycleChannelState {
        app: app.clone(),
        keepalive_action_handler: Mutex::new(None),
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
            NativeLifecycleEvent::Lifecycle { state } => lifecycle
                .app
                .emit(LIFECYCLE_EVENT, LifecycleEvent { state }),
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
    })
}

struct InsetsState<R: Runtime> {
    app: AppHandle<R>,
    latest: Mutex<MobileInsets>,
}

type KeepAliveActionHandler = Arc<dyn Fn(KeepAliveAction) + Send + Sync>;

struct LifecycleChannelState<R: Runtime> {
    app: AppHandle<R>,
    keepalive_action_handler: Mutex<Option<KeepAliveActionHandler>>,
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
