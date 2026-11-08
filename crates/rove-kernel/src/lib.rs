//! RoveTerm kernel: the shared [`Context`] (typed service registry and [`EventBus`]), revocable
//! registrations ([`Disposer`]), [`Plugin`]s and the profile-driven [`Runtime`].
//!
//! The kernel holds no product logic. See `docs/subsystems/kernel.md` for semantics.

mod context;
mod disposer;
mod events;
mod plugin;
mod profile;

pub use context::Context;
pub use disposer::{Disposer, Disposers};
pub use events::{BoxFuture, Event, EventBus, Next, Waterfall};
pub use plugin::{Plugin, Runtime};
pub use profile::{PluginConfig, Profile};

/// Errors raised by the kernel and by plugins while loading.
#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    /// A second provider was registered for a service type that already has one.
    #[error("service `{0}` already has a provider")]
    ServiceConflict(&'static str),
    /// A required service has no provider.
    #[error("service `{0}` has no provider")]
    MissingService(&'static str),
    /// A profile layer is not valid TOML or does not match the profile schema.
    /// `layer` is the zero-based layer index, or `usize::MAX` for the merged result.
    #[error("invalid profile (layer {layer}): {message}")]
    ProfileParse {
        /// Zero-based layer index, or `usize::MAX` for the merged document.
        layer: usize,
        /// Parser message.
        message: String,
    },
    /// A profile lists the same plugin twice.
    #[error("plugin `{0}` is listed twice")]
    DuplicatePlugin(String),
    /// A profile names a plugin that is not in the catalog.
    #[error("unknown plugin `{0}`")]
    UnknownPlugin(String),
    /// A profile configures a plugin it does not enable.
    #[error("configuration for plugin `{0}` but the plugin is not enabled")]
    UnusedConfig(String),
    /// A plugin requires another plugin that is not applied before it.
    #[error("plugin `{plugin}` requires `{requires}` to be enabled before it")]
    MissingDependency {
        /// The plugin being applied.
        plugin: &'static str,
        /// The missing prerequisite.
        requires: &'static str,
    },
    /// A plugin's configuration section is invalid, or the plugin failed to initialize.
    #[error("plugin `{plugin}`: {message}")]
    PluginConfig {
        /// The failing plugin.
        plugin: String,
        /// Human-readable reason.
        message: String,
    },
}

impl KernelError {
    /// Builds a [`KernelError::PluginConfig`] for `plugin` from any displayable error.
    pub fn plugin(plugin: &str, error: impl std::fmt::Display) -> Self {
        Self::PluginConfig {
            plugin: plugin.to_owned(),
            message: error.to_string(),
        }
    }
}
