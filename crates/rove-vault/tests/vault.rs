use std::sync::Arc;

use rove_vault::*;
use zeroize::Zeroizing;

fn draft(hostname: &str, auth: AuthMethod) -> HostDraft {
    HostDraft {
        name: String::new(),
        group: None,
        hostname: hostname.into(),
        port: 22,
        username: "root".into(),
        auth,
        route: HostRoute::Direct,
        tags: Vec::new(),
    }
}

fn password(text: &str) -> Secret {
    Secret::Password {
        password: Zeroizing::new(text.into()),
    }
}

fn vault() -> (Vault, Arc<MemoryKeyCache>) {
    let cache = Arc::new(MemoryKeyCache::default());
    (Vault::open_in_memory(cache.clone()).unwrap(), cache)
}

#[test]
fn first_unlock_creates_key_and_secrets_round_trip() {
    let (vault, _cache) = vault();
    assert!(matches!(
        vault.lock_state(),
        LockState::Locked {
            reason: LockReason::NotAttempted
        }
    ));
    assert_eq!(vault.try_unlock(), LockState::Unlocked);
    let host = vault
        .create_host(draft("a.example", AuthMethod::Password))
        .unwrap();
    vault.set_secret(host.id, &password("hunter2")).unwrap();
    assert_eq!(vault.secret(host.id).unwrap(), Some(password("hunter2")));
}

#[test]
fn unavailable_store_stays_locked_without_error_and_hosts_still_work() {
    let (vault, cache) = vault();
    cache.set_unavailable(true);
    assert!(matches!(
        vault.try_unlock(),
        LockState::Locked {
            reason: LockReason::StoreUnavailable(_)
        }
    ));
    let host = vault
        .create_host(draft("b.example", AuthMethod::Agent))
        .unwrap();
    assert_eq!(vault.list_hosts().unwrap(), vec![host.clone()]);
    assert!(matches!(
        vault.set_secret(host.id, &password("x")),
        Err(VaultError::Locked(_))
    ));
}

#[test]
fn reopening_with_same_key_reads_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let cache = Arc::new(MemoryKeyCache::default());
    let id = {
        let vault = Vault::open(&path, cache.clone()).unwrap();
        vault.try_unlock();
        let host = vault
            .create_host(draft("c.example", AuthMethod::Password))
            .unwrap();
        vault.set_secret(host.id, &password("pw")).unwrap();
        host.id
    };
    let vault = Vault::open(&path, cache).unwrap();
    assert_eq!(vault.try_unlock(), LockState::Unlocked);
    assert_eq!(vault.secret(id).unwrap(), Some(password("pw")));
}

#[test]
fn lost_key_locks_until_reset_which_discards_secrets_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let cache = Arc::new(MemoryKeyCache::default());
    let id = {
        let vault = Vault::open(&path, cache.clone()).unwrap();
        vault.try_unlock();
        let host = vault
            .create_host(draft("d.example", AuthMethod::Password))
            .unwrap();
        vault.set_secret(host.id, &password("pw")).unwrap();
        host.id
    };
    cache.clear();
    let vault = Vault::open(&path, cache).unwrap();
    assert!(matches!(
        vault.try_unlock(),
        LockState::Locked {
            reason: LockReason::KeyLost
        }
    ));
    assert_eq!(vault.reset_secrets().unwrap(), LockState::Unlocked);
    assert_eq!(vault.secret(id).unwrap(), None);
    assert_eq!(vault.host(id).unwrap().hostname, "d.example");
}

#[test]
fn deleting_host_deletes_its_secret() {
    let (vault, _cache) = vault();
    vault.try_unlock();
    let host = vault
        .create_host(draft("e.example", AuthMethod::Password))
        .unwrap();
    vault.set_secret(host.id, &password("pw")).unwrap();
    vault.delete_host(host.id).unwrap();
    assert!(!vault.has_secret(host.id).unwrap());
    assert!(matches!(
        vault.host(host.id),
        Err(VaultError::HostNotFound(_))
    ));
    assert!(matches!(
        vault.delete_host(host.id),
        Err(VaultError::HostNotFound(_))
    ));
}

#[test]
fn changing_auth_kind_drops_secret_but_editing_fields_keeps_it() {
    let (vault, _cache) = vault();
    vault.try_unlock();
    let host = vault
        .create_host(draft("f.example", AuthMethod::Password))
        .unwrap();
    vault.set_secret(host.id, &password("pw")).unwrap();
    let renamed = HostDraft {
        name: "Prod".into(),
        ..draft("f.example", AuthMethod::Password)
    };
    assert_eq!(vault.update_host(host.id, renamed).unwrap().name, "Prod");
    assert!(vault.has_secret(host.id).unwrap());
    vault
        .update_host(host.id, draft("f.example", AuthMethod::Agent))
        .unwrap();
    assert!(!vault.has_secret(host.id).unwrap());
}

#[test]
fn known_hosts_detect_unknown_trusted_and_mismatch() {
    let (vault, _cache) = vault();
    let key = |public_key: &str| HostKey {
        algorithm: "ssh-ed25519".into(),
        public_key: public_key.into(),
        fingerprint: format!("SHA256:{public_key}"),
    };
    assert_eq!(
        vault
            .check_host_key("Example.com", 22, "", &key("AAA"))
            .unwrap(),
        KnownHostStatus::Unknown
    );
    vault
        .trust_host_key("Example.com", 22, "", &key("AAA"))
        .unwrap();
    assert_eq!(
        vault
            .check_host_key("example.com", 22, "", &key("AAA"))
            .unwrap(),
        KnownHostStatus::Trusted
    );
    assert_eq!(
        vault
            .check_host_key("example.com", 2222, "", &key("AAA"))
            .unwrap(),
        KnownHostStatus::Unknown
    );
    assert!(matches!(
        vault.check_host_key("example.com", 22, "", &key("BBB")).unwrap(),
        KnownHostStatus::Mismatch { expected_fingerprint, .. } if expected_fingerprint == "SHA256:AAA"
    ));
}

#[test]
fn hosts_list_sorted_by_group_then_name() {
    let (vault, _cache) = vault();
    let mut grouped = draft("z.example", AuthMethod::Agent);
    grouped.group = Some("prod".into());
    vault.create_host(grouped).unwrap();
    vault
        .create_host(draft("b.example", AuthMethod::Agent))
        .unwrap();
    vault
        .create_host(draft("a.example", AuthMethod::Agent))
        .unwrap();
    let names: Vec<_> = vault
        .list_hosts()
        .unwrap()
        .into_iter()
        .map(|host| host.name)
        .collect();
    assert_eq!(names, ["a.example", "b.example", "z.example"]);
}

#[test]
fn forward_rules_round_trip_and_cascade_with_their_host() {
    let (vault, _cache) = vault();
    let host = vault
        .create_host(draft("fwd.example", AuthMethod::Agent))
        .unwrap();
    let other = vault
        .create_host(draft("other.example", AuthMethod::Agent))
        .unwrap();
    let rule = vault
        .create_forward(
            host.id,
            ForwardDraft {
                label: " db ".into(),
                spec: ForwardSpec::Local {
                    bind_host: "127.0.0.1".into(),
                    bind_port: 15432,
                    dest_host: "db.internal".into(),
                    dest_port: 5432,
                },
                auto_start: true,
            },
        )
        .unwrap();
    assert_eq!(rule.label, "db");
    vault
        .create_forward(
            other.id,
            ForwardDraft {
                label: String::new(),
                spec: ForwardSpec::Dynamic {
                    bind_host: "localhost".into(),
                    bind_port: 1080,
                },
                auto_start: false,
            },
        )
        .unwrap();
    assert_eq!(
        vault.list_forwards(Some(host.id)).unwrap(),
        vec![rule.clone()]
    );
    assert_eq!(vault.list_forwards(None).unwrap().len(), 2);

    let updated = vault
        .update_forward(
            rule.id,
            ForwardDraft {
                label: "db2".into(),
                spec: rule.spec.clone(),
                auto_start: false,
            },
        )
        .unwrap();
    assert_eq!(updated.label, "db2");
    assert!(!updated.auto_start);

    let invalid = ForwardDraft {
        label: String::new(),
        spec: ForwardSpec::Local {
            bind_host: "evil.example".into(),
            bind_port: 1,
            dest_host: "x".into(),
            dest_port: 1,
        },
        auto_start: false,
    };
    assert!(matches!(
        vault.create_forward(host.id, invalid),
        Err(VaultError::Invalid(_))
    ));

    vault.delete_host(host.id).unwrap();
    assert!(vault.list_forwards(Some(host.id)).unwrap().is_empty());
    assert!(matches!(
        vault.forward(rule.id),
        Err(VaultError::ForwardNotFound(_))
    ));
    assert_eq!(vault.list_forwards(None).unwrap().len(), 1);
}

#[test]
fn version_one_databases_migrate_to_the_current_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
             CREATE TABLE hosts (id TEXT PRIMARY KEY, name TEXT NOT NULL, group_name TEXT,
                 hostname TEXT NOT NULL, port INTEGER NOT NULL, username TEXT NOT NULL,
                 auth TEXT NOT NULL, tags TEXT NOT NULL, created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL);
             CREATE TABLE host_secrets (host_id TEXT PRIMARY KEY REFERENCES hosts(id) ON DELETE CASCADE,
                 sealed BLOB NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE known_hosts (hostname TEXT NOT NULL, port INTEGER NOT NULL,
                 algorithm TEXT NOT NULL, public_key TEXT NOT NULL, fingerprint TEXT NOT NULL,
                 added_at INTEGER NOT NULL, PRIMARY KEY (hostname, port));
             INSERT INTO hosts VALUES ('0190a0b0-0000-7000-8000-000000000001', 'old', NULL, 'old.lan',
                 22, 'root', '{"method":"agent"}', '[]', 0, 0);
             INSERT INTO known_hosts VALUES ('old.lan', 22, 'ssh-ed25519', 'ssh-ed25519 OLD', 'SHA256:OLD', 0);
             PRAGMA user_version = 1;"#,
        )
        .unwrap();
    }
    let vault = Vault::open(&path, Arc::new(MemoryKeyCache::default())).unwrap();
    assert!(vault.list_forwards(None).unwrap().is_empty());
    assert_eq!(vault.preferences().unwrap(), Preferences::default());
    let hosts = vault.list_hosts().unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].route, HostRoute::Direct);
    let old_key = HostKey {
        algorithm: "ssh-ed25519".into(),
        public_key: "ssh-ed25519 OLD".into(),
        fingerprint: "SHA256:OLD".into(),
    };
    assert_eq!(
        vault.check_host_key("old.lan", 22, "", &old_key).unwrap(),
        KnownHostStatus::Trusted
    );
    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn preferences_default_validate_and_persist_while_locked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let cache = Arc::new(MemoryKeyCache::default());
    {
        let vault = Vault::open(&path, cache.clone()).unwrap();
        assert_eq!(vault.preferences().unwrap(), Preferences::default());
        let mut prefs = Preferences::default();
        prefs.terminal.font_family = " MesloLGS NF ".into();
        prefs.terminal.font_size = 15;
        let stored = vault.set_preferences(prefs).unwrap();
        assert_eq!(stored.terminal.font_family, "MesloLGS NF");
        let mut bad = Preferences::default();
        bad.terminal.font_size = 99;
        assert!(matches!(
            vault.set_preferences(bad),
            Err(VaultError::Invalid(_))
        ));
    }
    let vault = Vault::open(&path, cache).unwrap();
    let prefs = vault.preferences().unwrap();
    assert_eq!(prefs.terminal.font_family, "MesloLGS NF");
    assert_eq!(prefs.terminal.font_size, 15);
}

#[test]
fn jump_routes_reject_cycles_and_reset_when_the_jump_host_is_deleted() {
    let (vault, _) = vault();
    let bastion = vault
        .create_host(draft("bastion", AuthMethod::Agent))
        .unwrap();
    let inner = vault
        .create_host(HostDraft {
            route: HostRoute::Jump { host: bastion.id },
            ..draft("inner", AuthMethod::Agent)
        })
        .unwrap();
    assert_eq!(
        vault.host(inner.id).unwrap().route,
        HostRoute::Jump { host: bastion.id }
    );

    // bastion -> inner -> bastion would loop.
    let looped = vault.update_host(
        bastion.id,
        HostDraft {
            route: HostRoute::Jump { host: inner.id },
            ..draft("bastion", AuthMethod::Agent)
        },
    );
    assert!(matches!(looped, Err(VaultError::Invalid(_))), "{looped:?}");
    let missing = vault.create_host(HostDraft {
        route: HostRoute::Jump {
            host: HostId::new(),
        },
        ..draft("orphan", AuthMethod::Agent)
    });
    assert!(matches!(missing, Err(VaultError::HostNotFound(_))));

    let mut previous = inner.id;
    for depth in 2..=MAX_JUMP_DEPTH + 1 {
        let next = vault.create_host(HostDraft {
            route: HostRoute::Jump { host: previous },
            ..draft(&format!("hop{depth}"), AuthMethod::Agent)
        });
        if depth <= MAX_JUMP_DEPTH {
            previous = next.unwrap().id;
        } else {
            assert!(matches!(next, Err(VaultError::Invalid(_))));
        }
    }

    vault.delete_host(bastion.id).unwrap();
    assert_eq!(vault.host(inner.id).unwrap().route, HostRoute::Direct);
}

#[test]
fn proxy_routes_round_trip() {
    let (vault, _) = vault();
    let route = HostRoute::Proxy {
        protocol: ProxyProtocol::Http,
        host: "proxy.lan".into(),
        port: 3128,
    };
    let host = vault
        .create_host(HostDraft {
            route: route.clone(),
            ..draft("web", AuthMethod::Agent)
        })
        .unwrap();
    assert_eq!(vault.list_hosts().unwrap()[0].route, route);
    assert_eq!(vault.host(host.id).unwrap().route, route);
}

fn key_draft(name: &str) -> KeyDraft {
    KeyDraft {
        name: name.into(),
        algorithm: "ssh-ed25519".into(),
        public_key: format!("ssh-ed25519 AAAA {name}"),
        fingerprint: format!("SHA256:{name}"),
        secret: KeySecret {
            private_key: Zeroizing::new(format!("PRIVATE {name}")),
            passphrase: None,
        },
    }
}

#[test]
fn key_library_seals_private_keys_and_guards_keys_in_use() {
    let (vault, _) = vault();
    assert!(matches!(
        vault.create_ssh_key(key_draft("work")),
        Err(VaultError::Locked(_))
    ));
    vault.unlock();
    let work = vault.create_ssh_key(key_draft(" work ")).unwrap();
    assert_eq!(work.name, "work");
    assert!(work.has_private);
    assert!(matches!(
        vault.create_ssh_key(key_draft("WORK")),
        Err(VaultError::Invalid(_))
    ));
    assert_eq!(
        vault.ssh_key_secret(work.id).unwrap().private_key.as_str(),
        "PRIVATE  work "
    );

    let host = vault
        .create_host(draft("db", AuthMethod::Key { key: work.id }))
        .unwrap();
    assert!(matches!(
        vault.create_host(draft("x", AuthMethod::Key { key: KeyId::new() })),
        Err(VaultError::KeyNotFound(_))
    ));
    assert!(matches!(
        vault.delete_ssh_key(work.id),
        Err(VaultError::KeyInUse(names)) if names == ["db"]
    ));

    let custom = vault
        .set_ssh_key_public(work.id, "ssh-ed25519 AAAA custom-comment")
        .unwrap();
    assert_eq!(custom.public_key, "ssh-ed25519 AAAA custom-comment");
    let renamed = vault.rename_ssh_key(work.id, "laptop").unwrap();
    assert_eq!(renamed.name, "laptop");
    let replaced = vault.replace_ssh_key(work.id, key_draft("laptop")).unwrap();
    assert_eq!(replaced.public_key, "ssh-ed25519 AAAA laptop");

    vault.reset_secrets().unwrap();
    let after_reset = vault.ssh_key(work.id).unwrap();
    assert!(!after_reset.has_private);
    assert!(matches!(
        vault.ssh_key_secret(work.id),
        Err(VaultError::Invalid(_))
    ));

    vault.delete_host(host.id).unwrap();
    vault.delete_ssh_key(work.id).unwrap();
    assert!(vault.list_ssh_keys().unwrap().is_empty());
}

#[test]
fn touching_a_host_records_the_connection_time() {
    let (vault, _) = vault();
    let host = vault.create_host(draft("web", AuthMethod::Agent)).unwrap();
    assert_eq!(host.last_connected_at, None);
    vault.touch_host(host.id).unwrap();
    assert!(vault.host(host.id).unwrap().last_connected_at.is_some());
}

#[test]
fn snippets_round_trip() {
    let (vault, _) = vault();
    let snippet = vault
        .create_snippet(SnippetDraft {
            name: "logs".into(),
            command: "journalctl -f".into(),
        })
        .unwrap();
    let updated = vault
        .update_snippet(
            snippet.id,
            SnippetDraft {
                name: String::new(),
                command: "tail -f /var/log/syslog".into(),
            },
        )
        .unwrap();
    assert_eq!(updated.name, "tail -f /var/log/syslog");
    assert_eq!(vault.list_snippets().unwrap(), [updated]);
    vault.delete_snippet(snippet.id).unwrap();
    assert!(matches!(
        vault.delete_snippet(snippet.id),
        Err(VaultError::SnippetNotFound(_))
    ));
}

#[test]
fn known_hosts_are_separate_per_route() {
    let (vault, _) = vault();
    let key = |data: &str| HostKey {
        algorithm: "ssh-ed25519".into(),
        public_key: format!("ssh-ed25519 {data}"),
        fingerprint: format!("SHA256:{data}"),
    };
    vault
        .trust_host_key("10.0.0.1", 22, "jump:a", &key("A"))
        .unwrap();
    // The same address behind another jump host is unknown, not a mismatch.
    assert_eq!(
        vault
            .check_host_key("10.0.0.1", 22, "jump:b", &key("B"))
            .unwrap(),
        KnownHostStatus::Unknown
    );
    vault
        .trust_host_key("10.0.0.1", 22, "jump:b", &key("B"))
        .unwrap();
    assert_eq!(
        vault
            .check_host_key("10.0.0.1", 22, "jump:a", &key("A"))
            .unwrap(),
        KnownHostStatus::Trusted
    );
    assert!(matches!(
        vault
            .check_host_key("10.0.0.1", 22, "jump:a", &key("B"))
            .unwrap(),
        KnownHostStatus::Mismatch { .. }
    ));
}
