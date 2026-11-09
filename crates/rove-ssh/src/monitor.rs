//! Server status for the status panel: one `exec` reads load, CPU counters, memory, root disk,
//! uptime and network counters from a Linux server's `/proc`. Counters are cumulative; callers
//! derive CPU usage and throughput from two samples.

use std::sync::Arc;

use serde::Serialize;

use crate::{SshConnection, SshError};

/// Script run on the server. Sections are separated by `---` lines, in the order parsed by
/// [`parse`]. Only reads files and runs read-only tools.
const SCRIPT: &str = "export LC_ALL=C; \
cat /proc/loadavg; echo ---; \
head -n 1 /proc/stat; echo ---; \
grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo; echo ---; \
df -kP / | tail -n 1; echo ---; \
cat /proc/uptime; echo ---; \
tail -n +3 /proc/net/dev; echo ---; \
nproc; echo ---; \
uname -sr; hostname";

/// Largest accepted output of [`SCRIPT`].
const MAX_OUTPUT: usize = 64 * 1024;

/// One status sample. Byte counts are bytes; CPU counters are clock ticks.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ServerStats {
    /// Load averages over 1, 5 and 15 minutes.
    pub load: [f64; 3],
    /// Cumulative CPU time across all cores.
    pub cpu_total: u64,
    /// Cumulative idle (including I/O wait) CPU time.
    pub cpu_idle: u64,
    /// Online CPU cores.
    pub cores: u32,
    /// Total memory.
    pub mem_total: u64,
    /// Memory available to new work.
    pub mem_available: u64,
    /// Total swap.
    pub swap_total: u64,
    /// Free swap.
    pub swap_free: u64,
    /// Size of the file system holding `/`.
    pub disk_total: u64,
    /// Used space on that file system.
    pub disk_used: u64,
    /// Seconds since boot.
    pub uptime_secs: u64,
    /// Bytes received on all interfaces except loopback.
    pub net_rx: u64,
    /// Bytes sent on all interfaces except loopback.
    pub net_tx: u64,
    /// `uname -sr`.
    pub kernel: String,
    /// Host name.
    pub hostname: String,
}

/// Samples the server behind `connection`. Fails with [`SshError::Protocol`] when the output is
/// not what a Linux server prints (the panel then reports the server as unsupported).
pub async fn sample(connection: &Arc<SshConnection>) -> Result<ServerStats, SshError> {
    let output = connection.exec(SCRIPT, MAX_OUTPUT).await?;
    parse(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| SshError::Protocol("unsupported server status output".into()))
}

fn number<T: std::str::FromStr>(text: Option<&str>) -> Option<T> {
    text?.trim().parse().ok()
}

/// Parses the output of [`SCRIPT`].
fn parse(text: &str) -> Option<ServerStats> {
    let sections: Vec<&str> = text.split("---\n").collect();
    let [load, cpu, mem, disk, uptime, net, cores, system] = sections.as_slice() else {
        return None;
    };
    let mut stats = ServerStats::default();

    let mut load = load.split_whitespace();
    for slot in &mut stats.load {
        *slot = number(load.next())?;
    }

    let ticks: Vec<u64> = cpu
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|value| value.parse().ok())
        .collect();
    if ticks.len() < 5 {
        return None;
    }
    stats.cpu_total = ticks.iter().sum();
    stats.cpu_idle = ticks[3] + ticks[4];

    for line in mem.lines() {
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        let kib: u64 = number(parts.next())?;
        let bytes = kib * 1024;
        match name {
            "MemTotal:" => stats.mem_total = bytes,
            "MemAvailable:" => stats.mem_available = bytes,
            "SwapTotal:" => stats.swap_total = bytes,
            "SwapFree:" => stats.swap_free = bytes,
            _ => {}
        }
    }

    let mut disk = disk.split_whitespace().skip(1);
    stats.disk_total = number::<u64>(disk.next())? * 1024;
    stats.disk_used = number::<u64>(disk.next())? * 1024;

    let uptime: f64 = number(uptime.split_whitespace().next())?;
    stats.uptime_secs = uptime as u64;

    for line in net.lines() {
        let Some((interface, counters)) = line.split_once(':') else {
            continue;
        };
        if interface.trim() == "lo" {
            continue;
        }
        let counters: Vec<u64> = counters
            .split_whitespace()
            .filter_map(|value| value.parse().ok())
            .collect();
        if counters.len() >= 9 {
            stats.net_rx += counters[0];
            stats.net_tx += counters[8];
        }
    }

    stats.cores = number(Some(cores))?;
    let mut system = system.lines();
    stats.kernel = system.next().unwrap_or_default().trim().to_owned();
    stats.hostname = system.next().unwrap_or_default().trim().to_owned();
    Some(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0.52 0.58 0.59 2/1203 41234\n---\n\
cpu  4705 150 1120 16250 520 0 30 0 0 0\n---\n\
MemTotal:       16314432 kB\nMemAvailable:    9123456 kB\nSwapTotal:       2097148 kB\nSwapFree:        2097148 kB\n---\n\
/dev/vda3      527322108 476133332  29719988      95% /\n---\n\
86400.53 340000.12\n---\n\
    lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0\n  eth0: 5000 40 0 0 0 0 0 0 7000 50 0 0 0 0 0 0\n---\n\
4\n---\n\
Linux 6.8.0-147-generic\nnetcup\n";

    #[test]
    fn parses_a_linux_sample() {
        let stats = parse(SAMPLE).unwrap();
        assert_eq!(stats.load, [0.52, 0.58, 0.59]);
        assert_eq!(stats.cpu_total, 4705 + 150 + 1120 + 16250 + 520 + 30);
        assert_eq!(stats.cpu_idle, 16250 + 520);
        assert_eq!(stats.mem_total, 16314432 * 1024);
        assert_eq!(stats.mem_available, 9123456 * 1024);
        assert_eq!(stats.disk_total, 527322108 * 1024);
        assert_eq!(stats.disk_used, 476133332 * 1024);
        assert_eq!(stats.uptime_secs, 86400);
        assert_eq!((stats.net_rx, stats.net_tx), (5000, 7000));
        assert_eq!(stats.cores, 4);
        assert_eq!(stats.kernel, "Linux 6.8.0-147-generic");
        assert_eq!(stats.hostname, "netcup");
    }

    #[test]
    fn rejects_other_output() {
        assert_eq!(parse("not linux\n"), None);
        assert_eq!(parse(&SAMPLE.replace("0.52", "x")), None);
    }
}
