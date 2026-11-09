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
            .check_host_key("Example.com", 22, &key("AAA"))
            .unwrap(),
        KnownHostStatus::Unknown
    );
    vault
        .trust_host_key("Example.com", 22, &key("AAA"))
        .unwrap();
    assert_eq!(
        vault
            .check_host_key("example.com", 22, &key("AAA"))
            .unwrap(),
        KnownHostStatus::Trusted
    );
    assert_eq!(
        vault
            .check_host_key("example.com", 2222, &key("AAA"))
            .unwrap(),
        KnownHostStatus::Unknown
    );
    assert!(matches!(
        vault.check_host_key("example.com", 22, &key("BBB")).unwrap(),
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
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);
             CREATE TABLE hosts (id TEXT PRIMARY KEY, name TEXT NOT NULL, group_name TEXT,
                 hostname TEXT NOT NULL, port INTEGER NOT NULL, username TEXT NOT NULL,
                 auth TEXT NOT NULL, tags TEXT NOT NULL, created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL);
             CREATE TABLE host_secrets (host_id TEXT PRIMARY KEY REFERENCES hosts(id) ON DELETE CASCADE,
                 sealed BLOB NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE known_hosts (hostname TEXT NOT NULL, port INTEGER NOT NULL,
                 algorithm TEXT NOT NULL, public_key TEXT NOT NULL, fingerprint TEXT NOT NULL,
                 added_at INTEGER NOT NULL, PRIMARY KEY (hostname, port));
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    let vault = Vault::open(&path, Arc::new(MemoryKeyCache::default())).unwrap();
    assert!(vault.list_forwards(None).unwrap().is_empty());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}
