//! The WebDAV backend against a minimal in-process WebDAV server.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use rove_sync::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[derive(Default)]
struct Dav {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
}

/// `user:pass` in the basic authentication header the server accepts.
const AUTH: &str = "Basic dXNlcjpwYXNz";

fn reply(code: u16, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {code} X\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn handle(
    dav: &Mutex<Dav>,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: Vec<u8>,
) -> Vec<u8> {
    if auth != Some(AUTH) {
        return reply(401, b"");
    }
    let mut dav = dav.lock().unwrap();
    let path = path.trim_end_matches('/').to_owned();
    // `/dav` is the server's root: it always exists.
    let exists = |dav: &Dav, dir: &str| dir == "/dav" || dav.dirs.contains(dir);
    let parent = path
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_default();
    match method {
        "MKCOL" => {
            if path == "/dav" {
                // Like some hosted servers: the root cannot be created, even though it exists.
                return reply(403, b"");
            }
            if exists(&dav, &path) {
                return reply(405, b"");
            }
            if !exists(&dav, &parent) {
                return reply(409, b"");
            }
            dav.dirs.insert(path);
            reply(201, b"")
        }
        "PUT" => {
            if !exists(&dav, &parent) {
                return reply(409, b"");
            }
            dav.files.insert(path, body);
            reply(201, b"")
        }
        "GET" => match dav.files.get(&path) {
            Some(data) => reply(200, data),
            None => reply(404, b""),
        },
        "DELETE" => match dav.files.remove(&path) {
            Some(_) => reply(204, b""),
            None => reply(404, b""),
        },
        "PROPFIND" => {
            if !exists(&dav, &path) {
                return reply(404, b"");
            }
            let prefix = format!("{path}/");
            let mut xml = format!(
                "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>{prefix}</d:href></d:response>"
            );
            let children = dav.files.keys().chain(dav.dirs.iter()).filter(|entry| {
                entry
                    .strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
            });
            for child in children {
                xml.push_str(&format!(
                    "<d:response><d:href>{child}</d:href></d:response>"
                ));
            }
            xml.push_str("</d:multistatus>");
            reply(207, xml.as_bytes())
        }
        _ => reply(405, b""),
    }
}

/// Serves WebDAV under `/dav/` on a loopback port; returns the base URL.
async fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let dav = Arc::new(Mutex::new(Dav::default()));
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let dav = dav.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut reader = BufReader::new(read);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or_default().to_owned();
                    let path = parts.next().unwrap_or_default().to_owned();
                    let (mut length, mut auth) = (0usize, None);
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).await.unwrap();
                        let header = header.trim_end();
                        if header.is_empty() {
                            break;
                        }
                        let (name, value) = header.split_once(':').unwrap();
                        match name.to_ascii_lowercase().as_str() {
                            "content-length" => length = value.trim().parse().unwrap(),
                            "authorization" => auth = Some(value.trim().to_owned()),
                            _ => {}
                        }
                    }
                    let mut body = vec![0u8; length];
                    reader.read_exact(&mut body).await.unwrap();
                    let response = handle(&dav, &method, &path, auth.as_deref(), body);
                    if write.write_all(&response).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    format!("http://127.0.0.1:{port}/dav")
}

fn fast() -> KdfParams {
    KdfParams {
        m: 8 * 1024,
        t: 1,
        p: 1,
    }
}

#[derive(Default)]
struct Store(Mutex<BTreeMap<(String, String), serde_json::Value>>);

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
    fn apply(&self, changes: &[Change]) -> Result<(), SyncError> {
        let mut records = self.0.lock().unwrap();
        for change in changes {
            let key = (change.kind.clone(), change.id.clone());
            match &change.body {
                Some(body) => records.insert(key, body.clone()),
                None => records.remove(&key),
            };
        }
        Ok(())
    }
}

#[tokio::test]
async fn two_devices_sync_through_webdav() {
    let url = serve().await;
    let backend = WebDavBackend::new(&url, "user", "pass").unwrap();
    let key = create_space(&backend, "pw", fast()).await.unwrap();

    let a = Store::default();
    a.0.lock().unwrap().insert(
        ("host".into(), "h1".into()),
        serde_json::json!({"name": "web"}),
    );
    let mut a_state = SyncState::new("device-a");
    assert_eq!(
        sync(&backend, &key, &mut a_state, &a).await.unwrap().pushed,
        1
    );

    // Another device with its own connection joins with the password.
    let other = WebDavBackend::new(&url, "user", "pass").unwrap();
    let joined = join_space(&other, "pw").await.unwrap();
    let b = Store::default();
    let mut b_state = SyncState::new("device-b");
    assert_eq!(
        sync(&other, &joined, &mut b_state, &b)
            .await
            .unwrap()
            .applied,
        1
    );
    assert_eq!(*b.0.lock().unwrap(), *a.0.lock().unwrap());
    assert_eq!(other.list("devices").await.unwrap(), ["device-a"]);

    // Deleting a file works, and so does deleting one that is already gone.
    other.put("scratch/x", b"1").await.unwrap();
    other.delete("scratch/x").await.unwrap();
    other.delete("scratch/x").await.unwrap();
    assert_eq!(other.get("scratch/x").await.unwrap(), None);
}

#[tokio::test]
async fn a_missing_sync_folder_is_created() {
    let root = serve().await;
    let backend = WebDavBackend::new(&format!("{root}/RoveTerm"), "user", "pass").unwrap();
    let key = create_space(&backend, "pw", fast()).await.unwrap();
    let other = WebDavBackend::new(&format!("{root}/RoveTerm/"), "user", "pass").unwrap();
    assert!(space_exists(&other).await.unwrap());
    assert_eq!(
        join_space(&other, "pw").await.unwrap().to_bytes(),
        key.to_bytes()
    );
}

#[tokio::test]
async fn wrong_credentials_are_reported() {
    let url = serve().await;
    let backend = WebDavBackend::new(&url, "user", "nope").unwrap();
    assert!(matches!(
        backend.list("").await,
        Err(SyncError::Unauthorized)
    ));
    assert!(matches!(
        create_space(&backend, "pw", fast()).await,
        Err(SyncError::Unauthorized)
    ));
}
