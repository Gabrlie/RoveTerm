//! Finding ACP agents on this computer. A built-in table names the agents RoveTerm knows; each is
//! found on the PATH a login shell would have (GUI apps on macOS and Linux start with a minimal
//! one), or else runs through npx when Node.js is installed.

use std::path::PathBuf;

use serde::Serialize;

/// An agent RoveTerm knows how to start.
#[derive(Debug, Clone, Copy)]
pub struct KnownAgent {
    /// Stable id, used in `acp:<id>`.
    pub id: &'static str,
    pub name: &'static str,
    /// Programs that speak ACP, most preferred first.
    pub commands: &'static [&'static str],
    /// Arguments that put the program into ACP mode.
    pub args: &'static [&'static str],
    /// The npm package to run through npx when no program is installed.
    pub package: &'static str,
    /// Environment variables an API key for the agent goes into.
    pub key_vars: &'static [&'static str],
    /// ACP sign-in methods that use that key, most preferred first.
    pub key_methods: &'static [&'static str],
    /// ACP sign-in methods that use the agent's own account.
    pub account_methods: &'static [&'static str],
    /// Interactive CLI login fallback (`(program, args)`) when installed locally.
    pub login_command: Option<(&'static str, &'static [&'static str])>,
}

/// The agents RoveTerm knows, in the order they are offered.
pub const KNOWN: &[KnownAgent] = &[
    KnownAgent {
        id: "codex",
        name: "Codex",
        commands: &["codex-acp"],
        args: &[],
        package: "@agentclientprotocol/codex-acp",
        key_vars: &["CODEX_API_KEY", "OPENAI_API_KEY"],
        key_methods: &["codex-api-key", "openai-api-key"],
        account_methods: &["chatgpt"],
        login_command: Some(("codex", &["login"])),
    },
    KnownAgent {
        id: "claude",
        name: "Claude Code",
        commands: &["claude-agent-acp", "claude-code-acp"],
        args: &[],
        package: "@agentclientprotocol/claude-agent-acp",
        key_vars: &["ANTHROPIC_API_KEY"],
        key_methods: &[],
        account_methods: &[],
        login_command: Some(("claude", &["auth", "login"])),
    },
    KnownAgent {
        id: "gemini",
        name: "Gemini CLI",
        commands: &["gemini"],
        args: &["--experimental-acp"],
        package: "@google/gemini-cli",
        key_vars: &["GEMINI_API_KEY"],
        key_methods: &["gemini-api-key"],
        account_methods: &["oauth-personal"],
        login_command: None,
    },
    KnownAgent {
        id: "qwen",
        name: "Qwen Code",
        commands: &["qwen"],
        args: &["--experimental-acp"],
        package: "@qwen-code/qwen-code",
        key_vars: &["OPENAI_API_KEY"],
        key_methods: &["openai"],
        account_methods: &["qwen-oauth"],
        login_command: None,
    },
];

/// How a known agent can run here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    /// Runs through npx (downloaded on first use) rather than an installed program.
    pub via_npx: bool,
}

/// Finds `name` on `path`, as the shell would (with `PATHEXT` on Windows).
pub fn which(name: &str, path: &str) -> Option<PathBuf> {
    let candidates: Vec<String> = if cfg!(windows) {
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        extensions
            .split(';')
            .map(|extension| format!("{name}{}", extension.to_lowercase()))
            .collect()
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(path).find_map(|dir| {
        candidates
            .iter()
            .map(|candidate| dir.join(candidate))
            .find(|file| file.is_file())
    })
}

/// How each known agent runs on `path`: installed, through npx, or not at all (left out).
pub fn detect(path: &str) -> Vec<Found> {
    let npx = which("npx", path);
    KNOWN
        .iter()
        .filter_map(|agent| {
            if let Some(program) = agent
                .commands
                .iter()
                .find_map(|command| which(command, path))
            {
                return Some(Found {
                    id: agent.id.into(),
                    name: agent.name.into(),
                    program: program.to_string_lossy().into_owned(),
                    args: agent.args.iter().map(|arg| (*arg).to_owned()).collect(),
                    via_npx: false,
                });
            }
            let npx = npx.as_ref()?;
            let mut args = vec!["-y".to_owned(), agent.package.to_owned()];
            args.extend(agent.args.iter().map(|arg| (*arg).to_owned()));
            Some(Found {
                id: agent.id.into(),
                name: agent.name.into(),
                program: npx.to_string_lossy().into_owned(),
                args,
                via_npx: true,
            })
        })
        .collect()
}

/// The table entry of `id`.
pub fn known(id: &str) -> Option<&'static KnownAgent> {
    KNOWN.iter().find(|agent| agent.id == id)
}

/// The PATH a login shell would have, worked out once: the login shell's own, then this
/// process's, then common install directories. Windows keeps this process's PATH.
pub fn search_path() -> String {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(compute_search_path).clone()
}

fn compute_search_path() -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let login = std::process::Command::new(&shell)
            .args(["-ilc", "printf '%s' \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|text| text.lines().last().unwrap_or_default().trim().to_owned())
            .filter(|path| !path.is_empty());
        let home = std::env::home_dir().unwrap_or_default();
        merge(login.as_deref(), &current, &home)
    }
    #[cfg(not(unix))]
    {
        current
    }
}

/// `login`, then `current`, then common install directories under `/` and `home`, without repeats.
#[cfg(any(unix, test))]
fn merge(login: Option<&str>, current: &str, home: &std::path::Path) -> String {
    let mut dirs: Vec<String> = Vec::new();
    let mut add = |dir: String| {
        if !dir.is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    for part in login
        .into_iter()
        .flat_map(|path| path.split(':'))
        .chain(current.split(':'))
    {
        add(part.to_owned());
    }
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        add(extra.to_owned());
    }
    for extra in [
        ".local/bin",
        ".cargo/bin",
        ".npm-global/bin",
        ".bun/bin",
        ".volta/bin",
    ] {
        add(home.join(extra).to_string_lossy().into_owned());
    }
    dirs.join(":")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn tool(dir: &Path, name: &str) {
        let file = dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        });
        std::fs::write(file, b"").unwrap();
    }

    #[test]
    fn finds_installed_agents_and_falls_back_to_npx() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([dir.path()])
            .unwrap()
            .into_string()
            .unwrap();
        assert!(detect(&path).is_empty(), "nothing installed, no npx");
        tool(dir.path(), "claude-code-acp");
        tool(dir.path(), "npx");
        let found = detect(&path);
        let claude = found.iter().find(|agent| agent.id == "claude").unwrap();
        assert!(!claude.via_npx && claude.program.contains("claude-code-acp"));
        let gemini = found.iter().find(|agent| agent.id == "gemini").unwrap();
        assert!(gemini.via_npx);
        assert_eq!(
            gemini.args,
            ["-y", "@google/gemini-cli", "--experimental-acp"]
        );
        assert_eq!(found.len(), KNOWN.len());
    }

    #[test]
    fn merges_search_paths_without_repeats() {
        let merged = merge(Some("/a:/b"), "/b:/c", Path::new("/home/me"));
        assert!(
            merged.starts_with("/a:/b:/c:/opt/homebrew/bin:/usr/local/bin:/home/me/.local/bin")
        );
        assert_eq!(merged.matches("/b:").count(), 1);
    }
}
