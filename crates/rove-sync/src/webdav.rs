//! A WebDAV server (Nextcloud, Jianguoyun, a NAS…) as a sync backend. Uses `PROPFIND` to list,
//! `GET` / `PUT` / `DELETE` for files and `MKCOL` to create directories. HTTPS is required except
//! for loopback addresses.

use std::collections::HashSet;
use std::sync::Mutex;

use async_trait::async_trait;
use reqwest::{Client, Method, StatusCode, Url};
use zeroize::Zeroizing;

use crate::backend::{MAX_FILE, check_path};
use crate::{SyncBackend, SyncError};

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;

/// A directory on a WebDAV server.
pub struct WebDavBackend {
    client: Client,
    /// Directory URL, ending in `/`.
    base: Url,
    user: String,
    password: Zeroizing<String>,
    /// Directories known to exist, so writes do not repeat `MKCOL`.
    created: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for WebDavBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDavBackend")
            .field("base", &self.base.as_str())
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// Whether `url` points at this machine, where plain HTTP is acceptable (tests, local servers).
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

impl WebDavBackend {
    /// Uses the directory at `url` with basic authentication. Fails with
    /// [`SyncError::Format`] for a URL that is not `https` (or `http` on a loopback address) or
    /// that carries credentials or a query.
    pub fn new(url: &str, user: &str, password: &str) -> Result<Self, SyncError> {
        let mut base = Url::parse(url.trim())
            .map_err(|error| SyncError::Format(format!("invalid WebDAV address: {error}")))?;
        let secure = base.scheme() == "https" || (base.scheme() == "http" && is_loopback(&base));
        if !secure || base.cannot_be_a_base() {
            return Err(SyncError::Format(
                "the WebDAV address must use https".into(),
            ));
        }
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(SyncError::Format(
                "the WebDAV address must not contain credentials, a query or a fragment".into(),
            ));
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        // reqwest is built without a default crypto provider; ring is the one this app ships.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let builder = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(concat!("RoveTerm/", env!("CARGO_PKG_VERSION")));
        #[cfg(target_os = "android")]
        let builder = builder.tls_backend_preconfigured(android_tls()?);
        let client = builder
            .build()
            .map_err(|error| SyncError::Backend(error.to_string()))?;
        Ok(Self {
            client,
            base,
            user: user.to_owned(),
            password: Zeroizing::new(password.to_owned()),
            created: Mutex::new(HashSet::new()),
        })
    }

    fn url(&self, path: &str, dir: bool) -> Result<Url, SyncError> {
        if path.is_empty() {
            return Ok(self.base.clone());
        }
        check_path(path)?;
        let suffix = if dir { "/" } else { "" };
        self.base
            .join(&format!("{path}{suffix}"))
            .map_err(|error| SyncError::Format(error.to_string()))
    }

    async fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        depth: Option<&str>,
    ) -> Result<reqwest::Response, SyncError> {
        let mut request = self
            .client
            .request(method, url)
            .basic_auth(&self.user, Some(self.password.as_str()));
        if let Some(depth) = depth {
            request = request
                .header("Depth", depth)
                .header("Content-Type", "application/xml; charset=utf-8");
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| SyncError::Backend(error.without_url().to_string()))?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SyncError::Unauthorized),
            _ => Ok(response),
        }
    }

    fn unexpected(response: &reqwest::Response) -> SyncError {
        SyncError::Backend(format!("unexpected response {}", response.status()))
    }

    fn method(name: &[u8]) -> Result<Method, SyncError> {
        Method::from_bytes(name).map_err(|error| SyncError::Backend(error.to_string()))
    }

    fn is_created(&self, dir: &str) -> bool {
        self.created
            .lock()
            .map(|created| created.contains(dir))
            .unwrap_or(false)
    }

    fn mark_created(&self, dir: &str) {
        if let Ok(mut created) = self.created.lock() {
            created.insert(dir.to_owned());
        }
    }

    /// Creates the configured directory if it does not exist yet. Checked with a `PROPFIND`
    /// first: some servers refuse `MKCOL` on their root even when it exists.
    async fn ensure_base(&self) -> Result<(), SyncError> {
        if self.is_created("") {
            return Ok(());
        }
        let response = self
            .send(
                Self::method(b"PROPFIND")?,
                self.base.clone(),
                Some(PROPFIND_BODY.as_bytes().to_vec()),
                Some("0"),
            )
            .await?;
        match response.status().as_u16() {
            207 => {}
            404 => {
                let response = self
                    .send(Self::method(b"MKCOL")?, self.base.clone(), None, None)
                    .await?;
                if !(response.status().is_success()
                    || response.status() == StatusCode::METHOD_NOT_ALLOWED)
                {
                    return Err(Self::unexpected(&response));
                }
            }
            _ => return Err(Self::unexpected(&response)),
        }
        self.mark_created("");
        Ok(())
    }

    /// Creates `dir` (relative to the configured directory) and its parents unless known to
    /// exist; an empty `dir` is the configured directory itself.
    async fn ensure_dir(&self, dir: &str) -> Result<(), SyncError> {
        self.ensure_base().await?;
        let mut prefix = String::new();
        for part in dir.split('/').filter(|part| !part.is_empty()) {
            prefix = if prefix.is_empty() {
                part.to_owned()
            } else {
                format!("{prefix}/{part}")
            };
            if self.is_created(&prefix) {
                continue;
            }
            let response = self
                .send(
                    Self::method(b"MKCOL")?,
                    self.url(&prefix, true)?,
                    None,
                    None,
                )
                .await?;
            // 405: it already exists.
            if !(response.status().is_success()
                || response.status() == StatusCode::METHOD_NOT_ALLOWED)
            {
                return Err(Self::unexpected(&response));
            }
            self.mark_created(&prefix);
        }
        Ok(())
    }
}

/// TLS for Android, where the system verifier would first have to be initialised over JNI:
/// trusts the Mozilla root set bundled with `webpki-roots`, so a server whose certificate chains
/// only to a user-installed CA is rejected there.
#[cfg(target_os = "android")]
fn android_tls() -> Result<rustls::ClientConfig, SyncError> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| SyncError::Backend(error.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// Names of the members of `dir_url` in a `PROPFIND` multistatus response.
fn member_names(xml: &str, dir_url: &Url) -> Result<Vec<String>, SyncError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut in_href = false;
    let mut names = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| SyncError::Format(format!("WebDAV listing: {error}")))?
        {
            quick_xml::events::Event::Start(start) => {
                in_href = AsRef::<str>::as_ref(&start.local_name()) == "href"
            }
            quick_xml::events::Event::End(_) => in_href = false,
            quick_xml::events::Event::Text(text) if in_href => {
                // Hrefs are percent-encoded; `&amp;` is the only entity a server would use in one.
                let href = text.into_inner().replace("&amp;", "&");
                let href = dir_url
                    .join(href.trim())
                    .map_err(|error| SyncError::Format(error.to_string()))?;
                let path = href.path().trim_end_matches('/');
                if path == dir_url.path().trim_end_matches('/') {
                    continue;
                }
                if let Some(name) = path.rsplit('/').next() {
                    let name = percent_decode(name);
                    if check_path(&name).is_ok() {
                        names.push(name);
                    }
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[async_trait]
impl SyncBackend for WebDavBackend {
    async fn list(&self, dir: &str) -> Result<Vec<String>, SyncError> {
        let url = self.url(dir, true)?;
        let response = self
            .send(
                Self::method(b"PROPFIND")?,
                url.clone(),
                Some(PROPFIND_BODY.as_bytes().to_vec()),
                Some("1"),
            )
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if response.status().as_u16() != 207 {
            return Err(Self::unexpected(&response));
        }
        let text = response
            .text()
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?;
        if text.len() > MAX_FILE {
            return Err(SyncError::Format("WebDAV listing too large".into()));
        }
        member_names(&text, &url)
    }

    async fn get(&self, path: &str) -> Result<Option<Vec<u8>>, SyncError> {
        let response = self
            .send(Method::GET, self.url(path, false)?, None, None)
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(Self::unexpected(&response));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_FILE as u64)
        {
            return Err(SyncError::Format("sync file too large".into()));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| SyncError::Backend(error.to_string()))?;
        if bytes.len() > MAX_FILE {
            return Err(SyncError::Format("sync file too large".into()));
        }
        Ok(Some(bytes.to_vec()))
    }

    async fn put(&self, path: &str, data: &[u8]) -> Result<(), SyncError> {
        check_path(path)?;
        self.ensure_dir(path.rsplit_once('/').map_or("", |(dir, _)| dir))
            .await?;
        let response = self
            .send(
                Method::PUT,
                self.url(path, false)?,
                Some(data.to_vec()),
                None,
            )
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Self::unexpected(&response))
        }
    }

    async fn delete(&self, path: &str) -> Result<(), SyncError> {
        let response = self
            .send(Method::DELETE, self.url(path, false)?, None, None)
            .await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(Self::unexpected(&response))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_https_except_on_loopback() {
        assert!(WebDavBackend::new("http://example.com/dav", "u", "p").is_err());
        assert!(WebDavBackend::new("https://user:pw@example.com/dav", "u", "p").is_err());
        assert!(WebDavBackend::new("ftp://example.com/", "u", "p").is_err());
        assert!(WebDavBackend::new("https://example.com/dav", "u", "p").is_ok());
        assert!(WebDavBackend::new("http://127.0.0.1:8080/dav", "u", "p").is_ok());
    }

    #[test]
    fn reads_member_names_from_a_listing() {
        let dir = Url::parse("https://dav.example/remote.php/dav/files/me/rove/devices/").unwrap();
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response><d:href>/remote.php/dav/files/me/rove/devices/</d:href></d:response>
  <d:response><d:href>/remote.php/dav/files/me/rove/devices/a-1/</d:href></d:response>
  <d:response><d:href>https://dav.example/remote.php/dav/files/me/rove/devices/b%2D2/</d:href></d:response>
  <D:response xmlns:D="DAV:"><D:href>0000000000000001.seg</D:href></D:response>
  <d:response><d:href>/remote.php/dav/files/me/rove/devices/bad%20name</d:href></d:response>
</d:multistatus>"#;
        assert_eq!(
            member_names(xml, &dir).unwrap(),
            ["0000000000000001.seg", "a-1", "b-2"]
        );
    }
}
