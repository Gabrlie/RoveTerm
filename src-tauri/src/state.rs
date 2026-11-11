//! Services shared by every command.

use std::sync::Arc;

use rove_fs::{FsSessions, LocalFs, TransferQueue};
use rove_kernel::{Disposer, Runtime};
use rove_session::SessionManager;
use rove_ssh::{Forwards, HostKeyPrompts, SshConnector};
use rove_vault::{LockState, Vault};
use std::sync::Mutex;

use crate::error::CommandError;

/// Handles to the profile's services, resolved once at startup.
pub struct AppState {
    /// Session table.
    pub sessions: Arc<SessionManager>,
    /// Host and secret storage.
    pub vault: Arc<Vault>,
    /// SSH terminal provider.
    pub ssh: Arc<SshConnector>,
    /// Pending host key confirmations.
    pub prompts: Arc<HostKeyPrompts>,
    /// Running port forwards.
    pub forwards: Arc<Forwards>,
    /// Open file systems (SFTP sessions).
    pub fs_sessions: Arc<FsSessions>,
    /// File transfer queue.
    pub transfers: Arc<TransferQueue>,
    /// This device's file system.
    pub local_fs: Arc<LocalFs>,
    /// Durable event journal.
    pub journal: Arc<dyn rove_journal::JournalStore>,
    /// Local PTY provider.
    #[cfg(desktop)]
    pub pty: Arc<rove_pty::LocalTerminals>,
    /// Running AI sessions.
    pub ai: crate::commands::ai::AiSessions,
    /// Access for agents the user runs on their own, while it is on.
    pub external: crate::commands::external::ExternalAccess,
    /// Held while a sync round runs, so rounds never overlap.
    pub sync_lock: tokio::sync::Mutex<()>,
    /// Plugin runtime and app-level registrations; taken on exit to unload plugins in order.
    pub lifetime: Mutex<Option<(Runtime, Vec<Disposer>)>>,
}

impl AppState {
    /// Resolves services from the booted runtime.
    pub fn new(runtime: Runtime, data_dir: &std::path::Path) -> Result<Self, CommandError> {
        let ctx = runtime.context().clone();
        let journal = ctx.require::<dyn rove_journal::JournalStore>()?;
        let ai = crate::commands::ai::AiSessions::new(data_dir, journal.clone())
            .map_err(|error| CommandError::new("app.io", error.to_string()))?;
        Ok(Self {
            sessions: ctx.require::<SessionManager>()?,
            vault: ctx.require::<Vault>()?,
            ssh: ctx.require::<SshConnector>()?,
            prompts: ctx.require::<HostKeyPrompts>()?,
            forwards: ctx.require::<Forwards>()?,
            fs_sessions: ctx.require::<FsSessions>()?,
            transfers: ctx.require::<TransferQueue>()?,
            local_fs: ctx.require::<LocalFs>()?,
            journal,
            ai,
            external: crate::commands::external::ExternalAccess::new(data_dir),
            #[cfg(desktop)]
            pty: ctx.require::<rove_pty::LocalTerminals>()?,
            sync_lock: tokio::sync::Mutex::new(()),
            lifetime: Mutex::new(Some((runtime, Vec::new()))),
        })
    }

    /// Unlocks the vault on behalf of a user action unless it is already unlocked. The platform
    /// store may prompt, so this runs on a blocking thread.
    pub async fn ensure_unlocked(&self) -> Result<(), CommandError> {
        if self.vault.lock_state() == LockState::Unlocked {
            return Ok(());
        }
        let vault = self.vault.clone();
        let state = tauri::async_runtime::spawn_blocking(move || vault.unlock())
            .await
            .map_err(|error| CommandError::new("app.task", error.to_string()))?;
        match state {
            LockState::Unlocked => Ok(()),
            LockState::Locked { reason } => Err(CommandError {
                code: "vault.locked",
                message: "vault is locked".into(),
                detail: serde_json::to_value(reason).ok(),
            }),
        }
    }
}
