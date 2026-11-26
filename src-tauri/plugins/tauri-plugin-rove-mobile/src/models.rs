use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::MobileError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApkInstallStatus {
    Launched,
    PermissionRequired,
}

#[cfg(target_os = "android")]
#[derive(Serialize)]
pub(crate) struct InstallApkRequest<'a> {
    pub path: &'a str,
}

#[cfg(target_os = "android")]
#[derive(Deserialize)]
pub(crate) struct InstallApkResponse {
    pub status: ApkInstallStatus,
}

#[cfg(any(mobile, test))]
use {url::Url, zeroize::Zeroizing};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFile {
    pub path: String,
    pub name: String,
    pub size: u64,
}

pub fn parse_stage_uri(uri: &str) -> Result<&str, MobileError> {
    if !uri.contains('\0')
        && let Some((scheme, remainder)) = uri.split_once("://")
        && !remainder.is_empty()
        && (scheme.eq_ignore_ascii_case("content") || scheme.eq_ignore_ascii_case("file"))
    {
        return Ok(uri);
    }
    Err(MobileError::InvalidInput(
        "staging requires a content:// or file:// URI".into(),
    ))
}

pub fn validate_local_file_path(path: &str) -> Result<&str, MobileError> {
    if path.contains('\0') || !Path::new(path).is_absolute() {
        return Err(MobileError::InvalidInput(
            "file path must be absolute and contain no NUL bytes".into(),
        ));
    }
    Ok(path)
}

pub fn sanitize_file_name(raw: &str) -> String {
    const MAX_SCALARS: usize = 200;

    let component = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let name: String = component.chars().filter(|ch| !ch.is_control()).collect();
    let name = name.trim_matches([' ', '.']);
    if name.is_empty() {
        return "file".into();
    }
    if name.chars().count() <= MAX_SCALARS {
        return name.into();
    }

    if let Some(dot) = name.rfind('.') {
        let (stem, extension) = name.split_at(dot);
        let extension_len = extension.chars().count();
        if extension_len < MAX_SCALARS {
            return stem
                .chars()
                .take(MAX_SCALARS - extension_len)
                .chain(extension.chars())
                .collect();
        }
    }
    let prefix: String = name.chars().take(MAX_SCALARS).collect();
    prefix.trim_end_matches([' ', '.']).into()
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepAliveUpdate {
    pub active: bool,
    pub sessions: u32,
    pub forwards: u32,
    pub transfers: u32,
    pub ops_tasks: u32,
    pub title: String,
    pub body: String,
    pub disconnect_label: String,
}

impl KeepAliveUpdate {
    pub fn total(&self) -> u32 {
        self.sessions
            .saturating_add(self.forwards)
            .saturating_add(self.transfers)
            .saturating_add(self.ops_tasks)
    }

    pub fn sanitized(&self) -> Self {
        fn bounded_text(text: &str, max_chars: usize, default: &str) -> String {
            let text = text.trim();
            if text.is_empty() {
                return default.into();
            }
            text.chars().take(max_chars).collect()
        }

        Self {
            active: self.active && self.total() > 0,
            sessions: self.sessions,
            forwards: self.forwards,
            transfers: self.transfers,
            ops_tasks: self.ops_tasks,
            title: bounded_text(&self.title, 80, "RoveTerm"),
            body: bounded_text(&self.body, 160, ""),
            disconnect_label: bounded_text(&self.disconnect_label, 40, "Disconnect all"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Foreground,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub state: LifecycleState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAliveAction {
    DisconnectAll,
}

#[cfg(any(mobile, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum NativeLifecycleEvent {
    Lifecycle { state: LifecycleState },
    Back,
    KeepAliveAction { action: KeepAliveAction },
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
    use serde_json::{from_value, json, to_value};

    #[test]
    fn stage_uris_require_a_supported_scheme_and_nonempty_remainder() {
        for uri in [
            "content://provider/document/123",
            "CoNtEnT://provider/document/a%2Fb",
            "file:///tmp/read%20me.txt",
            "FILE:///tmp/report",
        ] {
            assert_eq!(parse_stage_uri(uri).unwrap(), uri);
        }
        for uri in [
            "",
            " \t\n",
            "content://",
            "file://",
            "file:/tmp/x",
            "https://example.org",
            "content:/provider",
            "/tmp/x",
            " content://provider",
            "file:///tmp/x\0y",
        ] {
            assert!(
                matches!(parse_stage_uri(uri), Err(MobileError::InvalidInput(_))),
                "{uri:?}"
            );
        }
    }

    #[test]
    fn local_file_paths_are_absolute_and_nul_free() {
        #[cfg(windows)]
        let paths = [r"C:\Users\me\report.txt", r"\\server\share\report.txt"];
        #[cfg(not(windows))]
        let paths = ["/tmp/report.txt", "/tmp/报 告.txt", "/"];

        for path in paths {
            assert_eq!(validate_local_file_path(path).unwrap(), path);
        }
        for path in ["", " \t ", "relative.txt", "../report.txt", "file:///tmp/x"] {
            assert!(
                matches!(
                    validate_local_file_path(path),
                    Err(MobileError::InvalidInput(_))
                ),
                "{path:?}"
            );
        }
        assert!(matches!(
            validate_local_file_path(&format!("{}\0", paths[0])),
            Err(MobileError::InvalidInput(_))
        ));
    }

    #[test]
    fn file_names_drop_path_components_and_control_characters() {
        for (raw, expected) in [
            ("/tmp/export/  ..report.txt. ", "report.txt"),
            (r"C:\Users\me\报告.txt", "报告.txt"),
            (r"dir/subdir\a b.tar.gz", "a b.tar.gz"),
            ("\0a\n\u{0085}b\u{007f}.txt", "ab.txt"),
            (".hidden", "hidden"),
            ("", "file"),
            ("/tmp/", "file"),
            ("dir/ . . ", "file"),
            (r"dir\..", "file"),
            ("\t\n\0", "file"),
        ] {
            assert_eq!(sanitize_file_name(raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn file_names_truncate_on_scalar_boundaries_and_keep_a_fitting_extension() {
        let stem = "文🔌".repeat(110);
        assert_eq!(sanitize_file_name(&stem), "文🔌".repeat(100));
        assert_eq!(
            sanitize_file_name(&format!("{stem}.txt")),
            format!("{}.txt", "文🔌".repeat(98))
        );
        let long_extension = format!("a.{}", "文".repeat(201));
        assert_eq!(
            sanitize_file_name(&long_extension),
            format!("a.{}", "文".repeat(198))
        );
        let prefix = "x".repeat(199);
        assert_eq!(sanitize_file_name(&format!("{prefix} y")), prefix);
        let limit = format!("a.{}", "文".repeat(198));
        assert_eq!(sanitize_file_name(&limit), limit);
    }

    #[test]
    fn staged_file_round_trips_the_native_response() {
        let response =
            json!({ "path": "/cache/report.txt", "name": "report.txt", "size": u64::MAX });
        let file: StagedFile = from_value(response.clone()).unwrap();
        assert_eq!(file.path, "/cache/report.txt");
        assert_eq!(file.name, "report.txt");
        assert_eq!(file.size, u64::MAX);
        assert_eq!(to_value(file).unwrap(), response);
    }

    fn keepalive_update() -> KeepAliveUpdate {
        KeepAliveUpdate {
            active: true,
            sessions: 0,
            forwards: 0,
            transfers: 0,
            ops_tasks: 0,
            title: " \t ".into(),
            body: " \n ".into(),
            disconnect_label: String::new(),
        }
    }

    #[test]
    fn keepalive_total_sums_all_counts_and_saturates() {
        let update = KeepAliveUpdate {
            sessions: 1,
            forwards: 2,
            transfers: 4,
            ops_tasks: 8,
            ..keepalive_update()
        };
        assert_eq!(update.total(), 15);
        assert_eq!(
            KeepAliveUpdate {
                ops_tasks: u32::MAX,
                ..update
            }
            .total(),
            u32::MAX
        );
    }

    #[test]
    fn keepalive_requires_both_active_flag_and_work() {
        let mut update = keepalive_update();
        assert!(!update.sanitized().active);
        for counts in [(1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)] {
            (
                update.sessions,
                update.forwards,
                update.transfers,
                update.ops_tasks,
            ) = counts;
            assert!(update.sanitized().active);
        }
        update.active = false;
        assert!(!update.sanitized().active);
    }

    #[test]
    fn keepalive_sanitizes_copy_and_serializes_in_camel_case() {
        let defaults = keepalive_update().sanitized();
        assert_eq!(defaults.title, "RoveTerm");
        assert!(defaults.body.is_empty());
        assert_eq!(defaults.disconnect_label, "Disconnect all");

        let update = KeepAliveUpdate {
            sessions: 1,
            forwards: 2,
            transfers: 3,
            ops_tasks: 4,
            title: format!("  {}\n", "终".repeat(81)),
            body: format!("\t{}  ", "端".repeat(161)),
            disconnect_label: format!(" {} ", "🔌".repeat(41)),
            ..keepalive_update()
        };
        let sanitized = update.sanitized();
        assert_eq!(
            to_value(&sanitized).unwrap(),
            json!({
                "active": true, "sessions": 1, "forwards": 2, "transfers": 3, "opsTasks": 4,
                "title": "终".repeat(80), "body": "端".repeat(160), "disconnectLabel": "🔌".repeat(40),
            })
        );
        assert!(update.title.starts_with("  "));
    }

    #[test]
    fn native_lifecycle_events_match_the_channel_wire_format() {
        for (event, payload) in [
            (
                NativeLifecycleEvent::Lifecycle {
                    state: LifecycleState::Foreground,
                },
                json!({ "kind": "lifecycle", "state": "foreground" }),
            ),
            (
                NativeLifecycleEvent::Lifecycle {
                    state: LifecycleState::Background,
                },
                json!({ "kind": "lifecycle", "state": "background" }),
            ),
            (NativeLifecycleEvent::Back, json!({ "kind": "back" })),
            (
                NativeLifecycleEvent::KeepAliveAction {
                    action: KeepAliveAction::DisconnectAll,
                },
                json!({ "kind": "keep_alive_action", "action": "disconnect_all" }),
            ),
        ] {
            assert_eq!(to_value(event).unwrap(), payload);
            assert_eq!(from_value::<NativeLifecycleEvent>(payload).unwrap(), event);
        }
        assert_eq!(
            to_value(LifecycleEvent {
                state: LifecycleState::Background
            })
            .unwrap(),
            json!({ "state": "background" })
        );
    }

    #[test]
    fn native_lifecycle_events_reject_missing_or_unknown_discriminants() {
        for payload in [
            json!({}),
            json!({ "kind": "unknown" }),
            json!({ "kind": "lifecycle" }),
            json!({ "kind": "lifecycle", "state": "paused" }),
            json!({ "kind": "keep_alive_action" }),
            json!({ "kind": "keep_alive_action", "action": "disconnectAll" }),
        ] {
            assert!(from_value::<NativeLifecycleEvent>(payload).is_err());
        }
    }

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
