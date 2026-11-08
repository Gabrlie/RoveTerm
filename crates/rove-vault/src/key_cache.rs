//! The `KeyCache` seam: where the device master key is kept between runs.

use crate::MasterKey;

/// Whether a key cache may show system authorization UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    /// Startup path: never prompt. Stores that cannot guarantee this still return promptly.
    Silent,
    /// A user action needs the key; the store may prompt.
    Allowed,
}

/// Errors reported by a key cache.
#[derive(Debug, thiserror::Error)]
pub enum KeyCacheError {
    /// The platform store is not available (for example no Secret Service on Linux).
    #[error("key store unavailable: {0}")]
    Unavailable(String),
    /// The user denied access or cancelled the prompt.
    #[error("access to the key store was denied")]
    Denied,
    /// The stored value is not a valid master key.
    #[error("stored key is malformed")]
    Malformed,
}

/// Stores the device master key. Definition of the `KeyCache` seam.
pub trait KeyCache: Send + Sync {
    /// Loads the master key. `Ok(None)` means no key has been stored yet.
    fn load(&self, interaction: Interaction) -> Result<Option<MasterKey>, KeyCacheError>;

    /// Stores `key`, replacing any existing key.
    fn store(&self, key: &MasterKey) -> Result<(), KeyCacheError>;
}

/// [`KeyCache`] backed by the platform credential store: Keychain on macOS, Credential Manager on
/// Windows, Secret Service on Linux.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub struct OsKeyCache {
    service: String,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl OsKeyCache {
    const ACCOUNT: &'static str = "master/v1";

    /// Uses the credential store entry `(service, "master/v1")`. Development and production builds
    /// use different service names so their keys never mix.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, KeyCacheError> {
        keyring::Entry::new(&self.service, Self::ACCOUNT)
            .map_err(|error| KeyCacheError::Unavailable(error.to_string()))
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl KeyCache for OsKeyCache {
    fn load(&self, _interaction: Interaction) -> Result<Option<MasterKey>, KeyCacheError> {
        match self.entry()?.get_secret() {
            Ok(bytes) => {
                let bytes = zeroize::Zeroizing::new(bytes);
                MasterKey::from_bytes(&bytes)
                    .map(Some)
                    .map_err(|_| KeyCacheError::Malformed)
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoStorageAccess(_)) => Err(KeyCacheError::Denied),
            Err(error) => Err(KeyCacheError::Unavailable(error.to_string())),
        }
    }

    fn store(&self, key: &MasterKey) -> Result<(), KeyCacheError> {
        self.entry()?
            .set_secret(key.as_bytes())
            .map_err(|error| match error {
                keyring::Error::NoStorageAccess(_) => KeyCacheError::Denied,
                other => KeyCacheError::Unavailable(other.to_string()),
            })
    }
}

/// Non-persistent [`KeyCache`] for tests and development builds. Never used by shipped profiles.
#[cfg(feature = "memory-key-cache")]
#[derive(Default)]
pub struct MemoryKeyCache {
    key: parking_lot::Mutex<Option<MasterKey>>,
    unavailable: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "memory-key-cache")]
impl MemoryKeyCache {
    /// Makes every subsequent call fail with [`KeyCacheError::Unavailable`] when `true`.
    pub fn set_unavailable(&self, unavailable: bool) {
        self.unavailable
            .store(unavailable, std::sync::atomic::Ordering::SeqCst);
    }

    /// Removes the stored key, simulating a wiped platform store.
    pub fn clear(&self) {
        self.key.lock().take();
    }

    fn check(&self) -> Result<(), KeyCacheError> {
        if self.unavailable.load(std::sync::atomic::Ordering::SeqCst) {
            Err(KeyCacheError::Unavailable(
                "memory key cache disabled".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(feature = "memory-key-cache")]
impl KeyCache for MemoryKeyCache {
    fn load(&self, _interaction: Interaction) -> Result<Option<MasterKey>, KeyCacheError> {
        self.check()?;
        Ok(self.key.lock().clone())
    }

    fn store(&self, key: &MasterKey) -> Result<(), KeyCacheError> {
        self.check()?;
        *self.key.lock() = Some(key.clone());
        Ok(())
    }
}
