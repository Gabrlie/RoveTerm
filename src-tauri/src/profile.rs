//! Profile assembly: the plugin catalog and the configuration layers for this build.

use std::path::Path;
use std::sync::Arc;

use rove_kernel::{KernelError, Plugin, Profile};

/// Built-in profile of this target.
#[cfg(desktop)]
const BUILTIN: &str = include_str!("../profiles/desktop.toml");
#[cfg(mobile)]
const BUILTIN: &str = include_str!("../profiles/mobile.toml");

/// Every plugin this build can load.
pub fn catalog() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(rove_vault::VaultPlugin),
        Arc::new(rove_journal::JournalPlugin),
        Arc::new(rove_session::SessionPlugin),
        Arc::new(rove_fs::FsPlugin),
        Arc::new(rove_ssh::SshPlugin),
        #[cfg(desktop)]
        Arc::new(rove_pty::PtyPlugin),
    ]
}

/// Keychain service name; development builds never touch the production key.
fn keychain_service() -> &'static str {
    if cfg!(debug_assertions) {
        "dev.roveterm.app.dev"
    } else {
        "dev.roveterm.app"
    }
}

/// Builds the profile from three layers: the built-in profile, the optional user file
/// `<config_dir>/roveterm.toml`, and the paths this process derives from `data_dir`.
pub fn load(data_dir: &Path, user_file: &Path) -> Result<Profile, KernelError> {
    let user = match std::fs::read_to_string(user_file) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(KernelError::plugin(
                "profile",
                format!("read {}: {error}", user_file.display()),
            ));
        }
    };
    let runtime = runtime_layer(data_dir);
    let mut layers = vec![BUILTIN];
    if let Some(user) = user.as_deref() {
        layers.push(user);
    }
    layers.push(&runtime);
    Profile::from_layers(layers)
}

fn runtime_layer(data_dir: &Path) -> String {
    let path = |name: &str| toml::Value::String(data_dir.join(name).to_string_lossy().into_owned());
    let mut vault = toml::Table::new();
    vault.insert("database".into(), path("vault.db"));
    vault.insert(
        "keychain_service".into(),
        toml::Value::String(keychain_service().into()),
    );
    let mut journal = toml::Table::new();
    journal.insert("database".into(), path("journal.db"));
    let mut plugin = toml::Table::new();
    plugin.insert("vault".into(), toml::Value::Table(vault));
    plugin.insert("journal".into(), toml::Value::Table(journal));
    let mut root = toml::Table::new();
    root.insert("plugin".into(), toml::Value::Table(plugin));
    toml::to_string(&root).expect("runtime profile layer serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profile_boots_with_runtime_layer_and_user_overrides() {
        let dir = std::env::temp_dir().join(format!("roveterm-profile-{}", std::process::id()));
        let user = dir.join("roveterm.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&user, "[plugin.session]\ncoalesce_ms = 10\n").unwrap();
        let profile = load(&dir, &user).unwrap();
        let session: toml::Table = profile.plugin_config("session").parse().unwrap();
        assert_eq!(session["coalesce_ms"].as_integer(), Some(10));
        let vault: toml::Table = profile.plugin_config("vault").parse().unwrap();
        assert!(vault["database"].as_str().unwrap().ends_with("vault.db"));
        let names: Vec<_> = catalog().iter().map(|plugin| plugin.name()).collect();
        assert!(
            profile
                .plugins()
                .iter()
                .all(|plugin| names.contains(&plugin.as_str()))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
