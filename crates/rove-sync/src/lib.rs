//! End-to-end encrypted multi-device sync. Each device appends sealed segments of record changes
//! to its own directory in a shared space on an untrusted file store; devices merge each other's
//! changes per record by hybrid logical clock (last writer wins). See
//! docs/decisions/0005-sync.md.

mod backend;
mod crypto;
mod engine;
mod hlc;
mod space;

pub use backend::{FolderBackend, MAX_FILE, SyncBackend, check_path};
pub use crypto::{KdfParams, SyncKey};
pub use engine::{Change, Known, Record, RecordStore, Report, SyncState, sync};
pub use hlc::{Clock, Hlc};
pub use space::{MANIFEST, create_space, join_space, space_exists};

/// Errors of the sync engine and its backends.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The backend failed (network, file system, server response).
    #[error("sync storage: {0}")]
    Backend(String),
    /// A file in the space is not in the expected format.
    #[error("sync data: {0}")]
    Format(String),
    /// Sealed data did not open: the wrong key, or it was altered or moved.
    #[error("sync data was altered or sealed with another key")]
    Rejected,
    /// The sync password does not open the space.
    #[error("wrong sync password")]
    WrongPassword,
    /// The backend holds no space yet.
    #[error("no sync space here yet")]
    NoSpace,
    /// The backend already holds a space.
    #[error("a sync space already exists here")]
    SpaceExists,
    /// Key derivation or encryption failed.
    #[error("sync crypto: {0}")]
    Crypto(String),
    /// The local store could not read or write records.
    #[error("sync store: {0}")]
    Store(String),
}
