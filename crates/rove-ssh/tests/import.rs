use std::path::Path;
use std::sync::Arc;

use rove_ssh::import_ssh_config;
use rove_vault::{AuthMethod, HostDraft, MemoryKeyCache, Vault};

#[test]
fn imports_hosts_and_skips_existing() {
    let vault = Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap();
    vault
        .create_host(HostDraft {
            name: "existing".into(),
            group: None,
            hostname: "Old.Example".into(),
            port: 22,
            username: "me".into(),
            auth: AuthMethod::Agent,
            route: rove_vault::HostRoute::Direct,
            tags: Vec::new(),
        })
        .unwrap();
    let config = "Host old\n  HostName old.example\n\nHost new\n  HostName new.example\n  Port 2200\n  IdentityFile ~/.ssh/id\n";
    let report = import_ssh_config(&vault, config, Path::new("/home/me"), "me").unwrap();
    assert_eq!(report.created, ["new"]);
    assert_eq!(report.skipped, ["old"]);

    let imported = vault
        .list_hosts()
        .unwrap()
        .into_iter()
        .find(|host| host.name == "new")
        .unwrap();
    assert_eq!(
        (
            imported.hostname.as_str(),
            imported.port,
            imported.username.as_str()
        ),
        ("new.example", 2200, "me")
    );
    assert_eq!(
        imported.auth,
        AuthMethod::KeyFile {
            path: "/home/me/.ssh/id".into()
        }
    );
    assert_eq!(imported.tags, ["ssh-config"]);
}
