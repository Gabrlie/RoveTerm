//! The SOCKS5 server side needed by dynamic forwarding: no authentication, `CONNECT` only
//! (RFC 1928).

use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const VERSION: u8 = 5;
const NO_AUTH: u8 = 0;
const NO_ACCEPTABLE_METHOD: u8 = 0xff;
const CONNECT: u8 = 1;

/// Reply codes sent after the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reply {
    Succeeded = 0,
    HostUnreachable = 4,
    CommandNotSupported = 7,
    AddressTypeNotSupported = 8,
}

/// Why a client was refused before a destination was known.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SocksError {
    #[error("not a SOCKS5 client")]
    Version,
    #[error("client offers no supported authentication method")]
    NoMethod,
    #[error("unsupported command {0}")]
    Command(u8),
    #[error("unsupported address type {0}")]
    AddressType(u8),
    #[error("invalid domain name")]
    Domain,
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

/// Runs the method negotiation and reads the `CONNECT` request. Returns the destination host and
/// port; the caller must then send [`reply`]. Failures that the protocol can report are answered
/// before returning the error.
pub(crate) async fn accept<S>(stream: &mut S) -> Result<(String, u16), SocksError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await?;
    if header[0] != VERSION {
        return Err(SocksError::Version);
    }
    let mut methods = vec![0u8; usize::from(header[1])];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&NO_AUTH) {
        stream.write_all(&[VERSION, NO_ACCEPTABLE_METHOD]).await?;
        return Err(SocksError::NoMethod);
    }
    stream.write_all(&[VERSION, NO_AUTH]).await?;

    let mut request = [0u8; 4];
    stream.read_exact(&mut request).await?;
    if request[0] != VERSION {
        return Err(SocksError::Version);
    }
    let host = match request[3] {
        1 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).await?;
            Ipv4Addr::from(octets).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            stream.read_exact(&mut name).await?;
            match String::from_utf8(name) {
                Ok(name) if !name.is_empty() => name,
                _ => {
                    reply(stream, Reply::HostUnreachable).await?;
                    return Err(SocksError::Domain);
                }
            }
        }
        4 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).await?;
            Ipv6Addr::from(octets).to_string()
        }
        other => {
            reply(stream, Reply::AddressTypeNotSupported).await?;
            return Err(SocksError::AddressType(other));
        }
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    if request[1] != CONNECT {
        reply(stream, Reply::CommandNotSupported).await?;
        return Err(SocksError::Command(request[1]));
    }
    Ok((host, u16::from_be_bytes(port)))
}

/// Sends the request reply with an unspecified bound address.
pub(crate) async fn reply<S>(stream: &mut S, code: Reply) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&[VERSION, code as u8, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(client_bytes: &[u8]) -> (Result<(String, u16), SocksError>, Vec<u8>) {
        let (mut client, mut server) = tokio::io::duplex(256);
        client.write_all(client_bytes).await.unwrap();
        let result = accept(&mut server).await;
        drop(server);
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).await.unwrap();
        (result, answer)
    }

    #[tokio::test]
    async fn parses_domain_ipv4_and_ipv6_requests() {
        let mut domain = vec![5, 1, 0, 5, 1, 0, 3, 11];
        domain.extend_from_slice(b"example.com");
        domain.extend_from_slice(&443u16.to_be_bytes());
        let (result, answer) = run(&domain).await;
        assert_eq!(result.unwrap(), ("example.com".into(), 443));
        assert_eq!(answer, [5, 0]);

        let (result, _) = run(&[5, 1, 0, 5, 1, 0, 1, 10, 0, 0, 7, 0, 80]).await;
        assert_eq!(result.unwrap(), ("10.0.0.7".into(), 80));

        let mut v6 = vec![5, 1, 0, 5, 1, 0, 4];
        v6.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        v6.extend_from_slice(&22u16.to_be_bytes());
        let (result, _) = run(&v6).await;
        assert_eq!(result.unwrap(), ("::1".into(), 22));
    }

    #[tokio::test]
    async fn refuses_auth_only_clients_and_other_commands() {
        let (result, answer) = run(&[5, 1, 2]).await;
        assert!(matches!(result, Err(SocksError::NoMethod)));
        assert_eq!(answer, [5, 0xff]);

        let (result, answer) = run(&[5, 1, 0, 5, 2, 0, 1, 1, 2, 3, 4, 0, 80]).await;
        assert!(matches!(result, Err(SocksError::Command(2))));
        assert_eq!(answer[2..4], [5, 7]);

        let (result, _) = run(&[4, 1, 0, 80]).await;
        assert!(matches!(result, Err(SocksError::Version)));
    }
}
