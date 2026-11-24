//! Vault key cache backed by the native mobile credential store.

use std::sync::atomic::{AtomicBool, Ordering};

use rove_vault::{Interaction, KeyCache, KeyCacheError, MasterKey};
use tauri_plugin_rove_mobile::MobileError;
use zeroize::Zeroizing;

trait KeyStoreBackend: Send + Sync {
    fn load(&self, service: &str, silent: bool) -> Result<Option<Zeroizing<Vec<u8>>>, MobileError>;
    fn store(&self, service: &str, key: &[u8], biometric: bool) -> Result<(), MobileError>;
}

pub(crate) struct MobileKeyCache {
    backend: Box<dyn KeyStoreBackend>,
    biometric: AtomicBool,
    /// Serializes policy changes through both native rewrap and preference persistence.
    #[cfg(mobile)]
    pub(crate) policy_change: parking_lot::Mutex<()>,
}

impl MobileKeyCache {
    fn new(backend: impl KeyStoreBackend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
            biometric: AtomicBool::new(false),
            #[cfg(mobile)]
            policy_change: parking_lot::Mutex::new(()),
        }
    }

    #[cfg(mobile)]
    pub(crate) fn native(app: tauri::AppHandle) -> Self {
        Self::new(NativeKeyStore(app))
    }

    pub(crate) fn biometric(&self) -> bool {
        self.biometric.load(Ordering::Relaxed)
    }

    pub(crate) fn set_biometric(&self, enabled: bool) {
        self.biometric.store(enabled, Ordering::Relaxed);
    }
}

impl KeyCache for MobileKeyCache {
    fn load(&self, interaction: Interaction) -> Result<Option<MasterKey>, KeyCacheError> {
        match self.backend.load(
            crate::profile::keychain_service(),
            interaction == Interaction::Silent,
        ) {
            Ok(Some(bytes)) => MasterKey::from_bytes(&bytes)
                .map(Some)
                .map_err(|_| KeyCacheError::Malformed),
            Ok(None) | Err(MobileError::KeyInvalidated) => Ok(None),
            Err(error) => Err(key_cache_error(error)),
        }
    }

    fn store(&self, key: &MasterKey) -> Result<(), KeyCacheError> {
        self.backend
            .store(
                crate::profile::keychain_service(),
                key.as_bytes(),
                self.biometric(),
            )
            .map_err(key_cache_error)
    }
}

fn key_cache_error(error: MobileError) -> KeyCacheError {
    match error {
        MobileError::Denied => KeyCacheError::Denied,
        MobileError::Malformed(_) => KeyCacheError::Malformed,
        MobileError::Unavailable(message)
        | MobileError::BiometricUnavailable(message)
        | MobileError::Plugin(message)
        | MobileError::InvalidInput(message) => KeyCacheError::Unavailable(message),
        MobileError::Unsupported => {
            KeyCacheError::Unavailable("not supported on this platform".into())
        }
        MobileError::KeyInvalidated => KeyCacheError::Unavailable(error.to_string()),
    }
}

#[cfg(mobile)]
struct NativeKeyStore(tauri::AppHandle);

#[cfg(mobile)]
impl KeyStoreBackend for NativeKeyStore {
    fn load(&self, service: &str, silent: bool) -> Result<Option<Zeroizing<Vec<u8>>>, MobileError> {
        use tauri_plugin_rove_mobile::MobileBridgeExt;

        self.0.rove_mobile().load_master_key(
            service,
            silent,
            "Unlock RoveTerm Vault",
            "Authenticate to access saved credentials",
        )
    }

    fn store(&self, service: &str, key: &[u8], biometric: bool) -> Result<(), MobileError> {
        use tauri_plugin_rove_mobile::MobileBridgeExt;

        self.0.rove_mobile().store_master_key(
            service,
            key,
            biometric,
            "Protect RoveTerm Vault",
            "Authenticate to protect saved credentials",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use rove_vault::{LockReason, LockState, MASTER_KEY_LEN, Vault, VaultError};
    use std::sync::Arc;

    #[derive(Default)]
    struct TestBackend {
        key: Mutex<Option<Zeroizing<Vec<u8>>>>,
        load_error: Mutex<Option<MobileError>>,
        store_error: Mutex<Option<MobileError>>,
        silent_loads: Mutex<Vec<bool>>,
        biometric_stores: Mutex<Vec<bool>>,
    }

    impl KeyStoreBackend for Arc<TestBackend> {
        fn load(
            &self,
            service: &str,
            silent: bool,
        ) -> Result<Option<Zeroizing<Vec<u8>>>, MobileError> {
            assert_eq!(service, crate::profile::keychain_service());
            self.silent_loads.lock().push(silent);
            match self.load_error.lock().take() {
                Some(error) => Err(error),
                None => Ok(self.key.lock().clone()),
            }
        }

        fn store(&self, service: &str, key: &[u8], biometric: bool) -> Result<(), MobileError> {
            assert_eq!(service, crate::profile::keychain_service());
            self.biometric_stores.lock().push(biometric);
            if let Some(error) = self.store_error.lock().take() {
                return Err(error);
            }
            *self.key.lock() = Some(Zeroizing::new(key.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn forwards_interaction_to_load_and_biometric_policy_to_store() {
        let backend = Arc::new(TestBackend::default());
        let cache = MobileKeyCache::new(backend.clone());
        assert!(cache.load(Interaction::Silent).unwrap().is_none());
        let key = MasterKey::generate().unwrap();
        cache.store(&key).unwrap();
        cache.set_biometric(true);
        cache.store(&key).unwrap();
        let loaded = cache.load(Interaction::Allowed).unwrap().unwrap();
        assert!(loaded.as_bytes() == key.as_bytes());
        assert_eq!(*backend.silent_loads.lock(), [true, false]);
        assert_eq!(*backend.biometric_stores.lock(), [false, true]);
    }

    #[test]
    fn rejects_keys_with_the_wrong_length() {
        let backend = Arc::new(TestBackend::default());
        let cache = MobileKeyCache::new(backend.clone());
        *backend.key.lock() = Some(Zeroizing::new(vec![0; MASTER_KEY_LEN - 1]));
        assert!(matches!(
            cache.load(Interaction::Allowed),
            Err(KeyCacheError::Malformed)
        ));
    }

    #[test]
    fn maps_native_errors_to_vault_key_cache_errors() {
        for (error, expected) in [
            (MobileError::Denied, KeyCacheError::Denied),
            (
                MobileError::Malformed("invalid bytes".into()),
                KeyCacheError::Malformed,
            ),
            (
                MobileError::Unsupported,
                KeyCacheError::Unavailable("not supported on this platform".into()),
            ),
            (
                MobileError::Unavailable("offline".into()),
                KeyCacheError::Unavailable("offline".into()),
            ),
            (
                MobileError::BiometricUnavailable("no enrollment".into()),
                KeyCacheError::Unavailable("no enrollment".into()),
            ),
            (
                MobileError::Plugin("invocation failed".into()),
                KeyCacheError::Unavailable("invocation failed".into()),
            ),
            (
                MobileError::InvalidInput("invalid service".into()),
                KeyCacheError::Unavailable("invalid service".into()),
            ),
            (
                MobileError::KeyInvalidated,
                KeyCacheError::Unavailable("the device key was invalidated".into()),
            ),
        ] {
            let mapped = key_cache_error(error);
            assert_eq!(
                std::mem::discriminant(&mapped),
                std::mem::discriminant(&expected)
            );
            assert_eq!(mapped.to_string(), expected.to_string());
        }
    }

    #[test]
    fn invalidation_marks_a_vault_with_an_existing_key_check_as_key_lost() {
        let backend = Arc::new(TestBackend::default());
        let vault = Vault::open_in_memory(Arc::new(MobileKeyCache::new(backend.clone()))).unwrap();
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        *backend.load_error.lock() = Some(MobileError::KeyInvalidated);
        assert_eq!(
            vault.try_unlock(),
            LockState::Locked {
                reason: LockReason::KeyLost,
            }
        );
        assert_eq!(*backend.biometric_stores.lock(), [false]);
    }

    #[test]
    fn failed_rewrap_can_restore_the_previous_store_policy() {
        let backend = Arc::new(TestBackend::default());
        let cache = Arc::new(MobileKeyCache::new(backend.clone()));
        let vault = Vault::open_in_memory(cache.clone()).unwrap();
        assert_eq!(vault.try_unlock(), LockState::Unlocked);

        *backend.store_error.lock() = Some(MobileError::Denied);
        let result = vault.rewrap_master_key(
            || {
                cache.set_biometric(true);
                Ok(())
            },
            || cache.set_biometric(false),
        );
        assert!(matches!(
            result,
            Err(VaultError::KeyCache(KeyCacheError::Denied))
        ));
        assert!(!cache.biometric());
        assert_eq!(*backend.biometric_stores.lock(), [false, true]);
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
    }
}
