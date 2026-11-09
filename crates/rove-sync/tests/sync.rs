use std::collections::BTreeMap;
use std::sync::Mutex;

use rove_sync::*;
use serde_json::{Value, json};

/// A device's data: records by (kind, id).
#[derive(Default)]
struct Store(Mutex<BTreeMap<(String, String), Value>>);

impl Store {
    fn set(&self, kind: &str, id: &str, body: Value) {
        self.0
            .lock()
            .unwrap()
            .insert((kind.into(), id.into()), body);
    }
    fn remove(&self, kind: &str, id: &str) {
        self.0.lock().unwrap().remove(&(kind.into(), id.into()));
    }
    fn get(&self, kind: &str, id: &str) -> Option<Value> {
        self.0
            .lock()
            .unwrap()
            .get(&(kind.into(), id.into()))
            .cloned()
    }
    fn all(&self) -> BTreeMap<(String, String), Value> {
        self.0.lock().unwrap().clone()
    }
}

impl RecordStore for Store {
    fn records(&self) -> Result<Vec<Record>, SyncError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|((kind, id), body)| Record {
                kind: kind.clone(),
                id: id.clone(),
                body: body.clone(),
            })
            .collect())
    }
    fn apply(&self, change: &Change) -> Result<(), SyncError> {
        match &change.body {
            Some(body) => self.set(&change.kind, &change.id, body.clone()),
            None => self.remove(&change.kind, &change.id),
        }
        Ok(())
    }
}

/// Cheap key derivation so tests stay fast.
fn fast() -> KdfParams {
    KdfParams {
        m: 8 * 1024,
        t: 1,
        p: 1,
    }
}

struct Device {
    state: SyncState,
    store: Store,
}

impl Device {
    fn new(id: &str) -> Self {
        Self {
            state: SyncState::new(id),
            store: Store::default(),
        }
    }
    async fn sync(&mut self, backend: &FolderBackend, key: &SyncKey) -> Report {
        sync(backend, key, &mut self.state, &self.store)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn spaces_check_the_password() {
    let dir = tempfile::tempdir().unwrap();
    let backend = FolderBackend::new(dir.path());
    assert!(matches!(
        join_space(&backend, "pw").await,
        Err(SyncError::NoSpace)
    ));
    create_space(&backend, "right", fast()).await.unwrap();
    assert!(matches!(
        create_space(&backend, "again", fast()).await,
        Err(SyncError::SpaceExists)
    ));
    assert!(matches!(
        join_space(&backend, "wrong").await,
        Err(SyncError::WrongPassword)
    ));
    assert!(join_space(&backend, "right").await.is_ok());
}

#[tokio::test]
async fn two_devices_converge_and_settle() {
    let dir = tempfile::tempdir().unwrap();
    let backend = FolderBackend::new(dir.path());
    let key = create_space(&backend, "pw", fast()).await.unwrap();
    let mut a = Device::new("device-a");
    let mut b = Device::new("device-b");

    a.store.set("host", "h1", json!({"name": "web"}));
    a.store.set("snippet", "s1", json!({"command": "uptime"}));
    assert_eq!(a.sync(&backend, &key).await.pushed, 2);
    let report = b.sync(&backend, &key).await;
    assert_eq!((report.segments, report.applied, report.pushed), (1, 2, 0));
    assert_eq!(b.store.all(), a.store.all());

    b.store.set("host", "h1", json!({"name": "web-renamed"}));
    assert_eq!(b.sync(&backend, &key).await.pushed, 1);
    assert_eq!(a.sync(&backend, &key).await.applied, 1);
    assert_eq!(
        a.store.get("host", "h1"),
        Some(json!({"name": "web-renamed"}))
    );

    // Nothing changed: rounds neither push nor apply anything.
    for device in [&mut a, &mut b] {
        let report = device.sync(&backend, &key).await;
        assert_eq!((report.applied, report.pushed), (0, 0));
    }
}

#[tokio::test]
async fn concurrent_edits_keep_the_later_one_and_deletes_stick() {
    let dir = tempfile::tempdir().unwrap();
    let backend = FolderBackend::new(dir.path());
    let key = create_space(&backend, "pw", fast()).await.unwrap();
    let mut a = Device::new("device-a");
    let mut b = Device::new("device-b");
    a.store.set("host", "h1", json!({"v": 0}));
    a.sync(&backend, &key).await;
    b.sync(&backend, &key).await;

    // Both edit; A syncs first, B later: B's change is timestamped later and wins everywhere.
    a.store.set("host", "h1", json!({"v": "a"}));
    b.store.set("host", "h1", json!({"v": "b"}));
    a.sync(&backend, &key).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    b.sync(&backend, &key).await;
    a.sync(&backend, &key).await;
    assert_eq!(a.store.get("host", "h1"), Some(json!({"v": "b"})));
    assert_eq!(b.store.get("host", "h1"), Some(json!({"v": "b"})));

    // A deletes; B learns of it and does not bring the record back.
    a.store.remove("host", "h1");
    assert_eq!(a.sync(&backend, &key).await.pushed, 1);
    b.sync(&backend, &key).await;
    assert_eq!(b.store.get("host", "h1"), None);
    assert_eq!(b.sync(&backend, &key).await.pushed, 0);
    a.sync(&backend, &key).await;
    assert_eq!(a.store.get("host", "h1"), None);

    // A device joining later receives the current state only.
    let mut c = Device::new("device-c");
    c.sync(&backend, &key).await;
    assert_eq!(c.store.all(), a.store.all());
}

#[tokio::test]
async fn altered_or_moved_segments_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let backend = FolderBackend::new(dir.path());
    let key = create_space(&backend, "pw", fast()).await.unwrap();
    let mut a = Device::new("device-a");
    a.store.set("host", "h1", json!({"name": "web"}));
    a.sync(&backend, &key).await;

    let path = "devices/device-a/0000000000000001.seg";
    let original = backend.get(path).await.unwrap().unwrap();

    // Moved to another sequence number.
    backend.delete(path).await.unwrap();
    backend
        .put("devices/device-a/0000000000000002.seg", &original)
        .await
        .unwrap();
    let mut b = Device::new("device-b");
    assert!(matches!(
        sync(&backend, &key, &mut b.state, &b.store).await,
        Err(SyncError::Rejected)
    ));
    assert_eq!(
        b.state,
        SyncState::new("device-b"),
        "a failed round leaves the state unchanged"
    );

    // Altered in place.
    backend
        .delete("devices/device-a/0000000000000002.seg")
        .await
        .unwrap();
    let mut altered = original.clone();
    let last = altered.len() - 1;
    altered[last] ^= 0x40;
    backend.put(path, &altered).await.unwrap();
    assert!(matches!(
        sync(&backend, &key, &mut b.state, &b.store).await,
        Err(SyncError::Rejected)
    ));

    // A key from another password cannot read the space's segments.
    backend.put(path, &original).await.unwrap();
    let other = create_space(&FolderBackend::new(dir.path().join("other")), "pw", fast())
        .await
        .unwrap();
    assert!(matches!(
        sync(&backend, &other, &mut b.state, &b.store).await,
        Err(SyncError::Rejected)
    ));
    assert!(b.store.all().is_empty());
}
