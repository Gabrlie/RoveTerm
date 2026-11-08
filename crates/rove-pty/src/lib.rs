//! Local PTY terminal provider for RoveTerm.
//!
//! [`LocalTerminals`] starts a shell in a pseudo-terminal and returns an
//! [`OpenedTerminal`](rove_session::OpenedTerminal) for the session manager. See
//! `docs/subsystems/pty.md`.

mod shell;
mod terminal;

use std::sync::Arc;

use rove_kernel::{Context, Disposer, KernelError, Plugin, PluginConfig};
use serde::Deserialize;

pub use shell::{LaunchSpec, LocalRequest, default_shell};
pub use terminal::LocalTerminals;

/// `[plugin.pty]` configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PtyConfig {
    /// Shell program used when a request names none. Absent: platform default.
    pub shell: Option<String>,
    /// Arguments for the configured shell. Ignored when `shell` is absent.
    pub args: Vec<String>,
}

/// Registers [`LocalTerminals`] on the context.
pub struct PtyPlugin;

impl Plugin for PtyPlugin {
    fn name(&self) -> &'static str {
        "pty"
    }

    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError> {
        let config: PtyConfig = config.parse()?;
        if config.shell.as_deref().is_some_and(str::is_empty) {
            return Err(KernelError::plugin("pty", "shell must not be empty"));
        }
        Ok(vec![ctx.provide(Arc::new(LocalTerminals::new(config)))?])
    }
}
