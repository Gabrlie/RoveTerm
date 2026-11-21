use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::{
    TailnetDiscovery, TailnetSnapshot, TailnetState, TailscaleConfig, TailscaleError, status,
};

/// Desktop Provider that runs only `status --json` and never invokes a shell or changes the tailnet.
pub struct CliDiscovery {
    config: TailscaleConfig,
}

impl CliDiscovery {
    /// Validates deployment settings without requiring that the optional client is installed.
    pub fn new(config: TailscaleConfig) -> Result<Self, TailscaleError> {
        Ok(Self {
            config: config.validate()?,
        })
    }

    fn program(&self) -> Option<PathBuf> {
        if let Some(program) = &self.config.program {
            return Some(program.clone());
        }
        let name = if cfg!(windows) {
            "tailscale.exe"
        } else {
            "tailscale"
        };
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                let candidate = directory.join(name);
                if candidate.is_absolute() && candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        if cfg!(target_os = "macos") {
            let app = PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale");
            if app.is_file() {
                return Some(app);
            }
        }
        None
    }
}

fn captured_at() -> Result<i64, TailscaleError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TailscaleError::Clock)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| TailscaleError::Clock)
}

fn state(state: TailnetState, message: &'static str) -> Result<TailnetSnapshot, TailscaleError> {
    Ok(TailnetSnapshot {
        state,
        peers: Vec::new(),
        captured_at: captured_at()?,
        message: Some(message.into()),
    })
}

async fn bounded_read(
    mut stream: impl AsyncRead + Unpin,
    total: Arc<AtomicUsize>,
    limit: usize,
) -> Result<Vec<u8>, TailscaleError> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let count = stream.read(&mut chunk).await.map_err(TailscaleError::Io)?;
        if count == 0 {
            return Ok(bytes);
        }
        let previous = total.fetch_add(count, Ordering::Relaxed);
        if previous.saturating_add(count) > limit {
            return Err(TailscaleError::OutputLimit);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

#[async_trait]
impl TailnetDiscovery for CliDiscovery {
    async fn snapshot(&self) -> Result<TailnetSnapshot, TailscaleError> {
        let Some(program) = self.program() else {
            return state(
                TailnetState::NotInstalled,
                "The Tailscale client is not installed.",
            );
        };
        let mut command = Command::new(program);
        command
            .args(["status", "--json"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear();
        for name in [
            "PATH",
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "TEMP",
            "TMP",
            "TMPDIR",
            "SystemRoot",
            "WINDIR",
            "XDG_RUNTIME_DIR",
            "USER",
            "LOGNAME",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("TAILSCALE_BE_CLI", "1");
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return state(
                    TailnetState::NotInstalled,
                    "The configured Tailscale client was not found.",
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return state(
                    TailnetState::PermissionDenied,
                    "Reading Tailscale status is not permitted.",
                );
            }
            Err(error) => return Err(TailscaleError::Io(error)),
        };
        let stdout = child
            .stdout
            .take()
            .ok_or(TailscaleError::InvalidStatus("stdout pipe unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(TailscaleError::InvalidStatus("stderr pipe unavailable"))?;
        let total = Arc::new(AtomicUsize::new(0));
        let read = async {
            let (status, stdout, stderr) = tokio::try_join!(
                async { child.wait().await.map_err(TailscaleError::Io) },
                bounded_read(stdout, total.clone(), self.config.max_output_bytes),
                bounded_read(stderr, total, self.config.max_output_bytes),
            )?;
            Ok::<_, TailscaleError>((status, stdout, stderr))
        };
        let output =
            match tokio::time::timeout(Duration::from_millis(self.config.timeout_ms), read).await {
                Ok(Ok(output)) => output,
                Ok(Err(error)) => {
                    // The child may already have exited; kill_on_drop also covers cancelled reads.
                    let _ = child.kill().await;
                    return Err(error);
                }
                Err(_) => {
                    // Timeout ends this status request and reaps the process it started.
                    let _ = child.kill().await;
                    return Err(TailscaleError::Timeout);
                }
            };
        if !output.0.success() {
            let stderr = String::from_utf8_lossy(&output.2).to_ascii_lowercase();
            if [
                "permission denied",
                "access denied",
                "access is denied",
                "operation not permitted",
            ]
            .iter()
            .any(|value| stderr.contains(value))
            {
                return state(
                    TailnetState::PermissionDenied,
                    "Reading Tailscale status is not permitted.",
                );
            }
            if let Ok(snapshot) = status::parse(&output.1, captured_at()?) {
                return Ok(snapshot);
            }
            return state(
                TailnetState::ServiceUnavailable,
                "The local Tailscale service is unavailable.",
            );
        }
        status::parse(&output.1, captured_at()?)
    }
}
