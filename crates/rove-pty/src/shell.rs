//! Resolution of a local terminal request into a concrete launch specification.

use std::path::{Path, PathBuf};

use rove_session::TermSize;

use crate::PtyConfig;

/// A request to open a local terminal. Every field is optional; [`LocalRequest::resolve`] fills
/// defaults.
#[derive(Debug, Clone, Default)]
pub struct LocalRequest {
    /// Shell or program to run; absent uses the configured or platform default shell.
    pub shell: Option<String>,
    /// Explicit arguments for `shell`; when `Some`, replaces platform default shell flags.
    pub args: Option<Vec<String>>,
    /// Working directory; absent uses the user's home directory.
    pub cwd: Option<PathBuf>,
    /// Initial size; absent uses 80x24.
    pub size: Option<TermSize>,
    /// Additional environment variables merged after the terminal defaults (later values win).
    pub env: Vec<(String, String)>,
}

/// A fully resolved launch: no field is defaulted later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// Program to execute.
    pub program: String,
    /// Program arguments.
    pub args: Vec<String>,
    /// Working directory, when one could be determined.
    pub cwd: Option<PathBuf>,
    /// Initial size.
    pub size: TermSize,
    /// Extra environment variables set for the shell.
    pub env: Vec<(String, String)>,
}

impl LocalRequest {
    /// Resolves defaults: request values first, then `config`, then the platform default.
    pub fn resolve(self, config: &PtyConfig) -> LaunchSpec {
        let (program, args) = match (self.shell, self.args, &config.shell) {
            (Some(program), Some(args), _) => (program, args),
            (Some(shell), None, _) => {
                let args = default_args(&shell);
                (shell, args)
            }
            (None, _, Some(shell)) => (shell.clone(), config.args.clone()),
            (None, _, None) => {
                let shell = default_shell();
                let args = default_args(&shell);
                (shell, args)
            }
        };
        let mut env = vec![
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
            ("TERM_PROGRAM".into(), "RoveTerm".into()),
        ];
        for (key, value) in self.env {
            if let Some(existing) = env.iter_mut().find(|(name, _)| *name == key) {
                existing.1 = value;
            } else {
                env.push((key, value));
            }
        }
        LaunchSpec {
            program,
            args,
            cwd: self.cwd.or_else(std::env::home_dir),
            size: self.size.unwrap_or_default(),
            env,
        }
    }
}

/// Returns the platform default shell.
///
/// Unix: `$SHELL`, else `/bin/sh`. Windows: `pwsh.exe` or `powershell.exe` when found on `PATH`,
/// else `%COMSPEC%`, else `cmd.exe`.
pub fn default_shell() -> String {
    if cfg!(windows) {
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if find_in_path(candidate).is_some() {
                return candidate.to_owned();
            }
        }
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned())
    } else {
        std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_owned())
    }
}

/// macOS terminals conventionally start login shells; other platforms start interactive shells.
fn default_args(shell: &str) -> Vec<String> {
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(shell);
    let posix = matches!(name, "bash" | "zsh" | "sh" | "fish" | "ksh" | "dash");
    if cfg!(target_os = "macos") && posix {
        vec!["-l".to_owned()]
    } else {
        Vec::new()
    }
}

fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_overrides_config_and_default() {
        let config = PtyConfig {
            shell: Some("/bin/zsh".into()),
            args: vec!["-i".into()],
        };
        let spec = LocalRequest {
            shell: Some("/bin/bash".into()),
            ..LocalRequest::default()
        }
        .resolve(&config);
        assert_eq!(spec.program, "/bin/bash");

        let spec = LocalRequest::default().resolve(&config);
        assert_eq!(
            (spec.program.as_str(), spec.args.as_slice()),
            ("/bin/zsh", &["-i".to_owned()][..])
        );
        assert_eq!(spec.size, TermSize::default());
        assert!(
            spec.env
                .iter()
                .any(|(key, value)| key == "TERM" && value == "xterm-256color")
        );
    }

    #[test]
    fn explicit_args_and_env_replace_shell_defaults() {
        let spec = LocalRequest {
            shell: Some("/usr/local/bin/claude".into()),
            args: Some(vec!["auth".into(), "login".into()]),
            env: vec![
                ("PATH".into(), "/custom/bin".into()),
                ("TERM".into(), "xterm".into()),
            ],
            ..LocalRequest::default()
        }
        .resolve(&PtyConfig::default());
        assert_eq!(spec.program, "/usr/local/bin/claude");
        assert_eq!(spec.args, ["auth", "login"]);
        assert!(
            spec.env
                .iter()
                .any(|(key, value)| key == "PATH" && value == "/custom/bin")
        );
        assert_eq!(
            spec.env
                .iter()
                .filter(|(key, _)| key == "TERM")
                .collect::<Vec<_>>(),
            vec![&("TERM".into(), "xterm".into())]
        );
    }
}
