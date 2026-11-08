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
