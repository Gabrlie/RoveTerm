//! Detects a server's operating system for display: one `exec` reads `/etc/os-release`, with
//! `uname -s` as the fallback for systems without it (macOS, the BSDs).

use std::sync::Arc;

use rove_vault::HostOs;

use crate::{SshConnection, SshError};

/// Script run on the server; only reads.
const SCRIPT: &str = "cat /etc/os-release 2>/dev/null; echo ---; uname -s 2>/dev/null; true";

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

/// Parses the output of [`SCRIPT`].
fn parse(text: &str) -> Option<HostOs> {
    let (release, uname) = text.split_once("---").unwrap_or((text, ""));
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
}
