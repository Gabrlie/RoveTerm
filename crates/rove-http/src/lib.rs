//! One way to build HTTP clients in RoveTerm: reqwest on rustls with the ring provider, which the
//! app ships anyway, and the system's certificates. On Android the system verifier would first
//! have to be initialised over JNI, so the Mozilla roots bundled with `webpki-roots` are used
//! instead; a server whose certificate chains only to a user-installed CA is rejected there.

use std::time::Duration;

pub use reqwest;

/// Errors building a client.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not set up HTTPS: {0}")]
pub struct HttpError(String);

/// Install ring as the process default for rustls if a provider is not already installed.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// A client sending RoveTerm's user agent. `timeout` bounds each whole request; `None` leaves
/// requests unbounded (streams bound themselves).
pub fn client(timeout: Option<Duration>) -> Result<reqwest::Client, HttpError> {
    builder(timeout)?
        .build()
        .map_err(|error| HttpError(error.to_string()))
}

/// The same settings as [`client`], for callers that add their own (such as a redirect policy).
pub fn builder(timeout: Option<Duration>) -> Result<reqwest::ClientBuilder, HttpError> {
    install_crypto_provider();
    let mut builder =
        reqwest::Client::builder().user_agent(concat!("RoveTerm/", env!("CARGO_PKG_VERSION")));
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    #[cfg(target_os = "android")]
    {
        builder = builder.tls_backend_preconfigured(android_tls()?);
    }
    Ok(builder)
}

#[cfg(target_os = "android")]
fn android_tls() -> Result<rustls::ClientConfig, HttpError> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| HttpError(error.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

#[cfg(test)]
mod tests {
    #[test]
    fn installs_crypto_provider_idempotently() {
        super::install_crypto_provider();
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap();
        super::install_crypto_provider();
        assert!(std::sync::Arc::ptr_eq(
            provider,
            rustls::crypto::CryptoProvider::get_default().unwrap()
        ));
    }

    #[test]
    fn builds_clients() {
        assert!(super::client(Some(std::time::Duration::from_secs(5))).is_ok());
        assert!(super::client(None).is_ok());
    }
}
