use std::marker::PhantomData;

use tauri::Runtime;
use zeroize::Zeroizing;

use crate::{BiometricStatus, MobileError, MobileInsets};

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

    pub fn get_insets(&self) -> Result<MobileInsets, MobileError> {
        Ok(MobileInsets::default())
    }
}
