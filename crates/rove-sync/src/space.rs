//! The space manifest, `space.json`: how to derive the sync key and a sealed check string that
//! tells a right password from a wrong one.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

use crate::crypto::{KdfParams, SALT_LEN, SyncKey, random_salt};
use crate::{SyncBackend, SyncError};

/// Path of the manifest in a space.
pub const MANIFEST: &str = "space.json";
const FORMAT: u32 = 1;
const CHECK: &[u8] = b"rove-sync-space-check";
const CHECK_AAD: &[u8] = b"rove/sync/check/v1";

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: u32,
    salt: String,
    kdf: KdfParams,
    check: String,
}

/// Whether `backend` already holds a space.
pub async fn space_exists(backend: &dyn SyncBackend) -> Result<bool, SyncError> {
    Ok(backend.get(MANIFEST).await?.is_some())
}

/// Derives the key off the async runtime: Argon2id takes most of a second by design.
async fn derive(password: &str, salt: Vec<u8>, kdf: KdfParams) -> Result<SyncKey, SyncError> {
    let password = zeroize::Zeroizing::new(password.to_owned());
    tokio::task::spawn_blocking(move || SyncKey::derive(&password, &salt, kdf))
        .await
        .map_err(|error| SyncError::Crypto(error.to_string()))?
}

/// Creates a new space protected by `password` with `kdf` costs. Fails with
/// [`SyncError::SpaceExists`] when the backend already holds one.
pub async fn create_space(
    backend: &dyn SyncBackend,
    password: &str,
    kdf: KdfParams,
) -> Result<SyncKey, SyncError> {
    if space_exists(backend).await? {
        return Err(SyncError::SpaceExists);
    }
    let salt = random_salt()?;
    let key = derive(password, salt.to_vec(), kdf).await?;
    let manifest = Manifest {
        format: FORMAT,
        salt: BASE64.encode(salt),
        kdf,
        check: BASE64.encode(key.seal(CHECK, CHECK_AAD)?),
    };
    let json = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| SyncError::Format(error.to_string()))?;
    backend.put(MANIFEST, &json).await?;
    Ok(key)
}

/// Joins the space in `backend` with `password`. Fails with [`SyncError::NoSpace`] when there is
/// none and [`SyncError::WrongPassword`] when the password does not open it.
pub async fn join_space(backend: &dyn SyncBackend, password: &str) -> Result<SyncKey, SyncError> {
    let bytes = backend.get(MANIFEST).await?.ok_or(SyncError::NoSpace)?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|error| SyncError::Format(error.to_string()))?;
    if manifest.format != FORMAT {
        return Err(SyncError::Format(format!(
            "unsupported space format {}",
            manifest.format
        )));
    }
    let salt = BASE64
        .decode(&manifest.salt)
        .map_err(|error| SyncError::Format(error.to_string()))?;
    if salt.len() != SALT_LEN {
        return Err(SyncError::Format("invalid salt".into()));
    }
    let check = BASE64
        .decode(&manifest.check)
        .map_err(|error| SyncError::Format(error.to_string()))?;
    let key = derive(password, salt, manifest.kdf).await?;
    match key.open(&check, CHECK_AAD) {
        Ok(plain) if plain.as_slice() == CHECK => Ok(key),
        Ok(_) | Err(SyncError::Rejected) => Err(SyncError::WrongPassword),
        Err(error) => Err(error),
    }
}
