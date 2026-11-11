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
        vault.set_ai_key("acp:codex", Some("sk-test")).unwrap();
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
    // AI keys sealed with the lost key go too, instead of failing to open later.
    assert!(vault.ai_key_ids().unwrap().is_empty());
    assert_eq!(vault.ai_key("acp:codex").unwrap(), None);
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
    assert_eq!(hosts[0].os, None);
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
fn device_preferences_are_stored_apart_from_user_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let cache = Arc::new(MemoryKeyCache::default());
    {
        let vault = Vault::open(&path, cache.clone()).unwrap();
        assert_eq!(vault.device_preferences().unwrap(), DevicePrefs::default());
        let device = DevicePrefs {
            window_material: false,
            max_transfers: 5,
            ..DevicePrefs::default()
        };
        vault.set_device_preferences(device).unwrap();
        let bad = DevicePrefs {
            max_transfers: 0,
            ..DevicePrefs::default()
        };
        assert!(matches!(
            vault.set_device_preferences(bad),
            Err(VaultError::Invalid(_))
        ));
    }
    let vault = Vault::open(&path, cache).unwrap();
    let device = vault.device_preferences().unwrap();
    assert!(!device.window_material);
    assert_eq!(device.max_transfers, 5);
    assert_eq!(vault.preferences().unwrap(), Preferences::default());
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
fn host_os_is_stored_validated_and_cleared() {
    let (vault, _) = vault();
    let host = vault.create_host(draft("web", AuthMethod::Agent)).unwrap();
    assert_eq!(host.os, None);
    let ubuntu = HostOs {
        id: "ubuntu".into(),
        like: vec!["debian".into()],
        name: " Ubuntu 24.04.4 LTS\u{7}".into(),
    };
    vault.set_host_os(host.id, Some(ubuntu)).unwrap();
    let stored = vault.host(host.id).unwrap().os.unwrap();
    assert_eq!(stored.id, "ubuntu");
    assert_eq!(stored.like, ["debian"]);
    assert_eq!(stored.name, "Ubuntu 24.04.4 LTS");
    for bad in ["", "Ubuntu", "a b", &"x".repeat(33)] {
        let os = HostOs {
            id: bad.into(),
            like: Vec::new(),
            name: String::new(),
        };
        assert!(
            matches!(
                vault.set_host_os(host.id, Some(os)),
                Err(VaultError::Invalid(_))
            ),
            "{bad}"
        );
    }
    // Editing the host keeps the detected system.
    vault
        .update_host(host.id, draft("web2", AuthMethod::Agent))
        .unwrap();
    assert!(vault.host(host.id).unwrap().os.is_some());
    vault.set_host_os(host.id, None).unwrap();
    assert_eq!(vault.host(host.id).unwrap().os, None);
    assert!(matches!(
        vault.set_host_os(HostId::new(), None),
        Err(VaultError::HostNotFound(_))
    ));
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

fn unlocked() -> Vault {
    let (vault, _) = vault();
    assert_eq!(vault.try_unlock(), LockState::Unlocked);
    vault
}

fn as_changes(records: Vec<SyncRecord>) -> Vec<SyncChange> {
    records
        .into_iter()
        .map(|record| SyncChange {
            kind: record.kind.into(),
            id: record.id,
            body: Some(record.body),
        })
        .collect()
}

#[test]
fn sync_records_round_trip_into_another_vault() {
    let source = unlocked();
    let bastion = source
        .create_host(draft("bastion.example", AuthMethod::Password))
        .unwrap();
    source.set_secret(bastion.id, &password("hunter2")).unwrap();
    let key = source.create_ssh_key(key_draft("laptop")).unwrap();
    let mut inner = draft("inner.example", AuthMethod::Key { key: key.id });
    inner.route = HostRoute::Jump { host: bastion.id };
    let inner = source.create_host(inner).unwrap();
    source
        .create_forward(
            inner.id,
            ForwardDraft {
                label: "db".into(),
                spec: ForwardSpec::Local {
                    bind_host: "127.0.0.1".into(),
                    bind_port: 15432,
                    dest_host: "db.internal".into(),
                    dest_port: 5432,
                },
                auto_start: false,
            },
        )
        .unwrap();
    source
        .create_snippet(SnippetDraft {
            name: "logs".into(),
            command: "journalctl -f".into(),
        })
        .unwrap();
    let mut prefs = Preferences::default();
    prefs.terminal.font_size = 16;
    source.set_preferences(prefs).unwrap();
    source
        .set_ai_key("builtin:deepseek", Some("sk-sync"))
        .unwrap();

    // Children listed before their parents still apply: the vault orders them.
    let mut changes = as_changes(source.sync_records().unwrap());
    changes.reverse();
    let target = unlocked();
    target.apply_sync(&changes).unwrap();

    let normalize = |vault: &Vault| {
        let mut records = vault.sync_records().unwrap();
        records.sort_by(|a, b| (a.kind, &a.id).cmp(&(b.kind, &b.id)));
        records
    };
    assert_eq!(normalize(&target), normalize(&source));
    assert_eq!(
        target.secret(bastion.id).unwrap(),
        Some(password("hunter2"))
    );
    assert_eq!(
        target.ssh_key_secret(key.id).unwrap().private_key.as_str(),
        "PRIVATE laptop"
    );
    assert_eq!(target.preferences().unwrap().terminal.font_size, 16);
    assert_eq!(
        target
            .ai_key("builtin:deepseek")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("sk-sync")
    );
    target
        .apply_sync(&[SyncChange {
            kind: "ai_key".into(),
            id: "builtin:deepseek".into(),
            body: None,
        }])
        .unwrap();
    assert_eq!(target.ai_key("builtin:deepseek").unwrap(), None);

    // Deleting the parent host removes its forward and secret too.
    target
        .apply_sync(&[SyncChange {
            kind: "host".into(),
            id: inner.id.to_string(),
            body: None,
        }])
        .unwrap();
    assert!(target.host(inner.id).is_err());
    assert!(target.list_forwards(None).unwrap().is_empty());
}

#[test]
fn sync_needs_an_unlocked_vault_and_validates_bodies() {
    let (locked, _) = vault();
    assert!(matches!(locked.sync_records(), Err(VaultError::Locked(_))));
    let target = unlocked();
    let bad = SyncChange {
        kind: "snippet".into(),
        id: "0190a0b0-0000-7000-8000-000000000009".into(),
        body: Some(serde_json::json!({ "name": "logs", "command": "  ", "created_at": 0 })),
    };
    assert!(target.apply_sync(&[bad]).is_err());
    let unknown = SyncChange {
        kind: "from-the-future".into(),
        id: "x".into(),
        body: Some(serde_json::json!({})),
    };
    target.apply_sync(&[unknown]).unwrap();
}

#[test]
fn sync_settings_keep_the_key_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let cache = Arc::new(MemoryKeyCache::default());
    {
        let vault = Vault::open(&path, cache.clone()).unwrap();
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        assert_eq!(vault.sync_local().unwrap(), None);
        let local = SyncLocal {
            target: serde_json::json!({ "kind": "folder", "path": "/sync" }),
            state: serde_json::json!({ "device": "d1" }),
            key: Some(Zeroizing::new([9u8; 32])),
            secret: Some(Zeroizing::new("dav-password".into())),
        };
        vault.set_sync_local(Some(&local)).unwrap();
        // The key is not stored in the clear.
        let conn = rusqlite::Connection::open(&path).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT value FROM preferences WHERE key = 'sync'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!stored.contains(&"09".repeat(32)));
        assert!(!stored.contains("dav-password"));
    }
    let vault = Vault::open(&path, cache).unwrap();
    // Locked: settings readable, key not.
    let locked = vault.sync_local().unwrap().unwrap();
    assert_eq!(locked.key, None);
    assert_eq!(locked.secret, None);
    assert_eq!(locked.target["path"], "/sync");
    assert_eq!(vault.try_unlock(), LockState::Unlocked);
    let unlocked = vault.sync_local().unwrap().unwrap();
    assert_eq!(unlocked.key.unwrap().as_slice(), &[9u8; 32]);
    assert_eq!(unlocked.secret.unwrap().as_str(), "dav-password");
    vault.set_sync_local(None).unwrap();
    assert_eq!(vault.sync_local().unwrap(), None);
}

#[test]
fn ai_keys_are_sealed_per_agent_and_need_the_vault_unlocked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let cache = Arc::new(MemoryKeyCache::default());
    {
        let vault = Vault::open(&path, cache.clone()).unwrap();
        assert!(vault.ai_key_ids().unwrap().is_empty());
        assert!(
            vault.set_ai_key("builtin:openai", Some("sk-test")).is_err(),
            "storing needs the vault unlocked"
        );
        assert_eq!(vault.try_unlock(), LockState::Unlocked);
        assert!(matches!(
            vault.set_ai_key("builtin:openai", Some("has space")),
            Err(VaultError::Invalid(_))
        ));
        assert!(matches!(
            vault.set_ai_key("bad id!", Some("k")),
            Err(VaultError::Invalid(_))
        ));
        vault
            .set_ai_key("builtin:openai", Some(" sk-test-123 "))
            .unwrap();
        vault.set_ai_key("acp:claude", Some("ant-456")).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.windows(11).any(|window| window == b"sk-test-123"));
    let vault = Vault::open(&path, cache).unwrap();
    assert_eq!(
        vault.ai_key_ids().unwrap(),
        ["acp:claude", "builtin:openai"]
    );
    assert!(
        vault.ai_key("builtin:openai").is_err(),
        "reading needs the vault unlocked"
    );
    assert_eq!(vault.try_unlock(), LockState::Unlocked);
    assert_eq!(
        vault.ai_key("builtin:openai").unwrap().unwrap().as_str(),
        "sk-test-123"
    );
    assert_eq!(
        vault.ai_key("acp:claude").unwrap().unwrap().as_str(),
        "ant-456"
    );
    assert_eq!(vault.ai_key("acp:codex").unwrap(), None);
    vault.set_ai_key("builtin:openai", None).unwrap();
    assert_eq!(vault.ai_key_ids().unwrap(), ["acp:claude"]);
}

#[test]
fn ai_preferences_are_validated() {
    let (vault, _) = vault();
    let mut prefs = Preferences::default();
    prefs.ai.mode = rove_vault::AiMode::AskAll;
    prefs.ai.read_only = vec![" kubectx ".into(), "".into()];
    prefs.ai.dangerous = vec!["prod-db".into()];
    prefs.ai.providers = vec![rove_vault::ProviderPrefs {
        id: "deepseek".into(),
        name: " DeepSeek ".into(),
        format: rove_vault::ProviderFormat::Chat,
        base_url: "https://api.deepseek.com/v1/".into(),
        model: "deepseek-chat".into(),
        models: vec![
            " deepseek-reasoner ".into(),
            "deepseek-chat".into(),
            "".into(),
            "deepseek-reasoner".into(),
        ],
        max_tokens: None,
        contexts: Default::default(),
        context_window: None,
        effort: rove_vault::ReasoningEffort::Medium,
    }];
    prefs.ai.default_agent = Some("builtin:deepseek".into());
    let stored = vault.set_preferences(prefs.clone()).unwrap();
    assert_eq!(stored.ai.read_only, ["kubectx"]);
    assert_eq!(stored.ai.providers[0].name, "DeepSeek");
    // Other models: trimmed, without the default, blanks or repeats.
    assert_eq!(stored.ai.providers[0].models, ["deepseek-reasoner"]);
    assert_eq!(
        stored.ai.providers[0].base_url,
        "https://api.deepseek.com/v1"
    );
    let mut bad = Preferences::default();
    bad.ai.read_only = vec!["rm -rf".into()];
    assert!(matches!(
        vault.set_preferences(bad),
        Err(VaultError::Invalid(_))
    ));
    for (field, value) in [
        ("base_url", "ftp://x"),
        ("base_url", "https://user:pw@x"),
        ("id", "Bad Id"),
        ("model", ""),
    ] {
        let mut bad = prefs.clone();
        let provider = &mut bad.ai.providers[0];
        match field {
            "base_url" => provider.base_url = value.into(),
            "id" => provider.id = value.into(),
            _ => provider.model = value.into(),
        }
        assert!(
            matches!(vault.set_preferences(bad), Err(VaultError::Invalid(_))),
            "{field} {value}"
        );
    }
    let mut twice = prefs.clone();
    twice.ai.providers.push(twice.ai.providers[0].clone());
    assert!(matches!(
        vault.set_preferences(twice),
        Err(VaultError::Invalid(_))
    ));
    let mut bad = prefs;
    bad.ai.default_agent = Some("cloud:x".into());
    assert!(matches!(
        vault.set_preferences(bad),
        Err(VaultError::Invalid(_))
    ));

    let mut device = DevicePrefs::default();
    device.agents.custom.push(rove_vault::CustomAgent {
        id: "custom-mine".into(),
        name: " Mine ".into(),
        command: " /opt/agent ".into(),
        args: vec![],
    });
    let stored = vault.set_device_preferences(device).unwrap();
    assert_eq!(stored.agents.custom[0].command, "/opt/agent");
    let mut bad = DevicePrefs::default();
    bad.agents.custom.push(rove_vault::CustomAgent {
        id: "mine".into(),
        name: "x".into(),
        command: "x".into(),
        args: vec![],
    });
    assert!(matches!(
        vault.set_device_preferences(bad),
        Err(VaultError::Invalid(_))
    ));
}

#[test]
fn an_older_single_agent_setting_becomes_a_custom_agent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let cache = Arc::new(MemoryKeyCache::default());
    {
        let vault = Vault::open(&path, cache.clone()).unwrap();
        vault
            .set_device_preferences(DevicePrefs::default())
            .unwrap();
    }
    // Write a record the way the previous version did.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE preferences SET value = ?1 WHERE key = 'device'",
        [r#"{"window_material":true,"download_dir":"","max_transfers":3,"agent":{"command":"/usr/bin/codex-acp","args":["--x"],"auth":"api_key"}}"#],
    )
    .unwrap();
    drop(conn);
    let vault = Vault::open(&path, cache).unwrap();
    let device = vault.device_preferences().unwrap();
    assert_eq!(device.agents.custom.len(), 1);
    assert_eq!(device.agents.custom[0].command, "/usr/bin/codex-acp");
    assert_eq!(device.agents.custom[0].args, ["--x"]);
    assert_eq!(device.agents.auth, rove_vault::AgentAuth::ApiKey);
    let stored = vault.set_device_preferences(device).unwrap();
    assert_eq!(stored.agents.custom.len(), 1);
}

#[test]
fn connection_preferences_are_validated() {
    let (vault, _) = vault();
    let mut device = DevicePrefs::default();
    device.connection.keepalive_secs = 60;
    assert_eq!(
        vault
            .set_device_preferences(device)
            .unwrap()
            .connection
            .keepalive_secs,
        60
    );
    let mut bad = DevicePrefs::default();
    bad.connection.connect_timeout_secs = 1;
    assert!(matches!(
        vault.set_device_preferences(bad),
        Err(VaultError::Invalid(_))
    ));
    let mut bad = DevicePrefs::default();
    bad.connection.keepalive_max = 0;
    assert!(matches!(
        vault.set_device_preferences(bad),
        Err(VaultError::Invalid(_))
    ));
}

#[test]
fn preferences_sync_by_section_and_providers_one_by_one() {
    let provider = rove_vault::ProviderPrefs {
        id: "deepseek".into(),
        name: "DeepSeek".into(),
        format: rove_vault::ProviderFormat::Chat,
        base_url: "https://api.deepseek.com/v1".into(),
        model: "deepseek-chat".into(),
        models: Vec::new(),
        max_tokens: None,
        contexts: Default::default(),
        context_window: None,
        effort: rove_vault::ReasoningEffort::Medium,
    };
    // Only `a` has a provider; `b` changed its terminal font.
    let a = unlocked();
    let mut prefs = Preferences::default();
    prefs.ai.providers.push(provider.clone());
    a.set_preferences(prefs).unwrap();
    let b = unlocked();
    let mut prefs = Preferences::default();
    prefs.terminal.font_size = 18;
    b.set_preferences(prefs).unwrap();

    // `b`'s whole preferences arrive at `a`: the sections apply, `a` keeps its provider.
    a.apply_sync(&as_changes(b.sync_records().unwrap()))
        .unwrap();
    let merged = a.preferences().unwrap();
    assert_eq!(merged.terminal.font_size, 18);
    assert_eq!(merged.ai.providers, std::slice::from_ref(&provider));

    // The provider reaches `b` on its own, and so does its removal.
    b.apply_sync(&as_changes(a.sync_records().unwrap()))
        .unwrap();
    assert_eq!(b.preferences().unwrap().ai.providers.len(), 1);
    b.apply_sync(&[SyncChange {
        kind: "ai_provider".into(),
        id: "deepseek".into(),
        body: None,
    }])
    .unwrap();
    assert!(b.preferences().unwrap().ai.providers.is_empty());

    // The whole record of older versions is ignored, as is a deletion of it.
    let before = a.preferences().unwrap();
    a.apply_sync(&[
        SyncChange {
            kind: "prefs".into(),
            id: "app".into(),
            body: Some(serde_json::to_value(Preferences::default()).unwrap()),
        },
        SyncChange {
            kind: "prefs".into(),
            id: "app".into(),
            body: None,
        },
    ])
    .unwrap();
    assert_eq!(a.preferences().unwrap(), before);
}

#[test]
fn an_empty_effort_reads_as_medium() {
    let provider: rove_vault::ProviderPrefs = serde_json::from_value(serde_json::json!({
        "id": "p", "name": "P", "format": "chat", "base_url": "https://x/v1", "model": "m", "effort": null
    }))
    .unwrap();
    assert_eq!(provider.effort, rove_vault::ReasoningEffort::Medium);
    let missing: rove_vault::ProviderPrefs = serde_json::from_value(serde_json::json!({
        "id": "p", "name": "P", "format": "chat", "base_url": "https://x/v1", "model": "m"
    }))
    .unwrap();
    assert_eq!(missing.effort, rove_vault::ReasoningEffort::Medium);
}

#[test]
fn context_windows_are_kept_for_the_providers_models() {
    let vault = unlocked();
    let mut prefs = Preferences::default();
    let mut provider = rove_vault::ProviderPrefs {
        id: "p".into(),
        name: "P".into(),
        format: rove_vault::ProviderFormat::Chat,
        base_url: "https://x/v1".into(),
        model: "a".into(),
        models: vec!["b".into()],
        max_tokens: None,
        contexts: [("a".to_owned(), 128_000), ("gone".to_owned(), 8_000)].into(),
        context_window: Some(64_000),
        effort: rove_vault::ReasoningEffort::Medium,
    };
    prefs.ai.providers.push(provider.clone());
    let stored = vault.set_preferences(prefs.clone()).unwrap();
    assert_eq!(
        stored.ai.providers[0].contexts,
        [("a".to_owned(), 128_000)].into()
    );
    provider.context_window = Some(10);
    prefs.ai.providers[0] = provider;
    assert!(vault.set_preferences(prefs).is_err());
}

#[test]
fn assistant_limits_are_checked() {
    let vault = unlocked();
    let mut prefs = Preferences::default();
    assert_eq!(
        (prefs.ai.max_steps, prefs.ai.request_timeout_secs),
        (40, 180)
    );
    prefs.ai.max_steps = 0;
    assert!(vault.set_preferences(prefs.clone()).is_err());
    prefs.ai.max_steps = 80;
    prefs.ai.request_timeout_secs = 10;
    assert!(vault.set_preferences(prefs.clone()).is_err());
    prefs.ai.request_timeout_secs = 600;
    assert_eq!(vault.set_preferences(prefs).unwrap().ai.max_steps, 80);
    // Older records without them read the defaults.
    let old: Preferences =
        serde_json::from_value(serde_json::json!({ "ai": { "mode": "auto" } })).unwrap();
    assert_eq!(old.ai.request_timeout_secs, 180);
}

#[test]
fn the_auto_judge_reads_old_records_and_checks_its_model() {
    let old: Preferences =
        serde_json::from_value(serde_json::json!({ "ai": { "reviewer": "off" } })).unwrap();
    assert_eq!(old.ai.reviewer, rove_vault::ReviewerKind::Model);
    let vault = unlocked();
    let mut prefs = Preferences::default();
    prefs.ai.providers.push(rove_vault::ProviderPrefs {
        id: "p".into(),
        name: "P".into(),
        format: rove_vault::ProviderFormat::Chat,
        base_url: "https://x/v1".into(),
        model: "big".into(),
        models: vec!["small".into()],
        max_tokens: None,
        contexts: Default::default(),
        context_window: None,
        effort: rove_vault::ReasoningEffort::Medium,
    });
    prefs.ai.reviewer = rove_vault::ReviewerKind::Chosen;
    assert!(vault.set_preferences(prefs.clone()).is_err());
    prefs.ai.review_model = Some(rove_vault::ReviewModel {
        provider: "p".into(),
        model: "small".into(),
    });
    assert!(vault.set_preferences(prefs.clone()).is_ok());
    prefs.ai.review_model = Some(rove_vault::ReviewModel {
        provider: "p".into(),
        model: "other".into(),
    });
    assert!(vault.set_preferences(prefs).is_err());
}

#[test]
fn host_notes_follow_their_host_and_sync_on_their_own() {
    let a = unlocked();
    let host = a
        .create_host(draft("n.example", AuthMethod::Agent))
        .unwrap();
    assert_eq!(a.host_notes(host.id).unwrap(), "");
    a.set_host_notes(host.id, "  nginx + postgres 16\nbackups at 02:00  ")
        .unwrap();
    assert_eq!(
        a.host_notes(host.id).unwrap(),
        "nginx + postgres 16\nbackups at 02:00"
    );
    assert!(
        a.set_host_notes(host.id, &"x".repeat(MAX_NOTES + 1))
            .is_err()
    );
    assert!(a.set_host_notes(host.id, "bell\u{7}").is_err());

    // Another device gets the host and its notes, and later their removal.
    let b = unlocked();
    b.apply_sync(&as_changes(a.sync_records().unwrap()))
        .unwrap();
    assert_eq!(
        b.host_notes(host.id).unwrap(),
        "nginx + postgres 16\nbackups at 02:00"
    );
    b.apply_sync(&[SyncChange {
        kind: "host_notes".into(),
        id: host.id.to_string(),
        body: None,
    }])
    .unwrap();
    assert_eq!(b.host_notes(host.id).unwrap(), "");

    // Removing the host removes its notes.
    a.delete_host(host.id).unwrap();
    assert!(
        a.sync_records()
            .unwrap()
            .iter()
            .all(|record| record.kind != "host_notes")
    );
}

#[test]
fn schedules_are_checked() {
    let vault = unlocked();
    let task = |id: &str, repeat: Repeat| Schedule {
        id: id.into(),
        name: " Daily check ".into(),
        instruction: "check disks".into(),
        repeat,
        allow_changes: false,
        catch_up: true,
        enabled: true,
        created_at: 1,
    };
    let mut prefs = Preferences::default();
    prefs.ai.schedules = vec![task("daily", Repeat::Daily { hour: 9, minute: 0 })];
    assert_eq!(
        vault.set_preferences(prefs.clone()).unwrap().ai.schedules[0].name,
        "Daily check"
    );
    prefs.ai.schedules = vec![task(
        "bad",
        Repeat::Daily {
            hour: 24,
            minute: 0,
        },
    )];
    assert!(vault.set_preferences(prefs.clone()).is_err());
    prefs.ai.schedules = vec![
        task("same", Repeat::Hourly { minute: 5 }),
        task(
            "same",
            Repeat::Weekly {
                weekday: 6,
                hour: 1,
                minute: 0,
            },
        ),
    ];
    assert!(vault.set_preferences(prefs.clone()).is_err());
    prefs.ai.schedules = vec![task("Upper", Repeat::Hourly { minute: 5 })];
    assert!(vault.set_preferences(prefs).is_err());
    // Older records have no schedules.
    let old: Preferences = serde_json::from_value(serde_json::json!({ "ai": {} })).unwrap();
    assert!(old.ai.schedules.is_empty());
}
