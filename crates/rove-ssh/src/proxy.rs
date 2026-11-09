//! Client side of the proxies a host route may use: HTTP `CONNECT` (RFC 9110 §9.3.6) and SOCKS5
//! `CONNECT` without authentication (RFC 1928). Both leave `stream` connected to the destination.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::SshError;

/// Largest HTTP response head accepted from a proxy.
const MAX_HTTP_HEAD: usize = 8 * 1024;

fn proxy_error(message: impl Into<String>) -> SshError {
    SshError::Proxy(message.into())
}

fn io_error(error: std::io::Error) -> SshError {
    SshError::Proxy(error.to_string())
}

/// Formats `host:port`, bracketing IPv6 literals.
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Asks an HTTP proxy to open a tunnel to `host:port`.
pub(crate) async fn http_connect<S>(stream: &mut S, host: &str, port: u16) -> Result<(), SshError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let target = authority(host, port);
    let request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(io_error)?;
    // Read byte by byte so nothing after the head (the SSH banner) is consumed.
    let mut head = Vec::with_capacity(256);
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HTTP_HEAD {
            return Err(proxy_error("proxy response head is too long"));
        }
        head.push(stream.read_u8().await.map_err(io_error)?);
    }
    let status_line = head
        .split(|&byte| byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).trim().to_owned())
        .unwrap_or_default();
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or_default();
    let code = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(proxy_error(format!("not an HTTP proxy: {status_line:.80}")));
    }
    if !code.starts_with('2') || code.len() != 3 {
        return Err(proxy_error(format!("proxy refused: {status_line:.80}")));
    }
    Ok(())
}

/// Asks a SOCKS5 proxy to connect to `host:port`. The name is resolved by the proxy.
pub(crate) async fn socks5_connect<S>(stream: &mut S, host: &str, port: u16) -> Result<(), SshError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let name = host.as_bytes();
    let name_len = u8::try_from(name.len()).map_err(|_| proxy_error("host name is too long"))?;
    stream.write_all(&[5, 1, 0]).await.map_err(io_error)?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await.map_err(io_error)?;
    if choice[0] != 5 {
        return Err(proxy_error("not a SOCKS5 proxy"));
    }
    if choice[1] != 0 {
        return Err(proxy_error("SOCKS5 proxy requires authentication"));
    }
    let mut request = Vec::with_capacity(7 + name.len());
    request.extend_from_slice(&[5, 1, 0, 3, name_len]);
    request.extend_from_slice(name);
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await.map_err(io_error)?;

    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await.map_err(io_error)?;
    if reply[0] != 5 {
        return Err(proxy_error("malformed SOCKS5 reply"));
    }
    if reply[1] != 0 {
        return Err(proxy_error(format!(
            "SOCKS5 proxy could not connect: {}",
            socks_reply_text(reply[1])
        )));
    }
    // Skip the bound address and port.
    let address_len = match reply[3] {
        1 => 4,
        4 => 16,
        3 => usize::from(stream.read_u8().await.map_err(io_error)?),
        other => return Err(proxy_error(format!("unknown SOCKS5 address type {other}"))),
    };
    let mut bound = vec![0u8; address_len + 2];
    stream.read_exact(&mut bound).await.map_err(io_error)?;
    Ok(())
}

fn socks_reply_text(code: u8) -> &'static str {
    match code {
        1 => "general failure",
        2 => "not allowed by ruleset",
        3 => "network unreachable",
        4 => "host unreachable",
        5 => "connection refused",
        6 => "TTL expired",
        7 => "command not supported",
        8 => "address type not supported",
        _ => "unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `client` against a scripted proxy: for each step it expects the given bytes and sends
    /// the answer, then sends an SSH banner. Returns the client's result and, on success, the
    /// banner as the client reads it afterwards.
    async fn scripted<F, Fut>(
        steps: &[(&[u8], &[u8])],
        client: F,
    ) -> (Result<(), SshError>, Vec<u8>)
    where
        F: FnOnce(tokio::io::DuplexStream) -> Fut,
        Fut: Future<Output = (Result<(), SshError>, tokio::io::DuplexStream)>,
    {
        let (near, mut far) = tokio::io::duplex(1024);
        let steps: Vec<(Vec<u8>, Vec<u8>)> = steps
            .iter()
            .map(|(expect, answer)| (expect.to_vec(), answer.to_vec()))
            .collect();
        let proxy = tokio::spawn(async move {
            for (expect, answer) in steps {
                let mut got = vec![0u8; expect.len()];
                far.read_exact(&mut got).await.unwrap();
                assert_eq!(got, expect, "{}", String::from_utf8_lossy(&got));
                far.write_all(&answer).await.unwrap();
            }
            far.write_all(b"SSH-2.0-test\r\n").await.unwrap();
            far
        });
        let (result, mut near) = client(near).await;
        let _far = proxy.await.unwrap();
        let mut rest = Vec::new();
        if result.is_ok() {
            let mut banner = [0u8; 14];
            near.read_exact(&mut banner).await.unwrap();
            rest.extend_from_slice(&banner);
        }
        (result, rest)
    }

    #[tokio::test]
    async fn http_connect_leaves_the_banner_unread() {
        let (result, rest) = scripted(
            &[(
                b"CONNECT [::1]:22 HTTP/1.1\r\nHost: [::1]:22\r\n\r\n",
                b"HTTP/1.1 200 Connection established\r\nVia: test\r\n\r\n",
            )],
            |mut stream| async move { (http_connect(&mut stream, "::1", 22).await, stream) },
        )
        .await;
        result.unwrap();
        assert_eq!(rest, b"SSH-2.0-test\r\n");

        let (result, _) = scripted(
            &[(
                b"CONNECT h:22 HTTP/1.1\r\nHost: h:22\r\n\r\n",
                b"HTTP/1.1 403 Forbidden\r\n\r\n",
            )],
            |mut stream| async move { (http_connect(&mut stream, "h", 22).await, stream) },
        )
        .await;
        assert!(matches!(result, Err(SshError::Proxy(message)) if message.contains("403")));
    }

    #[tokio::test]
    async fn socks5_connect_sends_the_name_and_skips_the_bound_address() {
        let mut request = vec![5, 1, 0, 3, 8];
        request.extend_from_slice(b"internal");
        request.extend_from_slice(&22u16.to_be_bytes());
        // Success reply bound to 10.0.0.1:1080.
        let reply = [5, 0, 0, 1, 10, 0, 0, 1, 4, 56];
        let (result, rest) = scripted(
            &[(&[5, 1, 0], &[5, 0]), (&request, &reply)],
            |mut stream| async move { (socks5_connect(&mut stream, "internal", 22).await, stream) },
        )
        .await;
        result.unwrap();
        assert_eq!(rest, b"SSH-2.0-test\r\n");

        let mut request = vec![5, 1, 0, 3, 1, b'x'];
        request.extend_from_slice(&22u16.to_be_bytes());
        let (result, _) = scripted(
            &[(&[5, 1, 0], &[5, 0]), (&request, &[5, 5, 0, 1])],
            |mut stream| async move { (socks5_connect(&mut stream, "x", 22).await, stream) },
        )
        .await;
        assert!(matches!(result, Err(SshError::Proxy(message)) if message.contains("refused")));

        let (result, _) = scripted(&[(&[5, 1, 0], &[5, 0xff])], |mut stream| async move {
            (socks5_connect(&mut stream, "x", 22).await, stream)
        })
        .await;
        assert!(
            matches!(result, Err(SshError::Proxy(message)) if message.contains("authentication"))
        );
    }
}
