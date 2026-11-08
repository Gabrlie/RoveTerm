//! Host, secret and known-host storage for RoveTerm.
//!
//! Host metadata is stored in plain SQLite columns; each host's secret is sealed with
//! XChaCha20-Poly1305 under a key derived from the per-device master key, which lives only in the
//! platform key store behind the [`KeyCache`] seam. See `docs/subsystems/vault.md`.

mod crypto;
mod key_cache;
mod model;
mod store;

use std::path::PathBuf;
use std::sync::Arc;

use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use serde::Deserialize;

pub use crypto::{MASTER_KEY_LEN, MasterKey};
#[cfg(feature = "memory-key-cache")]
pub use key_cache::MemoryKeyCache;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use key_cache::OsKeyCache;
pub use key_cache::{Interaction, KeyCache, KeyCacheError};
pub use model::{AuthMethod, Host, HostDraft, HostId, HostKey, KnownHostStatus, Secret};
pub use store::{LockReason, LockState, SCHEMA_VERSION, Vault};

/// Errors returned by the vault.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The secret store is locked.
    #[error("vault is locked ({0})")]
    Locked(String),
    /// No host with this id exists.
    #[error("host {0} not found")]
    HostNotFound(HostId),
    /// Input failed validation.
    #[error("{0}")]
    Invalid(String),
    /// Decryption failed: wrong key, wrong record binding or corrupted data.
    #[error("decryption failed")]
    Crypto,
    /// Key bytes have the wrong length.
    #[error("master key has the wrong length")]
    KeyFormat,
    /// The operating system random source failed.
    #[error("random source failed: {0}")]
    Random(String),
    /// The database was written by a newer RoveTerm.
    #[error("vault schema version {0} is newer than this build supports")]
    FutureSchema(i32),
    /// SQLite error.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// Stored JSON could not be encoded or decoded.
    #[error("record encoding error: {0}")]
    Encoding(#[from] serde_json::Error),
    /// Filesystem error.
    #[error("filesystem error: {0}")]
    Io(String),
    /// The key cache failed.
    #[error(transparent)]
    KeyCache(#[from] KeyCacheError),
}

/// `[plugin.vault]` configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultConfig {
    /// Path of the vault database. The application sets it from its data directory.
    pub database: PathBuf,
    /// Platform key store service name for the device master key.
    #[serde(default = "default_service")]
    pub keychain_service: String,
}

fn default_service() -> String {
    "dev.roveterm.app".to_owned()
}

/// Registers [`Vault`], plus the platform [`KeyCache`] unless the host already provided one.
///
/// The vault starts locked; the application calls [`Vault::try_unlock`] after startup.
pub struct VaultPlugin;

impl Plugin for VaultPlugin {
    fn name(&self) -> &'static str {
        "vault"
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: VaultConfig = config.parse()?;
        let mut disposers = Vec::new();
        let key_cache = match ctx.get::<dyn KeyCache>() {
            Some(cache) => cache,
            None => {
                let cache = platform_key_cache(&config)?;
                disposers.push(ctx.provide::<dyn KeyCache>(cache.clone())?);
                cache
            }
        };
        let vault = Vault::open(&config.database, key_cache)
            .map_err(|error| KernelError::plugin("vault", error))?;
        disposers.push(ctx.provide(Arc::new(vault))?);
        Ok(disposers)
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn platform_key_cache(config: &VaultConfig) -> Result<Arc<dyn KeyCache>, KernelError> {
    Ok(Arc::new(OsKeyCache::new(config.keychain_service.clone())))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn platform_key_cache(_config: &VaultConfig) -> Result<Arc<dyn KeyCache>, KernelError> {
    Err(KernelError::plugin(
        "vault",
        "no platform key cache on this target; the host must provide a KeyCache",
    ))
}
