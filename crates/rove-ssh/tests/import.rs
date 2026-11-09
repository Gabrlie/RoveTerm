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

#[test]
fn proxy_jump_aliases_become_jump_routes() {
    use rove_vault::HostRoute;
    let vault = Vault::open_in_memory(Arc::new(MemoryKeyCache::default())).unwrap();
    // The jump host is declared after the host that uses it.
    let config = "Host db\n  HostName 10.0.0.5\n  ProxyJump bastion\n\n\
                  Host web\n  HostName 10.0.0.6\n  ProxyJump ops@gate:2222\n\n\
                  Host bastion\n  HostName bastion.example\n";
    let report = import_ssh_config(&vault, config, Path::new("/home/me"), "me").unwrap();
    assert_eq!(report.created, ["db", "web", "bastion"]);
    let hosts = vault.list_hosts().unwrap();
    let find = |name: &str| hosts.iter().find(|host| host.name == name).unwrap();
    assert_eq!(
        find("db").route,
        HostRoute::Jump {
            host: find("bastion").id
        }
    );
    assert_eq!(find("web").route, HostRoute::Direct);
    assert_eq!(find("bastion").route, HostRoute::Direct);
}
