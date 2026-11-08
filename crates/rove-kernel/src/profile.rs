//! Profiles: the enabled plugin list plus layered TOML configuration.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::KernelError;

/// A named assembly of plugins and their configuration.
///
/// Profile TOML has a `[profile]` table naming the profile and its ordered plugin list, and one
/// `[plugin.<name>]` table per plugin that needs configuration:
///
/// ```toml
/// [profile]
/// name = "desktop"
/// plugins = ["vault", "journal", "session"]
///
/// [plugin.session]
/// coalesce_ms = 6
/// ```
#[derive(Debug, Clone)]
pub struct Profile {
    name: String,
    plugins: Vec<String>,
    config: Table,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    profile: ProfileHeader,
    #[serde(default)]
    plugin: Table,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileHeader {
    name: String,
    plugins: Vec<String>,
}

impl Profile {
    /// Parses and merges TOML layers in order; later layers override earlier ones.
    ///
    /// Tables merge key by key at every depth; any other value, including arrays such as
    /// `profile.plugins`, replaces the earlier value whole.
    pub fn from_layers<'a>(layers: impl IntoIterator<Item = &'a str>) -> Result<Self, KernelError> {
        let mut merged = Table::new();
        for (index, layer) in layers.into_iter().enumerate() {
            let table: Table =
                toml::from_str(layer).map_err(|error| KernelError::ProfileParse {
                    layer: index,
                    message: error.to_string(),
                })?;
            merge(&mut merged, table);
        }
        let file = ProfileFile::deserialize(Value::Table(merged)).map_err(|error| {
            KernelError::ProfileParse {
                layer: usize::MAX,
                message: error.to_string(),
            }
        })?;
        let mut seen = std::collections::HashSet::new();
        for name in &file.profile.plugins {
            if !seen.insert(name.as_str()) {
                return Err(KernelError::DuplicatePlugin(name.clone()));
            }
        }
        Ok(Self {
            name: file.profile.name,
            plugins: file.profile.plugins,
            config: file.plugin,
        })
    }

    /// Returns the profile name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the enabled plugin names in application order.
    pub fn plugins(&self) -> &[String] {
        &self.plugins
    }

    /// Returns the configuration section of `plugin`, empty when the profile has none.
    pub fn plugin_config(&self, plugin: &str) -> PluginConfig {
        let table = match self.config.get(plugin) {
            Some(Value::Table(table)) => table.clone(),
            _ => Table::new(),
        };
        PluginConfig {
            plugin: plugin.to_owned(),
            table,
        }
    }

    /// Returns the names of configuration sections, including sections of plugins that are not
    /// enabled.
    pub fn configured_plugins(&self) -> impl Iterator<Item = &str> {
        self.config.keys().map(String::as_str)
    }
}

fn merge(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(base_table)), Value::Table(overlay_table)) => {
                merge(base_table, overlay_table)
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// One plugin's configuration section.
#[derive(Debug, Clone)]
pub struct PluginConfig {
    plugin: String,
    table: Table,
}

impl PluginConfig {
    /// Creates a configuration section directly, for tests and embedders.
    pub fn new(plugin: impl Into<String>, table: Table) -> Self {
        Self {
            plugin: plugin.into(),
            table,
        }
    }

    /// Deserializes the section into `T`. Missing fields take `T`'s serde defaults; mark `T` with
    /// `#[serde(deny_unknown_fields)]` so misspelled keys fail at load.
    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, KernelError> {
        T::deserialize(Value::Table(self.table.clone())).map_err(|error| {
            KernelError::PluginConfig {
                plugin: self.plugin.clone(),
                message: error.to_string(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
        [profile]
        name = "desktop"
        plugins = ["a", "b"]

        [plugin.a]
        limit = 1
        mode = "x"
    "#;

    #[test]
    fn later_layers_override_and_merge() {
        let profile =
            Profile::from_layers([BASE, "[plugin.a]\nlimit = 5\n[profile]\nplugins = [\"a\"]"])
                .unwrap();
        assert_eq!(profile.plugins(), ["a"]);
        #[derive(Deserialize)]
        struct A {
            limit: u32,
            mode: String,
        }
        let a: A = profile.plugin_config("a").parse().unwrap();
        assert_eq!((a.limit, a.mode.as_str()), (5, "x"));
    }

    #[test]
    fn rejects_duplicate_plugins() {
        let result = Profile::from_layers(["[profile]\nname = \"p\"\nplugins = [\"a\", \"a\"]"]);
        assert!(matches!(result, Err(KernelError::DuplicatePlugin(name)) if name == "a"));
    }

    #[test]
    fn rejects_unknown_top_level_keys() {
        let result = Profile::from_layers([BASE, "typo = 1"]);
        assert!(matches!(result, Err(KernelError::ProfileParse { .. })));
    }

    #[test]
    fn rejects_invalid_toml_with_layer_index() {
        let result = Profile::from_layers([BASE, "not toml ="]);
        assert!(matches!(
            result,
            Err(KernelError::ProfileParse { layer: 1, .. })
        ));
    }
}
