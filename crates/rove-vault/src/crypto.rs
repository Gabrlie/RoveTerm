//! Device master key and authenticated encryption of vault fields.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::VaultError;

/// Length of the device master key in bytes.
pub const MASTER_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const ENCRYPTION_INFO: &[u8] = b"rove/vault/enc/v1";

/// The per-device random master key. Lives only in the platform key store and in memory.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; MASTER_KEY_LEN]);

impl MasterKey {
    /// Generates a key from the operating system's random source.
    pub fn generate() -> Result<Self, VaultError> {
        let mut bytes = [0u8; MASTER_KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|error| VaultError::Random(error.to_string()))?;
        Ok(Self(bytes))
    }

    /// Wraps existing key bytes; fails unless exactly [`MASTER_KEY_LEN`] bytes are given.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VaultError> {
        let array: [u8; MASTER_KEY_LEN] = bytes.try_into().map_err(|_| VaultError::KeyFormat)?;
        Ok(Self(array))
    }

    /// Returns the raw key bytes for storing in a key cache.
    pub fn as_bytes(&self) -> &[u8; MASTER_KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(..)")
    }
}

/// An encryption key derived from the master key with HKDF-SHA256.
#[derive(ZeroizeOnDrop)]
pub(crate) struct FieldCipher {
    cipher: XChaCha20Poly1305,
}

impl FieldCipher {
    pub(crate) fn derive(master: &MasterKey) -> Self {
        let mut key = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(None, master.as_bytes())
            .expand(ENCRYPTION_INFO, key.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self {
            cipher: XChaCha20Poly1305::new(key.as_ref().into()),
        }
    }

    /// Encrypts `plaintext` bound to `aad`. Output layout: 24-byte nonce followed by ciphertext.
    pub(crate) fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, VaultError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|error| VaultError::Random(error.to_string()))?;
        let ciphertext = self
            .cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| VaultError::Crypto)?;
        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    /// Decrypts output of [`FieldCipher::seal`]. Fails when the key, the AAD or the data differ.
    pub(crate) fn open(&self, sealed: &[u8], aad: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        if sealed.len() < NONCE_LEN {
            return Err(VaultError::Crypto);
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        self.cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| VaultError::Crypto)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_binds_aad() {
        let cipher = FieldCipher::derive(&MasterKey::generate().unwrap());
        let sealed = cipher.seal(b"secret", b"host:1").unwrap();
        assert_eq!(&cipher.open(&sealed, b"host:1").unwrap()[..], b"secret");
        assert!(matches!(
            cipher.open(&sealed, b"host:2"),
            Err(VaultError::Crypto)
        ));
    }

    #[test]
    fn wrong_key_or_tampering_fails() {
        let cipher = FieldCipher::derive(&MasterKey::generate().unwrap());
        let other = FieldCipher::derive(&MasterKey::generate().unwrap());
        let mut sealed = cipher.seal(b"secret", b"a").unwrap();
        assert!(other.open(&sealed, b"a").is_err());
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(cipher.open(&sealed, b"a").is_err());
        assert!(cipher.open(&[0u8; 3], b"a").is_err());
    }

    #[test]
    fn nonces_differ_per_seal() {
        let cipher = FieldCipher::derive(&MasterKey::generate().unwrap());
        assert_ne!(
            cipher.seal(b"x", b"a").unwrap(),
            cipher.seal(b"x", b"a").unwrap()
        );
    }

    #[test]
    fn debug_hides_key() {
        assert_eq!(
            format!("{:?}", MasterKey::generate().unwrap()),
            "MasterKey(..)"
        );
    }
}
