//! Plan usage of the agents that report it: how much of each rate-limit window is used. Codex
//! writes its limits into the session logs under `$CODEX_HOME/sessions` (`token_count` events);
//! Claude Code answers its local `/usage` command with a Markdown report. Both are read as data:
//! nothing here holds or reads a sign-in token.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// One rate-limit window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    /// The window as the agent names it (Claude Code), or empty when `minutes` describes it.
    pub label: String,
    /// Length of the window in minutes, when known (Codex: 300 for five hours, 10080 a week).
    pub minutes: Option<u64>,
    pub used_percent: f64,
    /// When the window starts over (Unix milliseconds), when known.
    pub resets_at: Option<i64>,
    /// When it starts over, as the agent wrote it, when it gives no time.
    pub resets_text: Option<String>,
}

/// Longest log tail read when looking for the latest limits.
const TAIL: u64 = 4 * 1024 * 1024;
/// Most session log files looked at, newest first.
const MAX_FILES: usize = 20;

/// Codex's latest limits from a session log (JSON lines): the last `token_count` event that has
/// `rate_limits`.
pub fn codex_limits(log: &str, now_ms: i64) -> Option<Vec<Window>> {
    log.lines().rev().find_map(|line| {
        if !line.contains("rate_limits") {
            return None;
        }
        let value: Value = serde_json::from_str(line).ok()?;
        let payload = value.get("payload").unwrap_or(&value);
        let limits = payload
            .get("rate_limits")
            .or_else(|| payload.pointer("/info/rate_limits"))?;
        let windows: Vec<Window> = ["primary", "secondary"]
            .iter()
            .filter_map(|name| codex_window(limits.get(*name)?, now_ms))
            .collect();
        (!windows.is_empty()).then_some(windows)
    })
}

fn codex_window(window: &Value, now_ms: i64) -> Option<Window> {
    let used_percent = window.get("used_percent")?.as_f64()?;
    let resets_at = window
        .get("resets_at")
        .and_then(Value::as_i64)
        .map(|seconds| seconds * 1000)
        .or_else(|| {
            window
                .get("resets_in_seconds")
                .and_then(Value::as_i64)
                .map(|seconds| now_ms + seconds * 1000)
        });
    Some(Window {
        label: String::new(),
        minutes: window.get("window_minutes").and_then(Value::as_u64),
        used_percent,
        resets_at,
        resets_text: None,
    })
}

/// Session logs under `codex_home`, newest first (by modification time).
pub fn codex_logs(codex_home: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<(std::time::SystemTime, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() && depth < 4 {
                walk(&path, depth + 1, out);
            } else if meta.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
                && let Ok(modified) = meta.modified()
            {
                out.push((modified, path));
            }
        }
    }
    let mut found = Vec::new();
    walk(&codex_home.join("sessions"), 0, &mut found);
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found
        .into_iter()
        .take(MAX_FILES)
        .map(|(_, path)| path)
        .collect()
}

/// The end of `path`, at most [`TAIL`] bytes, as text.
pub fn tail(path: &Path) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL)))?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Codex's latest limits from the newest session logs under `codex_home`.
pub fn codex_quota(codex_home: &Path, now_ms: i64) -> Option<Vec<Window>> {
    codex_logs(codex_home)
        .iter()
        .find_map(|path| codex_limits(&tail(path).ok()?, now_ms))
}

/// The limits in Claude Code's `/usage` report: lines such as
/// `**5-hour limit** — **23%** · resets in 2h`.
pub fn claude_limits(report: &str) -> Vec<Window> {
    report
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("**")?;
            let (label, rest) = rest.split_once("**")?;
            let rest = rest.trim_start().strip_prefix('—')?.trim_start();
            let rest = rest.strip_prefix("**")?;
            let (percent, rest) = rest.split_once("%**")?;
            let used_percent: f64 = percent.trim().parse().ok()?;
            let resets = rest
                .trim()
                .trim_start_matches(['·', '-', '—', ','])
                .trim()
                .to_owned();
            Some(Window {
                label: label.replace('\\', ""),
                minutes: None,
                used_percent,
                resets_at: None,
                resets_text: (!resets.is_empty()).then_some(resets),
            })
        })
        .take(16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_limits_come_from_the_last_token_count() {
        let log = [
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":10.0,"window_minutes":300,"resets_in_seconds":60}}}}"#,
            r#"{"type":"response_item","payload":{"type":"message"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":42.5,"window_minutes":300,"resets_at":1800000000},"secondary":{"used_percent":7.0,"window_minutes":10080,"resets_in_seconds":3600}}}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message"}}"#,
        ]
        .join("\n");
        let windows = codex_limits(&log, 1_000).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].used_percent, 42.5);
        assert_eq!(windows[0].minutes, Some(300));
        assert_eq!(windows[0].resets_at, Some(1_800_000_000_000));
        assert_eq!(windows[1].resets_at, Some(1_000 + 3_600_000));
        assert_eq!(codex_limits("{}\nnot json rate_limits", 0), None);
    }

    #[test]
    fn codex_logs_are_found_newest_first() {
        let home = tempfile::tempdir().unwrap();
        let day = home.path().join("sessions/2026/11/10");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-a.jsonl"), "{}\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            day.join("rollout-b.jsonl"),
            r#"{"payload":{"rate_limits":{"primary":{"used_percent":3.0}}}}"#,
        )
        .unwrap();
        let logs = codex_logs(home.path());
        assert!(logs[0].ends_with("rollout-b.jsonl"));
        assert_eq!(codex_quota(home.path(), 0).unwrap()[0].used_percent, 3.0);
    }

    #[test]
    fn claude_limits_are_read_from_the_usage_report() {
        let report = "## Usage\n\n### Limits\n\n**5-hour limit** — **23%** · resets 3:00pm\n\n`████░░░░`\n\n**Weekly · all models** — **41%**\n\nSomething else **bold** here";
        let windows = claude_limits(report);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "5-hour limit");
        assert_eq!(windows[0].used_percent, 23.0);
        assert_eq!(windows[0].resets_text.as_deref(), Some("resets 3:00pm"));
        assert_eq!(windows[1].label, "Weekly · all models");
        assert_eq!(windows[1].resets_text, None);
    }
}
