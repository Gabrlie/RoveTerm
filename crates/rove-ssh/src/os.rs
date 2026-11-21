//! Detects a server's operating system for display: one `exec` reads `/etc/os-release`, with
//! `uname -s` as the fallback for systems without it (macOS, the BSDs).

use std::sync::Arc;

use rove_vault::HostOs;

use crate::{SshConnection, SshError};

/// Script run on the server; only reads.
const SCRIPT: &str = "cat /etc/os-release 2>/dev/null; echo ---; uname -s 2>/dev/null; echo ---; \
uname -r 2>/dev/null; \
echo '==PVE=='; pveversion 2>/dev/null; \
echo '==FNOS=='; [ -d /usr/trim ] && echo trim; \
cat /etc/fn-os-release /etc/trim_version /usr/trim/etc/version 2>/dev/null; true";

/// Largest accepted output of [`SCRIPT`].
const MAX_OUTPUT: usize = 16 * 1024;

/// Detects the operating system behind `connection`. `None` when the output names nothing
/// recognisable (for example a Windows server without `cat` and `uname`).
pub async fn probe(connection: &Arc<SshConnection>) -> Result<Option<HostOs>, SshError> {
    let output = connection.exec(SCRIPT, MAX_OUTPUT).await?;
    Ok(parse(&String::from_utf8_lossy(&output.stdout)))
}

/// Lowercase identifier made of the characters the vault accepts, at most 32 long.
fn slug(value: &str) -> Option<String> {
    let slug: String = value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        .take(32)
        .collect();
    (!slug.is_empty()).then_some(slug)
}

/// The value of an os-release assignment, without surrounding quotes.
fn unquote(value: &str) -> &str {
    let value = value.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

fn display_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect()
}

fn fnos_name(value: &str) -> bool {
    value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("fnOS"))
        || value.starts_with("飞牛")
}

fn release_field<'a>(release: &'a str, key: &str) -> Option<&'a str> {
    release.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        let value = unquote(value).trim();
        (name.trim() == key && !value.is_empty()).then_some(value)
    })
}

fn with_version(name: &str, version: Option<&str>) -> String {
    match version {
        Some(version) => display_name(&format!("{name} {version}")),
        None => name.to_owned(),
    }
}

/// Reads the shared Proxmox and fnOS probe section used by OS detection and status sampling.
pub(crate) fn specialized_os(
    kernel: &str,
    extra: &str,
    release_id: Option<&str>,
    pretty_name: &str,
) -> Option<(&'static str, String)> {
    let (pve, fnos) = extra.split_once("==FNOS==").unwrap_or((extra, ""));
    let pve_version = pve
        .split_whitespace()
        .filter_map(|token| token.strip_prefix("pve-manager/"))
        .filter_map(|rest| rest.split('/').next())
        .find(|version| !version.is_empty());
    if pve_version.is_some() || pve.contains("pve-manager/") || kernel.contains("-pve") {
        return Some(("proxmox", with_version("Proxmox VE", pve_version)));
    }

    let fnos_pretty = release_field(fnos, "PRETTY_NAME")
        .filter(|value| fnos_name(value))
        .or_else(|| fnos_name(pretty_name).then_some(pretty_name));
    if matches!(release_id, Some("fnos" | "trim"))
        || fnos_pretty.is_some()
        || !fnos.trim().is_empty()
        || kernel.contains("-trim")
    {
        let name = fnos_pretty.map(display_name).unwrap_or_else(|| {
            let version = release_field(fnos, "VERSION_ID")
                .or_else(|| release_field(fnos, "VERSION"))
                .or_else(|| {
                    fnos.lines().map(str::trim).map(unquote).find(|line| {
                        line.starts_with(|c: char| c.is_ascii_digit())
                            && line.contains('.')
                            && !line.contains('=')
                    })
                });
            with_version("fnOS", version)
        });
        return Some(("fnos", name));
    }
    None
}

/// Keeps the original distribution and ancestry within the vault's eight-entry limit.
fn ensure_like_contains(
    required: &str,
    like: Vec<String>,
    id: Option<String>,
    detected: &str,
) -> Vec<String> {
    let mut ancestors = Vec::new();
    for ancestor in id.into_iter().chain(like) {
        if ancestor != detected && !ancestors.contains(&ancestor) {
            ancestors.push(ancestor);
        }
    }
    if !ancestors
        .iter()
        .take(8)
        .any(|ancestor| ancestor == required)
    {
        ancestors.truncate(7);
        ancestors.push(required.to_owned());
    }
    ancestors.truncate(8);
    ancestors
}

/// Parses the output of [`SCRIPT`].
fn parse(text: &str) -> Option<HostOs> {
    let mut sections = text.splitn(3, "---");
    let release = sections.next().unwrap_or_default();
    let uname = sections.next().unwrap_or_default();
    let extra = sections.next().unwrap_or_default();
    let (kernel, extra) = extra.trim_start().split_once('\n').unwrap_or((extra, ""));
    let mut id = None;
    let mut like = Vec::new();
    let mut name = String::new();
    for line in release.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value);
        match key.trim() {
            "ID" => id = slug(value),
            "ID_LIKE" => like = value.split_whitespace().filter_map(slug).take(8).collect(),
            "PRETTY_NAME" => name = value.chars().take(100).collect(),
            _ => {}
        }
    }
    if let Some((detected, mut name)) = specialized_os(kernel, extra, id.as_deref(), &name) {
        if detected == "fnos" && name == "fnOS" && matches!(id.as_deref(), Some("fnos" | "trim")) {
            let version =
                release_field(release, "VERSION_ID").or_else(|| release_field(release, "VERSION"));
            name = with_version("fnOS", version);
        }
        return Some(HostOs {
            id: detected.into(),
            like: ensure_like_contains("debian", like, id, detected),
            name,
        });
    }
    if let Some(id) = id {
        return Some(HostOs { id, like, name });
    }
    let (id, name) = match uname.trim() {
        "Darwin" => ("macos", "macOS"),
        "FreeBSD" => ("freebsd", "FreeBSD"),
        "OpenBSD" => ("openbsd", "OpenBSD"),
        "NetBSD" => ("netbsd", "NetBSD"),
        "Linux" => ("linux", "Linux"),
        _ => return None,
    };
    Some(HostOs {
        id: id.into(),
        like: Vec::new(),
        name: name.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_os_release() {
        let os = parse(
            "PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"\nNAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n---\nLinux\n",
        )
        .unwrap();
        assert_eq!(os.id, "ubuntu");
        assert_eq!(os.like, ["debian"]);
        assert_eq!(os.name, "Ubuntu 24.04.4 LTS");

        let rocky = parse("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n---\nLinux\n").unwrap();
        assert_eq!(rocky.id, "rocky");
        assert_eq!(rocky.like, ["rhel", "centos", "fedora"]);
        assert_eq!(rocky.name, "");
    }

    #[test]
    fn falls_back_to_uname_and_cleans_odd_values() {
        assert_eq!(parse("---\nDarwin\n").unwrap().id, "macos");
        assert_eq!(parse("---\nFreeBSD\n").unwrap().id, "freebsd");
        assert_eq!(parse("'cat' is not recognized\n---\n"), None);
        assert_eq!(parse(""), None);
        let odd = parse("ID='Open SUSE!'\n---\n").unwrap();
        assert_eq!(odd.id, "opensuse");
    }
    #[test]
    fn detects_proxmox_from_manager_or_kernel() {
        for (kernel, manager, expected) in [
            (
                "6.8.0-generic",
                "pve-manager/8.2.4/abc (running kernel: 6.8.0)",
                "Proxmox VE 8.2.4",
            ),
            ("6.8.12-4-pve", "", "Proxmox VE"),
        ] {
            let text = format!(
                "ID=debian\nPRETTY_NAME=Debian\n---\nLinux\n---\n{kernel}\n==PVE==\n{manager}\n==FNOS==\n"
            );
            let os = parse(&text).unwrap();
            assert_eq!(os.id, "proxmox");
            assert_eq!(os.name, expected);
            assert_eq!(os.like, ["debian"]);
        }
    }

    #[test]
    fn detects_fnos_from_release_id_marker_version_files_or_kernel() {
        for (id, kernel, extra, expected) in [
            ("debian", "6.8.0", "trim\n", "fnOS"),
            ("debian", "6.8.0", "0.8.41\n", "fnOS 0.8.41"),
            ("debian", "6.8.0-trim", "", "fnOS"),
            (
                "debian",
                "6.8.0",
                "PRETTY_NAME='飞牛 OS 1.0'\n",
                "飞牛 OS 1.0",
            ),
            ("trim", "6.8.0", "VERSION='1.0'\n", "fnOS 1.0"),
            ("fnos", "6.8.0", "VERSION_ID='1.1'\n", "fnOS 1.1"),
        ] {
            let text = format!(
                "ID={id}\nID_LIKE=linux\n---\nLinux\n---\n{kernel}\n==PVE==\n==FNOS==\n{extra}"
            );
            let os = parse(&text).unwrap();
            assert_eq!(os.id, "fnos");
            assert_eq!(os.name, expected);
            assert!(os.like.iter().any(|ancestor| ancestor == "debian"));
            assert_eq!(
                os.like
                    .iter()
                    .filter(|ancestor| *ancestor == "debian")
                    .count(),
                1
            );
        }
        let legacy = parse("ID=fnos\nVERSION_ID=0.8.41\n---\nLinux\n").unwrap();
        assert_eq!(legacy.name, "fnOS 0.8.41");
    }

    #[test]
    fn specialized_names_and_ancestry_fit_vault_limits() {
        let text = format!(
            "ID=debian\nID_LIKE='a b c d e f g h'\n---\nLinux\n---\n6.8-trim\n==FNOS==\nVERSION='1.0\u{7}{}'",
            "x".repeat(110)
        );
        let os = parse(&text).unwrap();
        assert_eq!(os.like.len(), 8);
        assert_eq!(os.like[0], "debian");
        assert_eq!(os.name.chars().count(), 100);
        assert!(!os.name.chars().any(char::is_control));
        assert_eq!(os.clone().validate().unwrap(), os);
    }
}
