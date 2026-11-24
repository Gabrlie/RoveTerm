use serde::{Deserialize, Serialize};

#[cfg(any(mobile, test))]
use {crate::MobileError, url::Url, zeroize::Zeroizing};

#[derive(Default, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileInsets {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
    pub ime_bottom: f64,
}

impl MobileInsets {
    pub fn sanitized(self) -> Self {
        fn nonnegative(value: f64) -> f64 {
            if value.is_finite() && value > 0.0 {
                value
            } else {
                0.0
            }
        }

        Self {
            top: nonnegative(self.top),
            right: nonnegative(self.right),
            bottom: nonnegative(self.bottom),
            left: nonnegative(self.left),
            ime_bottom: nonnegative(self.ime_bottom),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiometricStatus {
    pub supported: bool,
    pub enrolled: bool,
}

#[cfg(any(mobile, test))]
pub(crate) const MASTER_KEY_LEN: usize = 32;

#[cfg(any(mobile, test))]
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[cfg(any(mobile, test))]
pub(crate) fn encode_base64(bytes: &[u8]) -> Zeroizing<String> {
    let mut encoded = Zeroizing::new(String::with_capacity(bytes.len().div_ceil(3) * 4));

    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or_default();
        let third = chunk.get(2).copied().unwrap_or_default();
        encoded.push(BASE64[(first >> 2) as usize].into());
        encoded.push(BASE64[(((first & 3) << 4) | (second >> 4)) as usize].into());
        encoded.push(if chunk.len() >= 2 {
            BASE64[(((second & 15) << 2) | (third >> 6)) as usize].into()
        } else {
            '='
        });
        encoded.push(if chunk.len() == 3 {
            BASE64[(third & 63) as usize].into()
        } else {
            '='
        });
    }

    encoded
}

#[cfg(any(mobile, test))]
fn decode_base64(encoded: &str) -> Option<Zeroizing<Vec<u8>>> {
    fn digit(byte: u8) -> Option<u8> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }

    if !encoded.len().is_multiple_of(4) {
        return None;
    }

    let mut decoded = Zeroizing::new(Vec::with_capacity(encoded.len() / 4 * 3));
    let mut chunks = encoded.as_bytes().chunks_exact(4).peekable();
    while let Some(chunk) = chunks.next() {
        let first = digit(chunk[0])?;
        let second = digit(chunk[1])?;
        decoded.push((first << 2) | (second >> 4));

        match (chunk[2], chunk[3]) {
            (b'=', b'=') if chunks.peek().is_none() && second & 15 == 0 => break,
            (third, b'=') if chunks.peek().is_none() => {
                let third = digit(third)?;
                if third & 3 != 0 {
                    return None;
                }
                decoded.push((second << 4) | (third >> 2));
            }
            (third, fourth) => {
                let third = digit(third)?;
                decoded.push((second << 4) | (third >> 2));
                decoded.push((third << 6) | digit(fourth)?);
            }
        }
    }

    Some(decoded)
}

#[cfg(any(mobile, test))]
pub(crate) fn decode_master_key(encoded: &str) -> Result<Zeroizing<Vec<u8>>, MobileError> {
    if encoded.len() != MASTER_KEY_LEN.div_ceil(3) * 4 {
        return Err(MobileError::Malformed(
            "invalid encoded master key length".into(),
        ));
    }
    let decoded = decode_base64(encoded)
        .ok_or_else(|| MobileError::Malformed("invalid master key encoding".into()))?;
    if decoded.len() != MASTER_KEY_LEN {
        return Err(MobileError::Malformed("invalid master key length".into()));
    }

    Ok(decoded)
}

#[cfg(any(mobile, test))]
pub(crate) fn parse_external_url(input: &str) -> Result<Url, MobileError> {
    let url =
        Url::parse(input).map_err(|_| MobileError::InvalidInput("invalid external URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") || !url.has_host() {
        return Err(MobileError::InvalidInput(
            "external URL requires HTTP or HTTPS and a host".into(),
        ));
    }

    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_rfc_vectors_and_all_byte_values() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode_base64(plain.as_bytes()).as_str(), encoded);
            assert_eq!(decode_base64(encoded).unwrap().as_slice(), plain.as_bytes());
        }
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(
            decode_base64(&encode_base64(&bytes)).unwrap().as_slice(),
            bytes
        );
        assert_eq!(encode_base64(&[251, 255, 255]).as_str(), "+///");
    }

    #[test]
    fn base64_rejects_malformed_and_noncanonical_input() {
        for encoded in [
            "Zg", "Zg===", "Zg=_", "Zg-/", "=g==", "Z===", "Zg==Zm8=", "Zm8=Zg==", "Zh==", "Zm9=",
            "Zg==\n", "☃=", "Zg =",
        ] {
            assert!(decode_base64(encoded).is_none(), "{encoded:?}");
        }
    }

    #[test]
    fn master_key_requires_exactly_32_decoded_bytes() {
        let key = [0; MASTER_KEY_LEN];
        assert_eq!(
            decode_master_key(&encode_base64(&key)).unwrap().as_slice(),
            key
        );

        for length in [0, 30, 31, 33, 34] {
            let encoded = encode_base64(&vec![0; length]);
            assert!(matches!(
                decode_master_key(&encoded),
                Err(MobileError::Malformed(_))
            ));
        }
        assert!(matches!(
            decode_master_key(&"!".repeat(44)),
            Err(MobileError::Malformed(_))
        ));
    }

    #[test]
    fn external_urls_require_an_http_scheme_and_host() {
        for url in [
            "http://localhost:8080/",
            "HTTPS://example.org/path",
            "https://[::1]/",
        ] {
            assert!(parse_external_url(url).is_ok(), "{url}");
        }
        for url in [
            "https://",
            "http:",
            "/relative",
            "file:///tmp/x",
            "mailto:me@example.org",
            "javascript:alert(1)",
            "intent://example.org",
            "ftp://example.org",
        ] {
            assert!(matches!(
                parse_external_url(url),
                Err(MobileError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn insets_sanitize_each_component_independently() {
        let sanitized = MobileInsets {
            top: f64::NAN,
            right: f64::INFINITY,
            bottom: -12.5,
            left: f64::NEG_INFINITY,
            ime_bottom: 37.5,
        }
        .sanitized();
        assert_eq!(
            sanitized,
            MobileInsets {
                ime_bottom: 37.5,
                ..MobileInsets::default()
            }
        );
        assert_eq!(
            serde_json::to_value(sanitized).unwrap(),
            serde_json::json!({ "top": 0.0, "right": 0.0, "bottom": 0.0, "left": 0.0, "imeBottom": 37.5 })
        );
    }
}
