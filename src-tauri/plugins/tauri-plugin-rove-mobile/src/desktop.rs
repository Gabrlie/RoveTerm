use std::{marker::PhantomData, sync::Arc};

use tauri::Runtime;
use zeroize::Zeroizing;

use crate::{
    BiometricStatus, KeepAliveAction, KeepAliveUpdate, MobileError, MobileInsets, StagedFile,
    models::{parse_stage_uri, sanitize_file_name, validate_local_file_path},
};

pub(crate) fn init<R: Runtime>() -> MobileBridge<R> {
    MobileBridge(PhantomData)
}

pub struct MobileBridge<R: Runtime>(PhantomData<fn() -> R>);

impl<R: Runtime> MobileBridge<R> {
    pub fn load_master_key(
        &self,
        _service: &str,
        _silent: bool,
        _prompt_title: &str,
        _prompt_subtitle: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, MobileError> {
        Err(MobileError::Unsupported)
    }

    pub fn store_master_key(
        &self,
        _service: &str,
        _key: &[u8],
        _biometric: bool,
        _prompt_title: &str,
        _prompt_subtitle: &str,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    pub fn biometric_status(&self) -> Result<BiometricStatus, MobileError> {
        Ok(BiometricStatus {
            supported: false,
            enrolled: false,
        })
    }

    pub fn open_external_url(&self, _url: &str) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    pub fn stage_file(
        &self,
        uri: &str,
        dest_dir: &str,
        _max_bytes: Option<u64>,
    ) -> Result<StagedFile, MobileError> {
        parse_stage_uri(uri)?;
        validate_local_file_path(dest_dir)?;
        Err(MobileError::Unsupported)
    }

    pub fn share_file(&self, path: &str, _title: Option<&str>) -> Result<(), MobileError> {
        validate_local_file_path(path)?;
        Err(MobileError::Unsupported)
    }

    pub fn export_file(
        &self,
        source_path: &str,
        suggested_name: &str,
    ) -> Result<bool, MobileError> {
        validate_local_file_path(source_path)?;
        let _ = sanitize_file_name(suggested_name);
        Err(MobileError::Unsupported)
    }

    pub fn get_insets(&self) -> Result<MobileInsets, MobileError> {
        Ok(MobileInsets::default())
    }

    pub fn update_keepalive(&self, _update: &KeepAliveUpdate) -> Result<(), MobileError> {
        Ok(())
    }

    pub fn move_to_background(&self) -> Result<(), MobileError> {
        Ok(())
    }

    pub fn set_keepalive_action_handler(
        &self,
        _handler: Option<Arc<dyn Fn(KeepAliveAction) + Send + Sync>>,
    ) {
    }
}
