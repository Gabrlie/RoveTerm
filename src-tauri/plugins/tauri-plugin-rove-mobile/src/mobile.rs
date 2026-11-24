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
    BiometricStatus, INSETS_EVENT, MobileError, MobileInsets,
    error::map_invoke_error,
    models::{MASTER_KEY_LEN, decode_master_key, encode_base64, parse_external_url},
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
        .run_mobile_plugin::<()>("registerInsetsChannel", RegisterInsetsArgs { channel })
        .map_err(invoke_error)?;

    Ok(MobileBridge { handle, insets })
}

struct InsetsState<R: Runtime> {
    app: AppHandle<R>,
    latest: Mutex<MobileInsets>,
}

#[derive(Serialize)]
struct RegisterInsetsArgs {
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

    pub fn get_insets(&self) -> Result<MobileInsets, MobileError> {
        Ok(*self
            .insets
            .latest
            .lock()
            .unwrap_or_else(|err| err.into_inner()))
    }
}
