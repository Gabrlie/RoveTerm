//! Plugins and the runtime that applies them in profile order.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{Context, Disposer, Disposers, KernelError, PluginConfig, Profile};

/// A unit of functionality that registers services, listeners and interceptors on a [`Context`].
pub trait Plugin: Send + Sync + 'static {
    /// The unique plugin name used in profiles.
    fn name(&self) -> &'static str;

    /// Names of plugins that must be applied before this one.
    fn requires(&self) -> &'static [&'static str] {
        &[]
    }

    /// Validates `config` and registers this plugin's contributions.
    ///
    /// Every registration's [`Disposer`] must be returned; the runtime drops them in reverse order
    /// when the plugin is unloaded. On error, disposers created so far are dropped by the plugin.
    fn apply(&self, ctx: &Context, config: &PluginConfig) -> Result<Vec<Disposer>, KernelError>;
}

/// Applies a profile's plugins to a fresh [`Context`] and owns their registrations.
///
/// Dropping the runtime unloads plugins in reverse application order.
pub struct Runtime {
    ctx: Context,
    applied: Vec<(&'static str, Disposers)>,
}

impl Runtime {
    /// Boots `profile` using plugins from `catalog`.
    ///
    /// Fails, after unloading anything already applied, when a profile names an unknown plugin, a
    /// plugin's requirement is not applied earlier, a configuration section names a plugin that is
    /// not enabled, or a plugin's `apply` fails.
    pub fn boot(profile: &Profile, catalog: &[Arc<dyn Plugin>]) -> Result<Self, KernelError> {
        Self::boot_in(Context::new(), profile, catalog)
    }

    /// Boots `profile` on an existing context, which may already hold host-provided services.
    pub fn boot_in(
        ctx: Context,
        profile: &Profile,
        catalog: &[Arc<dyn Plugin>],
    ) -> Result<Self, KernelError> {
        let by_name: HashMap<&str, &Arc<dyn Plugin>> = catalog
            .iter()
            .map(|plugin| (plugin.name(), plugin))
            .collect();
        for configured in profile.configured_plugins() {
            if !profile.plugins().iter().any(|name| name == configured) {
                return Err(KernelError::UnusedConfig(configured.to_owned()));
            }
        }
        let mut runtime = Self {
            ctx,
            applied: Vec::new(),
        };
        for name in profile.plugins() {
            let plugin = by_name
                .get(name.as_str())
                .ok_or_else(|| KernelError::UnknownPlugin(name.clone()))?;
            for required in plugin.requires() {
                if !runtime
                    .applied
                    .iter()
                    .any(|(applied, _)| applied == required)
                {
                    return Err(KernelError::MissingDependency {
                        plugin: plugin.name(),
                        requires: required,
                    });
                }
            }
            let disposers = plugin.apply(&runtime.ctx, &profile.plugin_config(name))?;
            tracing::debug!(
                plugin = plugin.name(),
                registrations = disposers.len(),
                "plugin applied"
            );
            runtime
                .applied
                .push((plugin.name(), Disposers::from(disposers)));
        }
        Ok(runtime)
    }

    /// Returns the shared context.
    pub fn context(&self) -> &Context {
        &self.ctx
    }

    /// Returns the applied plugin names in application order.
    pub fn plugins(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.applied.iter().map(|(name, _)| *name)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        while let Some((name, disposers)) = self.applied.pop() {
            drop(disposers);
            tracing::debug!(plugin = name, "plugin unloaded");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Value(u32);

    struct Provider {
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Plugin for Provider {
        fn name(&self) -> &'static str {
            "provider"
        }
        fn apply(
            &self,
            ctx: &Context,
            config: &PluginConfig,
        ) -> Result<Vec<Disposer>, KernelError> {
            #[derive(serde::Deserialize, Default)]
            #[serde(deny_unknown_fields, default)]
            struct Config {
                value: u32,
            }
            let config: Config = config.parse()?;
            let log = self.log.clone();
            Ok(vec![
                ctx.provide(Arc::new(Value(config.value)))?,
                Disposer::new(move || log.lock().unwrap().push("provider unloaded".into())),
            ])
        }
    }

    struct Consumer {
        log: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    impl Plugin for Consumer {
        fn name(&self) -> &'static str {
            "consumer"
        }
        fn requires(&self) -> &'static [&'static str] {
            &["provider"]
        }
        fn apply(
            &self,
            ctx: &Context,
            _config: &PluginConfig,
        ) -> Result<Vec<Disposer>, KernelError> {
            let value = ctx.require::<Value>()?;
            if self.fail {
                return Err(KernelError::PluginConfig {
                    plugin: "consumer".into(),
                    message: "forced".into(),
                });
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("consumer saw {}", value.0));
            let log = self.log.clone();
            Ok(vec![Disposer::new(move || {
                log.lock().unwrap().push("consumer unloaded".into())
            })])
        }
    }

    fn catalog(log: &Arc<Mutex<Vec<String>>>, fail: bool) -> Vec<Arc<dyn Plugin>> {
        vec![
            Arc::new(Provider { log: log.clone() }),
            Arc::new(Consumer {
                log: log.clone(),
                fail,
            }),
        ]
    }

    fn profile(text: &str) -> Profile {
        Profile::from_layers([text]).unwrap()
    }

    #[test]
    fn boots_in_order_and_unloads_in_reverse() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let runtime = Runtime::boot(
            &profile("[profile]\nname = \"t\"\nplugins = [\"provider\", \"consumer\"]\n[plugin.provider]\nvalue = 3"),
            &catalog(&log, false),
        )
        .unwrap();
        assert_eq!(
            runtime.plugins().collect::<Vec<_>>(),
            ["provider", "consumer"]
        );
        drop(runtime);
        assert_eq!(
            *log.lock().unwrap(),
            ["consumer saw 3", "consumer unloaded", "provider unloaded"]
        );
    }

    #[test]
    fn missing_dependency_fails_and_unloads_applied_plugins() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let result = Runtime::boot(
            &profile("[profile]\nname = \"t\"\nplugins = [\"consumer\", \"provider\"]"),
            &catalog(&log, false),
        );
        assert!(matches!(
            result,
            Err(KernelError::MissingDependency {
                plugin: "consumer",
                ..
            })
        ));
    }

    #[test]
    fn failed_apply_rolls_back_earlier_plugins() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let result = Runtime::boot(
            &profile("[profile]\nname = \"t\"\nplugins = [\"provider\", \"consumer\"]"),
            &catalog(&log, true),
        );
        assert!(result.is_err());
        assert_eq!(*log.lock().unwrap(), ["provider unloaded"]);
    }

    #[test]
    fn unknown_plugin_and_unused_config_fail() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let unknown = Runtime::boot(
            &profile("[profile]\nname = \"t\"\nplugins = [\"nope\"]"),
            &catalog(&log, false),
        );
        assert!(matches!(unknown, Err(KernelError::UnknownPlugin(name)) if name == "nope"));
        let unused = Runtime::boot(
            &profile("[profile]\nname = \"t\"\nplugins = [\"provider\"]\n[plugin.consumer]\nx = 1"),
            &catalog(&log, false),
        );
        assert!(matches!(unused, Err(KernelError::UnusedConfig(name)) if name == "consumer"));
    }

    #[test]
    fn misspelled_config_key_fails() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let result = Runtime::boot(
            &profile(
                "[profile]\nname = \"t\"\nplugins = [\"provider\"]\n[plugin.provider]\nvalu = 3",
            ),
            &catalog(&log, false),
        );
        assert!(matches!(result, Err(KernelError::PluginConfig { .. })));
    }
}
