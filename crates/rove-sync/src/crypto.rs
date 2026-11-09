//! The sync key: derived from the sync password with Argon2id, sealing data with
//! XChaCha20-Poly1305 and fingerprinting records with HMAC-SHA256 under a derived subkey.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::SyncError;

const NONCE_LEN: usize = 24;
/// Length of the random salt stored in the space manifest.
pub const SALT_LEN: usize = 16;

/// Argon2id cost parameters, stored with the space so every device derives the same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m: u32,
    /// Iterations.
    pub t: u32,
    /// Lanes.
    pub p: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m: 64 * 1024,
            t: 3,
            p: 1,
        }
    }
}

impl KdfParams {
    /// Rejects parameters a hostile manifest could use to exhaust memory or time, and ones too
    /// weak to slow down guessing.
    pub fn check(&self) -> Result<(), SyncError> {
        let sane = (8 * 1024..=1024 * 1024).contains(&self.m)
            && (1..=10).contains(&self.t)
            && (1..=8).contains(&self.p);
        if sane {
            Ok(())
        } else {
            Err(SyncError::Format(
                "unsupported key derivation parameters".into(),
            ))
        }
    }
}

/// The 32-byte sync key. Zeroed on drop; never logged.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SyncKey([u8; 32]);

impl std::fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncKey(..)")
    }
}

impl SyncKey {
    /// Derives the key from `password` and the space's `salt`. Slow by design (Argon2id).
    pub fn derive(password: &str, salt: &[u8], params: KdfParams) -> Result<Self, SyncError> {
        params.check()?;
        let argon = argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(params.m, params.t, params.p, Some(32))
                .map_err(|error| SyncError::Crypto(error.to_string()))?,
        );
        let mut key = [0u8; 32];
        argon
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|error| SyncError::Crypto(error.to_string()))?;
        Ok(Self(key))
    }

    /// Restores a key previously exported with [`SyncKey::to_bytes`].
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw key, for storing it sealed on this device.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0)
    }

    fn subkey(&self, purpose: &[u8]) -> Zeroizing<[u8; 32]> {
        let hkdf = Hkdf::<Sha256>::new(None, &self.0);
        let mut out = Zeroizing::new([0u8; 32]);
        // 32 bytes is always a valid HKDF-SHA256 output length.
        hkdf.expand(purpose, out.as_mut())
            .expect("32-byte HKDF output");
        out
    }

    /// Encrypts `plaintext`, binding it to `aad`. Output: nonce followed by ciphertext.
    pub fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, SyncError> {
        let key = self.subkey(b"rove/sync/seal/v1");
        let cipher = XChaCha20Poly1305::new(key.as_ref().into());
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|error| SyncError::Crypto(error.to_string()))?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| SyncError::Crypto("encryption failed".into()))?;
        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    /// Decrypts data sealed with the same key and `aad`. Fails with [`SyncError::Rejected`] when
    /// the key is wrong or the data or its binding was changed.
    pub fn open(&self, sealed: &[u8], aad: &[u8]) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        if sealed.len() < NONCE_LEN {
            return Err(SyncError::Rejected);
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let key = self.subkey(b"rove/sync/seal/v1");
        let cipher = XChaCha20Poly1305::new(key.as_ref().into());
        cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| SyncError::Rejected)
    }

    /// Keyed fingerprint of a record's content, for change detection. Without the key it reveals
    /// nothing about the content.
    pub fn fingerprint(&self, content: &[u8]) -> String {
        let key = self.subkey(b"rove/sync/fingerprint/v1");
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_ref())
            .expect("HMAC takes keys of any length");
        mac.update(content);
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// A fresh random salt for a new space.
pub fn random_salt() -> Result<[u8; SALT_LEN], SyncError> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|error| SyncError::Crypto(error.to_string()))?;
    Ok(salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap parameters so tests stay fast.
    fn fast() -> KdfParams {
        KdfParams {
            m: 8 * 1024,
            t: 1,
            p: 1,
        }
    }

    #[test]
    fn same_password_and_salt_give_the_same_key() {
        let salt = [7u8; SALT_LEN];
        let a = SyncKey::derive("correct horse", &salt, fast()).unwrap();
        let b = SyncKey::derive("correct horse", &salt, fast()).unwrap();
        let sealed = a.seal(b"hello", b"aad").unwrap();
        assert_eq!(&b.open(&sealed, b"aad").unwrap()[..], b"hello");
        assert_eq!(a.fingerprint(b"x"), b.fingerprint(b"x"));
    }

    #[test]
    fn wrong_key_or_aad_or_tampering_is_rejected() {
        let salt = [7u8; SALT_LEN];
        let key = SyncKey::derive("one", &salt, fast()).unwrap();
        let other = SyncKey::derive("two", &salt, fast()).unwrap();
        let mut sealed = key.seal(b"secret", b"place-1").unwrap();
        assert!(matches!(
            other.open(&sealed, b"place-1"),
            Err(SyncError::Rejected)
        ));
        assert!(matches!(
            key.open(&sealed, b"place-2"),
            Err(SyncError::Rejected)
        ));
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(matches!(
            key.open(&sealed, b"place-1"),
            Err(SyncError::Rejected)
        ));
        assert!(matches!(
            key.open(b"short", b"place-1"),
            Err(SyncError::Rejected)
        ));
    }

    #[test]
    fn hostile_kdf_parameters_are_refused() {
        assert!(
            KdfParams {
                m: 4 * 1024 * 1024,
                t: 3,
                p: 1
            }
            .check()
            .is_err()
        );
        assert!(
            KdfParams {
                m: 64 * 1024,
                t: 0,
                p: 1
            }
            .check()
            .is_err()
        );
        assert!(KdfParams::default().check().is_ok());
    }
}
