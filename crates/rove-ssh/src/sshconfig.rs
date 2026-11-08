//! Parsing of OpenSSH client configuration and `user@host:port` quick-connect strings.

use std::path::Path;

use rove_vault::{AuthMethod, HostDraft, Vault, VaultError};
use serde::Serialize;

/// A parsed quick-connect string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuickTarget {
    /// User, when given.
    pub username: Option<String>,
    /// Host name or address, without brackets.
    pub hostname: String,
    /// Port, when given.
    pub port: Option<u16>,
}

/// Parses `host`, `user@host`, `host:port`, `user@host:port`, `[v6]:port`, `ssh://user@host:port`
/// and `ssh [-p port] [-l user] user@host`.
pub fn parse_quick_connect(input: &str) -> Result<QuickTarget, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty address".into());
    }
    if let Some(rest) = input.strip_prefix("ssh://") {
        return parse_authority(rest.trim_end_matches('/'));
    }
    if let Some(rest) = input.strip_prefix("ssh ") {
        return parse_ssh_command(rest);
    }
    parse_authority(input)
}

fn parse_ssh_command(args: &str) -> Result<QuickTarget, String> {
    let mut words = args.split_whitespace();
    let mut port = None;
    let mut user = None;
    let mut destination = None;
    while let Some(word) = words.next() {
        match word {
            "-p" => port = Some(parse_port(words.next().ok_or("-p needs a port")?)?),
            "-l" => user = Some(words.next().ok_or("-l needs a user")?.to_owned()),
            flag if flag.starts_with('-') => return Err(format!("unsupported option {flag}")),
            target if destination.is_none() => destination = Some(target),
            _ => return Err("only one destination is supported".into()),
        }
    }
    let mut target = parse_authority(destination.ok_or("missing destination")?)?;
    target.port = port.or(target.port);
    target.username = target.username.or(user);
    Ok(target)
}

fn parse_authority(input: &str) -> Result<QuickTarget, String> {
    let (username, host_port) = match input.rsplit_once('@') {
        Some((user, rest)) if !user.is_empty() => (Some(user.to_owned()), rest),
        Some(_) => return Err("empty user name".into()),
        None => (None, input),
    };
    let (hostname, port) = if let Some(rest) = host_port.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or("unclosed '[' in address")?;
        let port = match after {
            "" => None,
            _ => Some(parse_port(
                after.strip_prefix(':').ok_or("expected ':' after ']'")?,
            )?),
        };
        (host.to_owned(), port)
    } else if host_port.matches(':').count() > 1 {
        // Bare IPv6 literal without a port.
        (host_port.to_owned(), None)
    } else if let Some((host, port)) = host_port.split_once(':') {
        (host.to_owned(), Some(parse_port(port)?))
    } else {
        (host_port.to_owned(), None)
    };
    if hostname.is_empty() || hostname.chars().any(char::is_whitespace) {
        return Err("invalid host name".into());
    }
    if username
        .as_ref()
        .is_some_and(|user| user.chars().any(char::is_whitespace))
    {
        return Err("invalid user name".into());
    }
    Ok(QuickTarget {
        username,
        hostname,
        port,
    })
}

fn parse_port(text: &str) -> Result<u16, String> {
    match text.parse::<u16>() {
        Ok(0) | Err(_) => Err(format!("invalid port `{text}`")),
        Ok(port) => Ok(port),
    }
}

/// One concrete host alias resolved from an OpenSSH config file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SshConfigHost {
    /// The `Host` alias.
    pub alias: String,
    /// `HostName`, or the alias when absent.
    pub hostname: String,
    /// `Port`, when set.
    pub port: Option<u16>,
    /// `User`, when set.
    pub user: Option<String>,
    /// First `IdentityFile`, unexpanded.
    pub identity_file: Option<String>,
    /// `ProxyJump`, when set.
    pub proxy_jump: Option<String>,
}

struct Block {
    patterns: Vec<String>,
    settings: Vec<(String, String)>,
}

/// Parses OpenSSH client configuration text.
///
/// Every alias on a `Host` line without wildcards becomes one entry. Settings follow OpenSSH's
/// first-match rule across every block whose patterns match the alias, including `Host *`.
/// `Match` blocks and `Include` directives are ignored.
pub fn parse_ssh_config(text: &str) -> Vec<SshConfigHost> {
    let mut blocks = vec![Block {
        patterns: vec!["*".into()],
        settings: Vec::new(),
    }];
    let mut in_match = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = split_directive(line);
        let key = key.to_ascii_lowercase();
        match key.as_str() {
            "host" => {
                in_match = false;
                blocks.push(Block {
                    patterns: value.split_whitespace().map(str::to_owned).collect(),
                    settings: Vec::new(),
                });
            }
            "match" => in_match = true,
            _ if in_match => {}
            _ => {
                if let Some(block) = blocks.last_mut() {
                    block.settings.push((key, unquote(value).to_owned()));
                }
            }
        }
    }

    let mut aliases: Vec<String> = Vec::new();
    for block in &blocks[1..] {
        for pattern in &block.patterns {
            let concrete = !pattern.contains(['*', '?', '!']);
            if concrete && !aliases.contains(pattern) {
                aliases.push(pattern.clone());
            }
        }
    }

    aliases
        .into_iter()
        .map(|alias| {
            let get = |name: &str| {
                blocks
                    .iter()
                    .filter(|block| block_matches(&block.patterns, &alias))
                    .flat_map(|block| block.settings.iter())
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
            };
            SshConfigHost {
                hostname: get("hostname").unwrap_or_else(|| alias.clone()),
                port: get("port").and_then(|port| parse_port(&port).ok()),
                user: get("user"),
                identity_file: get("identityfile"),
                proxy_jump: get("proxyjump").filter(|jump| !jump.eq_ignore_ascii_case("none")),
                alias,
            }
        })
        .collect()
}

fn split_directive(line: &str) -> (&str, &str) {
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let (key, rest) = line.split_at(end);
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
    (key, rest)
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

fn block_matches(patterns: &[String], alias: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob(negated, alias) {
                return false;
            }
        } else if glob(pattern, alias) {
            matched = true;
        }
    }
    matched
}

fn glob(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p].eq_ignore_ascii_case(&text[t])) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Outcome of [`import_ssh_config`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    /// Aliases saved as new hosts.
    pub created: Vec<String>,
    /// Aliases skipped because a host with the same user, host name and port exists.
    pub skipped: Vec<String>,
}

/// Saves the hosts of an OpenSSH config file into the vault.
///
/// `IdentityFile` becomes [`AuthMethod::KeyFile`] with `~` expanded against `home`; otherwise the
/// host uses the SSH agent. A missing `User` falls back to `default_user`.
pub fn import_ssh_config(
    vault: &Vault,
    text: &str,
    home: &Path,
    default_user: &str,
) -> Result<ImportReport, VaultError> {
    let existing: Vec<(String, u16, String)> = vault
        .list_hosts()?
        .into_iter()
        .map(|host| (host.hostname.to_ascii_lowercase(), host.port, host.username))
        .collect();
    let mut report = ImportReport::default();
    for entry in parse_ssh_config(text) {
        let username = entry
            .user
            .clone()
            .unwrap_or_else(|| default_user.to_owned());
        let port = entry.port.unwrap_or(22);
        if existing.contains(&(entry.hostname.to_ascii_lowercase(), port, username.clone())) {
            report.skipped.push(entry.alias);
            continue;
        }
        let auth = match &entry.identity_file {
            Some(path) => AuthMethod::KeyFile {
                path: expand_home(path, home),
            },
            None => AuthMethod::Agent,
        };
        vault.create_host(HostDraft {
            name: entry.alias.clone(),
            group: None,
            hostname: entry.hostname,
            port,
            username,
            auth,
            tags: vec!["ssh-config".into()],
        })?;
        report.created.push(entry.alias);
    }
    Ok(report)
}

fn expand_home(path: &str, home: &Path) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None if path == "~" => home.to_string_lossy().into_owned(),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick(user: Option<&str>, host: &str, port: Option<u16>) -> QuickTarget {
        QuickTarget {
            username: user.map(str::to_owned),
            hostname: host.into(),
            port,
        }
    }

    #[test]
    fn quick_connect_forms() {
        assert_eq!(
            parse_quick_connect("example.com").unwrap(),
            quick(None, "example.com", None)
        );
        assert_eq!(
            parse_quick_connect(" root@10.0.0.1:2222 ").unwrap(),
            quick(Some("root"), "10.0.0.1", Some(2222))
        );
        assert_eq!(
            parse_quick_connect("host:22").unwrap(),
            quick(None, "host", Some(22))
        );
        assert_eq!(
            parse_quick_connect("u@[::1]:2200").unwrap(),
            quick(Some("u"), "::1", Some(2200))
        );
        assert_eq!(
            parse_quick_connect("fe80::1").unwrap(),
            quick(None, "fe80::1", None)
        );
        assert_eq!(
            parse_quick_connect("ssh://a@b:3/").unwrap(),
            quick(Some("a"), "b", Some(3))
        );
        assert_eq!(
            parse_quick_connect("ssh -p 2022 deploy@box").unwrap(),
            quick(Some("deploy"), "box", Some(2022))
        );
        assert_eq!(
            parse_quick_connect("ssh -l me box").unwrap(),
            quick(Some("me"), "box", None)
        );
    }

    #[test]
    fn quick_connect_rejects_garbage() {
        for bad in [
            "",
            "@host",
            "host:0",
            "host:99999",
            "[::1",
            "a b",
            "ssh -i key host",
        ] {
            assert!(parse_quick_connect(bad).is_err(), "{bad}");
        }
    }

    const CONFIG: &str = r#"
# comment
Host web prod-*
    HostName 10.0.0.5
    User deploy

Host db
    HostName=db.internal
    Port 2201
    IdentityFile "~/.ssh/db key"
    ProxyJump bastion

Host bastion web
    User jump
    Port 2222

Match host foo
    User ignored

Host *
    User fallback
    IdentityFile ~/.ssh/id_ed25519
"#;

    #[test]
    fn ssh_config_first_match_wins() {
        let hosts = parse_ssh_config(CONFIG);
        let names: Vec<_> = hosts.iter().map(|host| host.alias.as_str()).collect();
        assert_eq!(names, ["web", "db", "bastion"]);

        let web = &hosts[0];
        assert_eq!(
            (web.hostname.as_str(), web.user.as_deref(), web.port),
            ("10.0.0.5", Some("deploy"), Some(2222))
        );
        assert_eq!(web.identity_file.as_deref(), Some("~/.ssh/id_ed25519"));

        let db = &hosts[1];
        assert_eq!(
            (db.hostname.as_str(), db.port, db.user.as_deref()),
            ("db.internal", Some(2201), Some("fallback"))
        );
        assert_eq!(db.identity_file.as_deref(), Some("~/.ssh/db key"));
        assert_eq!(db.proxy_jump.as_deref(), Some("bastion"));

        let bastion = &hosts[2];
        assert_eq!(
            (bastion.hostname.as_str(), bastion.user.as_deref()),
            ("bastion", Some("jump"))
        );
    }

    #[test]
    fn glob_matching() {
        assert!(glob("prod-*", "prod-eu"));
        assert!(glob("db?", "db1"));
        assert!(!glob("db?", "db12"));
        assert!(block_matches(&["*".into(), "!secret".into()], "web"));
        assert!(!block_matches(&["*".into(), "!secret".into()], "secret"));
    }
}
