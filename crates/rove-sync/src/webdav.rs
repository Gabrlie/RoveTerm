//! A WebDAV server (Nextcloud, Jianguoyun, a NAS…) as a sync backend. Uses `PROPFIND` to list,
//! `GET` / `PUT` / `DELETE` for files and `MKCOL` to create directories. HTTPS is required except
//! for loopback addresses.

use std::collections::HashSet;
use std::error::Error as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rove_http::reqwest::{Client, Method, Response, StatusCode, Url, header, redirect};
use zeroize::Zeroizing;

use crate::backend::{MAX_FILE, check_path};
use crate::{SyncBackend, SyncError};

/// Formats a `Url` as `scheme://host[:port]/path`, omitting credentials, query and fragment.
fn sanitized_target(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}{}", url.scheme(), url.path()),
        None => format!("{}://{host}{}", url.scheme(), url.path()),
    }
}

/// Expands a reqwest error with its underlying `Error::source` chain (DNS, TLS, timeout, reset),
/// stripping the URL so credentials or queries can never appear in logs or user messages.
fn describe_reqwest_error(error: rove_http::reqwest::Error) -> String {
    let error = error.without_url();
    let is_timeout = error.is_timeout();
    let mut parts = Vec::new();
    let base = error.to_string();
    if !base.is_empty() {
        parts.push(base);
    }
    let mut current = error.source();
    while let Some(source) = current {
        let text = source.to_string();
        if !text.is_empty() && !parts.iter().any(|existing| existing.contains(&text)) {
            parts.push(text);
        }
        current = source.source();
    }
    let detail = parts.join(": ");
    if is_timeout && !detail.to_ascii_lowercase().contains("timed out") {
        format!("request timed out: {detail}")
    } else if detail.is_empty() {
        "network request failed".to_owned()
    } else {
        detail
    }
}

/// Classifies auth/permission HTTP statuses into distinct [`SyncError`] variants.
fn classify_auth_status(method: &str, path: &str, status: StatusCode) -> Option<SyncError> {
    match status {
        StatusCode::UNAUTHORIZED => Some(SyncError::Unauthorized),
        StatusCode::FORBIDDEN => Some(SyncError::Forbidden(format!(
            "HTTP 403 Forbidden ({method} {path})"
        ))),
        _ => None,
    }
}

/// Formats an unexpected HTTP status with method, sanitized path and a short diagnostic hint.
fn unexpected_status(method: &str, path: &str, status: StatusCode) -> SyncError {
    let hint = match status.as_u16() {
        404 => Some("not found — check the server URL or folder path"),
        405 => Some("method not allowed — this URL may not be a WebDAV endpoint"),
        409 => Some("conflict — parent folder may not exist on the server"),
        413 => Some("payload too large"),
        429 => Some("too many requests — server rate limit reached"),
        500..=599 => Some("server error"),
        _ => None,
    };
    let message = match hint {
        Some(hint) => format!("HTTP {status} on {method} {path} ({hint})"),
        None => format!("unexpected HTTP {status} on {method} {path}"),
    };
    SyncError::Backend(message)
}

const MAX_REDIRECTS: usize = 5;

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
    #[cfg(test)]
    referer_override: Option<(&'static str, &'static str)>,
}

impl std::fmt::Debug for WebDavBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDavBackend")
            .field("base", &sanitized_target(&self.base))
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

fn secure_transport(url: &Url) -> bool {
    url.scheme() == "https" || (url.scheme() == "http" && is_loopback(url))
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str()
            .zip(b.host_str())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && a.port_or_known_default() == b.port_or_known_default()
}

fn cdn_referer_for_host(host: &str) -> Option<&'static str> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let matches = |domain: &str| {
        host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    };
    if ["aliyundrive.net", "aliyundrive.com", "alipan.com"]
        .into_iter()
        .any(matches)
    {
        Some("https://www.alipan.com/")
    } else if matches("baidupcs.com") {
        Some("https://pan.baidu.com/")
    } else if matches("quark.cn") {
        Some("https://pan.quark.cn/")
    } else if matches("115cdn.net") || matches("115.com") {
        Some("https://115.com/")
    } else {
        None
    }
}

fn redirect_location(response: &Response) -> Option<&header::HeaderValue> {
    match response.status() {
        StatusCode::MOVED_PERMANENTLY
        | StatusCode::FOUND
        | StatusCode::SEE_OTHER
        | StatusCode::TEMPORARY_REDIRECT
        | StatusCode::PERMANENT_REDIRECT => response.headers().get(header::LOCATION),
        _ => None,
    }
}

fn resolve_redirect(url: &Url, location: &header::HeaderValue) -> Result<Url, SyncError> {
    let target = location
        .to_str()
        .ok()
        .and_then(|location| url.join(location).ok())
        .ok_or_else(|| SyncError::Backend("invalid WebDAV redirect Location".into()))?;
    if !secure_transport(&target) {
        return Err(SyncError::Backend(
            "WebDAV redirect must use HTTPS or loopback HTTP".into(),
        ));
    }
    if !target.username().is_empty() || target.password().is_some() {
        return Err(SyncError::Backend(
            "WebDAV redirect URL must not contain credentials".into(),
        ));
    }
    Ok(target)
}

impl WebDavBackend {
    /// Uses the directory at `url` with basic authentication. Fails with
    /// [`SyncError::Format`] for a URL that is not `https` (or `http` on a loopback address) or
    /// that carries credentials or a query.
    pub fn new(url: &str, user: &str, password: &str) -> Result<Self, SyncError> {
        let mut base = Url::parse(url.trim())
            .map_err(|error| SyncError::Format(format!("invalid WebDAV address: {error}")))?;
        if !secure_transport(&base) || base.cannot_be_a_base() {
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
        let client = rove_http::builder(Some(Duration::from_secs(60)))
            .map_err(|error| SyncError::Backend(error.to_string()))?
            .redirect(redirect::Policy::none())
            .build()
            .map_err(|error| SyncError::Backend(error.to_string()))?;
        Ok(Self {
            client,
            base,
            user: user.to_owned(),
            password: Zeroizing::new(password.to_owned()),
            created: Mutex::new(HashSet::new()),
            #[cfg(test)]
            referer_override: None,
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
    ) -> Result<Response, SyncError> {
        let headers: &[(&str, &str)] = match depth {
            Some(depth) => &[
                ("Depth", depth),
                ("Content-Type", "application/xml; charset=utf-8"),
            ],
            None => &[],
        };
        self.send_with(method, url, body, headers).await
    }

    async fn send_with(
        &self,
        mut method: Method,
        mut url: Url,
        mut body: Option<Vec<u8>>,
        headers: &[(&str, &str)],
    ) -> Result<Response, SyncError> {
        let mut hops = 0;
        let mut referer = None;
        let mut retried = false;
        loop {
            let same_origin = same_origin(&self.base, &url);
            let target = sanitized_target(&url);
            let started = Instant::now();
            tracing::debug!(%method, %target, "webdav request");

            let mut request = self.client.request(method.clone(), url.clone());
            if same_origin {
                request = request.basic_auth(&self.user, Some(self.password.as_str()));
                for (name, value) in headers {
                    request = request.header(*name, *value);
                }
            } else if let Some(referer) = referer {
                request = request.header(header::REFERER, referer);
            }
            if let Some(body) = &body {
                request = request.body(body.clone());
            }
            let response = request.send().await.map_err(|error| {
                let detail = describe_reqwest_error(error);
                tracing::warn!(
                    %method, %target,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    error = %detail,
                    "webdav request failed"
                );
                SyncError::Backend(format!("{method} {}: {detail}", url.path()))
            })?;
            let status = response.status();
            let elapsed_ms = started.elapsed().as_millis() as u64;

            if let Some(location) = redirect_location(&response) {
                if hops == MAX_REDIRECTS {
                    return Err(SyncError::Backend(format!(
                        "too many redirects on {method} {target}"
                    )));
                }
                let next = resolve_redirect(&url, location)?;
                tracing::debug!(
                    %method, %target, redirected_to = %sanitized_target(&next),
                    status = status.as_u16(),
                    "webdav redirect"
                );
                if status == StatusCode::SEE_OTHER
                    || (matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND)
                        && method != Method::GET
                        && method != Method::HEAD)
                {
                    method = Method::GET;
                    body = None;
                }
                url = next;
                hops += 1;
                referer = None;
                continue;
            }

            if !same_origin && status == StatusCode::FORBIDDEN {
                let host = url.host_str().unwrap_or_default();
                if method == Method::GET
                    && !retried
                    && let Some(cdn_referer) = self.cdn_referer(host)
                {
                    tracing::debug!(%target, "retrying WebDAV redirect with CDN Referer");
                    referer = Some(cdn_referer);
                    retried = true;
                    continue;
                }
                tracing::warn!(%host, elapsed_ms, "webdav redirect returned HTTP 403");
                return Err(SyncError::Forbidden(format!(
                    "HTTP 403 Forbidden from redirected host {host}. Try switching the OpenList/AList WebDAV policy to Native Proxy (本地代理)."
                )));
            }
            if let Some(error) = classify_auth_status(method.as_str(), url.path(), status) {
                tracing::warn!(%method, %target, status = status.as_u16(), elapsed_ms, "webdav access rejected");
                return Err(error);
            }
            tracing::debug!(%method, %target, status = status.as_u16(), elapsed_ms, "webdav response");
            return Ok(response);
        }
    }

    fn cdn_referer(&self, host: &str) -> Option<&'static str> {
        #[cfg(test)]
        if let Some((test_host, referer)) = self.referer_override
            && host == test_host
        {
            return Some(referer);
        }
        cdn_referer_for_host(host)
    }

    fn unexpected(method: &str, response: &Response) -> SyncError {
        let status = response.status();
        let path = response.url().path();
        tracing::warn!(
            method,
            target = %sanitized_target(response.url()),
            status = status.as_u16(),
            "webdav unexpected status"
        );
        unexpected_status(method, path, status)
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
                    return Err(Self::unexpected("MKCOL", &response));
                }
            }
            _ => return Err(Self::unexpected("PROPFIND", &response)),
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
                return Err(Self::unexpected("MKCOL", &response));
            }
            self.mark_created(&prefix);
        }
        Ok(())
    }
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
            return Err(Self::unexpected("PROPFIND", &response));
        }
        let text = response
            .text()
            .await
            .map_err(|error| SyncError::Backend(describe_reqwest_error(error)))?;
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
            return Err(Self::unexpected("GET", &response));
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
            .map_err(|error| SyncError::Backend(describe_reqwest_error(error)))?;
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
            Err(Self::unexpected("PUT", &response))
        }
    }

    async fn put_new(&self, path: &str, data: &[u8]) -> Result<bool, SyncError> {
        check_path(path)?;
        self.ensure_dir(path.rsplit_once('/').map_or("", |(dir, _)| dir))
            .await?;
        // RFC 7232: the server refuses the write with 412 when the file exists.
        let response = self
            .send_with(
                Method::PUT,
                self.url(path, false)?,
                Some(data.to_vec()),
                &[("If-None-Match", "*")],
            )
            .await?;
        match response.status() {
            status if status.is_success() => Ok(true),
            StatusCode::PRECONDITION_FAILED => Ok(false),
            _ => Err(Self::unexpected("PUT", &response)),
        }
    }

    async fn delete(&self, path: &str) -> Result<(), SyncError> {
        let response = self
            .send(Method::DELETE, self.url(path, false)?, None, None)
            .await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(Self::unexpected("DELETE", &response))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    async fn mock_listener() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    async fn exchange(listener: &TcpListener, response: &str) -> (String, Vec<u8>) {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut request = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let line = line.to_ascii_lowercase();
                if let Some(value) = line.strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                request.push_str(&line);
            }
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
            (request, body)
        })
        .await
        .unwrap()
    }

    const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata";
    const FORBIDDEN: &str =
        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    fn redirect_to(status: &str, location: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn mock_backend(port: u16) -> WebDavBackend {
        let mut backend =
            WebDavBackend::new(&format!("http://127.0.0.1:{port}/dav/"), "", "").unwrap();
        backend.client = rove_http::builder(Some(Duration::from_secs(5)))
            .unwrap()
            .redirect(redirect::Policy::none())
            .no_proxy()
            .build()
            .unwrap();
        backend
    }

    #[tokio::test]
    async fn same_origin_redirect_preserves_basic_auth_and_reads_body() {
        let (listener, port) = mock_listener().await;
        let server = tokio::spawn(async move {
            let (initial, _) = exchange(&listener, &redirect_to("302 Found", "redirected")).await;
            let (redirected, _) = exchange(&listener, OK).await;
            assert!(initial.starts_with("get /dav/space.json "));
            assert!(redirected.starts_with("get /dav/redirected "));
            for request in [initial, redirected] {
                assert!(request.contains("\r\nauthorization: basic "));
            }
        });
        assert_eq!(
            mock_backend(port)
                .get("space.json")
                .await
                .unwrap()
                .as_deref(),
            Some(&b"data"[..])
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cross_origin_redirect_strips_basic_auth_and_webdav_headers() {
        let (listener, port) = mock_listener().await;
        let server = tokio::spawn(async move {
            let location = format!("http://localhost:{port}/redirected");
            let (initial, _) = exchange(&listener, &redirect_to("302 Found", &location)).await;
            let (redirected, _) = exchange(&listener, OK).await;
            assert!(initial.contains("\r\nauthorization: basic "));
            assert!(initial.contains("\r\ndepth: 1\r\n"));
            assert!(redirected.starts_with("get /redirected "));
            assert!(!redirected.contains("\r\nauthorization:"));
            assert!(!redirected.contains("\r\ndepth:"));
        });
        let backend = mock_backend(port);
        let response = backend
            .send_with(
                Method::GET,
                backend.url("space.json", false).unwrap(),
                None,
                &[("Depth", "1")],
            )
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "data");
        server.await.unwrap();
    }

    #[test]
    fn matches_only_supported_cdn_domains_and_their_subdomains() {
        for (domain, referer) in [
            ("aliyundrive.net", "https://www.alipan.com/"),
            ("aliyundrive.com", "https://www.alipan.com/"),
            ("alipan.com", "https://www.alipan.com/"),
            ("baidupcs.com", "https://pan.baidu.com/"),
            ("quark.cn", "https://pan.quark.cn/"),
            ("115cdn.net", "https://115.com/"),
            ("115.com", "https://115.com/"),
        ] {
            assert_eq!(cdn_referer_for_host(domain), Some(referer));
            assert_eq!(
                cdn_referer_for_host(&format!("cdn.{domain}")),
                Some(referer)
            );
            assert_eq!(cdn_referer_for_host(&format!("other{domain}")), None);
            assert_eq!(cdn_referer_for_host(&format!("{domain}.example.org")), None);
        }
    }

    #[tokio::test]
    async fn cross_origin_cdn_403_retries_once_with_referer() {
        for (response, expected) in [(OK, Some(&b"data"[..])), (FORBIDDEN, None)] {
            let (listener, port) = mock_listener().await;
            let server = tokio::spawn(async move {
                let location = format!("http://localhost:{port}/redirected?test=fixture");
                exchange(&listener, &redirect_to("302 Found", &location)).await;
                let (initial, _) = exchange(&listener, FORBIDDEN).await;
                let (retry, _) = exchange(&listener, response).await;
                assert!(!initial.contains("\r\nreferer:"));
                assert!(retry.contains("\r\nreferer: https://www.alipan.com/\r\n"));
                for request in [initial, retry] {
                    assert!(request.starts_with("get /redirected?test=fixture "));
                    assert!(!request.contains("\r\nauthorization:"));
                }
            });
            let mut backend = mock_backend(port);
            backend.referer_override = Some(("localhost", "https://www.alipan.com/"));
            let result = backend.get("space.json").await;
            if let Some(expected) = expected {
                assert_eq!(result.unwrap().as_deref(), Some(expected));
            } else {
                let Err(SyncError::Forbidden(detail)) = result else {
                    panic!("expected Forbidden, got {result:?}");
                };
                assert!(detail.contains("localhost"));
                assert!(detail.contains("Native Proxy (本地代理)"));
                assert!(!detail.contains("fixture"));
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn cross_origin_403_on_other_host_suggests_native_proxy() {
        let (listener, port) = mock_listener().await;
        let server = tokio::spawn(async move {
            let location = format!("http://localhost:{port}/redirected?test=fixture");
            exchange(&listener, &redirect_to("302 Found", &location)).await;
            exchange(&listener, FORBIDDEN).await;
        });
        let result = mock_backend(port).get("space.json").await;
        let Err(SyncError::Forbidden(detail)) = result else {
            panic!("expected Forbidden, got {result:?}");
        };
        assert!(detail.contains("localhost"));
        assert!(detail.contains("Native Proxy (本地代理)"));
        assert!(!detail.contains("fixture"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn redirects_handle_method_and_body_according_to_status() {
        for (status, preserve) in [
            ("301 Moved Permanently", false),
            ("302 Found", false),
            ("303 See Other", false),
            ("307 Temporary Redirect", true),
            ("308 Permanent Redirect", true),
        ] {
            let (listener, port) = mock_listener().await;
            let server = tokio::spawn(async move {
                let (initial, body) =
                    exchange(&listener, &redirect_to(status, "/redirected")).await;
                assert!(initial.starts_with("put /dav/space.json "));
                assert_eq!(body, b"data");
                let (redirected, body) = exchange(&listener, OK).await;
                if preserve {
                    assert!(redirected.starts_with("put /redirected "));
                    assert_eq!(body, b"data");
                } else {
                    assert!(redirected.starts_with("get /redirected "));
                    assert!(body.is_empty());
                }
            });
            let backend = mock_backend(port);
            backend
                .send(
                    Method::PUT,
                    backend.url("space.json", false).unwrap(),
                    Some(b"data".to_vec()),
                    None,
                )
                .await
                .unwrap();
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn caps_redirects_at_five_hops() {
        let (listener, port) = mock_listener().await;
        let server = tokio::spawn(async move {
            for _ in 0..=MAX_REDIRECTS {
                exchange(&listener, &redirect_to("302 Found", "/redirected")).await;
            }
        });
        let result = mock_backend(port).get("space.json").await;
        assert!(
            matches!(result, Err(SyncError::Backend(detail)) if detail.contains("too many redirects"))
        );
        server.await.unwrap();
    }

    #[test]
    fn rejects_insecure_or_credential_bearing_redirects() {
        let base = Url::parse("https://dav.example/dav/space.json").unwrap();
        for location in [
            "http://example.com/file",
            "ftp://example.com/file",
            "https://user@example.com/file",
        ] {
            assert!(resolve_redirect(&base, &header::HeaderValue::from_static(location)).is_err());
        }
    }

    #[test]
    fn requires_https_except_on_loopback() {
        assert!(WebDavBackend::new("http://example.com/dav", "u", "p").is_err());
        assert!(WebDavBackend::new("https://user:pw@example.com/dav", "u", "p").is_err());
        assert!(WebDavBackend::new("ftp://example.com/", "u", "p").is_err());
        assert!(WebDavBackend::new("https://example.com/dav", "u", "p").is_ok());
        assert!(WebDavBackend::new("http://127.0.0.1:8080/dav", "u", "p").is_ok());
    }

    #[test]
    fn classifies_401_vs_403_and_unexpected_statuses() {
        assert!(matches!(
            classify_auth_status("GET", "/dav/space.json", StatusCode::UNAUTHORIZED),
            Some(SyncError::Unauthorized)
        ));
        match classify_auth_status("PROPFIND", "/dav/RoveTerm/", StatusCode::FORBIDDEN) {
            Some(SyncError::Forbidden(detail)) => {
                assert!(detail.contains("403"));
                assert!(detail.contains("PROPFIND"));
                assert!(detail.contains("/dav/RoveTerm/"));
            }
            other => panic!("expected Forbidden, got {other:?}"),
        }
        assert!(classify_auth_status("GET", "/dav/space.json", StatusCode::NOT_FOUND).is_none());

        match unexpected_status("PROPFIND", "/dav/", StatusCode::METHOD_NOT_ALLOWED) {
            SyncError::Backend(detail) => {
                assert!(detail.contains("405"));
                assert!(detail.contains("PROPFIND"));
                assert!(detail.contains("WebDAV"));
            }
            other => panic!("expected Backend, got {other:?}"),
        }
        match unexpected_status("MKCOL", "/dav/RoveTerm/", StatusCode::CONFLICT) {
            SyncError::Backend(detail) => {
                assert!(detail.contains("409"));
                assert!(detail.contains("MKCOL"));
            }
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn sanitizes_logged_target_without_credentials_or_query() {
        let url = Url::parse("https://user:secret@dav.example.com:8443/dav/RoveTerm/?token=abc#f")
            .unwrap();
        let target = sanitized_target(&url);
        assert_eq!(target, "https://dav.example.com:8443/dav/RoveTerm/");
        assert!(!target.contains("secret"));
        assert!(!target.contains("token"));
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
