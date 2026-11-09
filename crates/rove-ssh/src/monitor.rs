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
grep '^cpu' /proc/stat; echo ---; \
grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo; echo ---; \
df -kP 2>/dev/null | tail -n +2; echo ---; \
cat /proc/uptime; echo ---; \
tail -n +3 /proc/net/dev; echo ---; \
nproc; echo ---; \
uname -sr; hostname; echo ---; \
grep -m1 '^PRETTY_NAME=' /etc/os-release 2>/dev/null; true";

/// Largest accepted output of [`SCRIPT`].
const MAX_OUTPUT: usize = 64 * 1024;

/// Cumulative CPU time of one core or of all cores, in clock ticks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CpuTimes {
    /// All time.
    pub total: u64,
    /// Idle time, including I/O wait.
    pub idle: u64,
}

/// One mounted file system.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Disk {
    /// Device or source, for example `/dev/vda1`.
    pub filesystem: String,
    /// Mount point.
    pub mount: String,
    /// Size in bytes.
    pub total: u64,
    /// Used bytes.
    pub used: u64,
}

/// Cumulative counters of one network interface.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NetInterface {
    /// Interface name.
    pub name: String,
    /// Bytes received.
    pub rx: u64,
    /// Bytes sent.
    pub tx: u64,
}

/// One status sample. Byte counts are bytes; counters are cumulative.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ServerStats {
    /// Load averages over 1, 5 and 15 minutes.
    pub load: [f64; 3],
    /// All cores together.
    pub cpu: CpuTimes,
    /// Each core, in order.
    pub cpus: Vec<CpuTimes>,
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
    /// Mounted file systems that hold data (pseudo file systems, loop devices and runtime
    /// mounts left out), one entry per device.
    pub disks: Vec<Disk>,
    /// Seconds since boot.
    pub uptime_secs: u64,
    /// Interfaces except loopback.
    pub interfaces: Vec<NetInterface>,
    /// `uname -sr`.
    pub kernel: String,
    /// Host name.
    pub hostname: String,
    /// Distribution name from `/etc/os-release`, when present.
    pub os: String,
    /// How long the sample took end to end, in milliseconds: one channel round trip plus the
    /// script.
    pub sample_ms: u64,
}

/// Samples the server behind `connection`. Fails with [`SshError::Protocol`] when the output is
/// not what a Linux server prints (the panel then reports the server as unsupported).
pub async fn sample(connection: &Arc<SshConnection>) -> Result<ServerStats, SshError> {
    let started = std::time::Instant::now();
    let output = connection.exec(SCRIPT, MAX_OUTPUT).await?;
    let mut stats = parse(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| SshError::Protocol("unsupported server status output".into()))?;
    stats.sample_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(stats)
}

fn number<T: std::str::FromStr>(text: Option<&str>) -> Option<T> {
    text?.trim().parse().ok()
}

/// Parses one `/proc/stat` cpu line into times.
fn cpu_times(line: &str) -> Option<CpuTimes> {
    let ticks: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|value| value.parse().ok())
        .collect();
    (ticks.len() >= 5).then(|| CpuTimes {
        total: ticks.iter().sum(),
        idle: ticks[3] + ticks[4],
    })
}

/// File systems that only exist in memory or for the system's own bookkeeping.
const PSEUDO_FILESYSTEMS: [&str; 9] = [
    "tmpfs", "devtmpfs", "overlay", "shm", "udev", "none", "efivarfs", "cgroup", "proc",
];

/// Whether a `df` row describes storage worth showing.
fn is_data_disk(filesystem: &str, mount: &str) -> bool {
    !PSEUDO_FILESYSTEMS.contains(&filesystem)
        && !filesystem.starts_with("/dev/loop")
        && !["/dev", "/run", "/sys", "/proc", "/snap", "/var/lib/docker"]
            .iter()
            .any(|prefix| mount == *prefix || mount.starts_with(&format!("{prefix}/")))
}

/// Parses the output of [`SCRIPT`].
fn parse(text: &str) -> Option<ServerStats> {
    let sections: Vec<&str> = text.split("---\n").collect();
    let [load, cpu, mem, disk, uptime, net, cores, system, os] = sections.as_slice() else {
        return None;
    };
    let mut stats = ServerStats::default();

    let mut load = load.split_whitespace();
    for slot in &mut stats.load {
        *slot = number(load.next())?;
    }

    for line in cpu.lines() {
        let label = line.split_whitespace().next().unwrap_or_default();
        if label == "cpu" {
            stats.cpu = cpu_times(line)?;
        } else if label.starts_with("cpu") {
            stats.cpus.extend(cpu_times(line));
        }
    }
    if stats.cpu.total == 0 {
        return None;
    }

    for line in mem.lines() {
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        let bytes = number::<u64>(parts.next())? * 1024;
        match name {
            "MemTotal:" => stats.mem_total = bytes,
            "MemAvailable:" => stats.mem_available = bytes,
            "SwapTotal:" => stats.swap_total = bytes,
            "SwapFree:" => stats.swap_free = bytes,
            _ => {}
        }
    }

    for line in disk.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [filesystem, blocks, used, _available, _capacity, mount @ ..] = fields.as_slice()
        else {
            continue;
        };
        // Mount points may contain spaces.
        let mount = mount.join(" ");
        let (Ok(blocks), Ok(used)) = (blocks.parse::<u64>(), used.parse::<u64>()) else {
            continue;
        };
        if blocks == 0 || !is_data_disk(filesystem, &mount) {
            continue;
        }
        // A device mounted several times (bind mounts) is shown once, at its first mount.
        if stats
            .disks
            .iter()
            .any(|disk| disk.filesystem == *filesystem)
        {
            continue;
        }
        stats.disks.push(Disk {
            filesystem: (*filesystem).to_owned(),
            mount,
            total: blocks * 1024,
            used: used * 1024,
        });
    }

    let uptime: f64 = number(uptime.split_whitespace().next())?;
    stats.uptime_secs = uptime as u64;

    for line in net.lines() {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" {
            continue;
        }
        let counters: Vec<u64> = counters
            .split_whitespace()
            .filter_map(|value| value.parse().ok())
            .collect();
        if counters.len() >= 9 {
            stats.interfaces.push(NetInterface {
                name: name.to_owned(),
                rx: counters[0],
                tx: counters[8],
            });
        }
    }

    stats.cores = number(Some(cores))?;
    let mut system = system.lines();
    stats.kernel = system.next().unwrap_or_default().trim().to_owned();
    stats.hostname = system.next().unwrap_or_default().trim().to_owned();
    stats.os = os
        .trim()
        .strip_prefix("PRETTY_NAME=")
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned();
    Some(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0.52 0.58 0.59 2/1203 41234\n---\n\
cpu  4705 150 1120 16250 520 0 30 0 0 0\n\
cpu0 2400 70 560 8100 260 0 15 0 0 0\n\
cpu1 2305 80 560 8150 260 0 15 0 0 0\n---\n\
MemTotal:       16314432 kB\nMemAvailable:    9123456 kB\nSwapTotal:       2097148 kB\nSwapFree:        2097148 kB\n---\n\
/dev/vda3      527322108 476133332  29719988      95% /\n\
tmpfs            1631444      1200   1630244       1% /run\n\
/dev/vda2        1046508      6220   1040288       1% /boot/efi\n\
/dev/vdb1      104806400  10240000  94566400      10% /data/my files\n\
/dev/vda3      527322108 476133332  29719988      95% /var/bind\n\
/dev/loop0         65536     65536         0     100% /snap/core/1\n---\n\
86400.53 340000.12\n---\n\
    lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0\n  eth0: 5000 40 0 0 0 0 0 0 7000 50 0 0 0 0 0 0\n docker0: 10 1 0 0 0 0 0 0 20 1 0 0 0 0 0 0\n---\n\
2\n---\n\
Linux 6.8.0-147-generic\nnetcup\n---\n\
PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"\n";

    #[test]
    fn parses_a_linux_sample() {
        let stats = parse(SAMPLE).unwrap();
        assert_eq!(stats.load, [0.52, 0.58, 0.59]);
        assert_eq!(stats.cpu.total, 4705 + 150 + 1120 + 16250 + 520 + 30);
        assert_eq!(stats.cpu.idle, 16250 + 520);
        assert_eq!(stats.cpus.len(), 2);
        assert_eq!(stats.cpus[1].idle, 8150 + 260);
        assert_eq!(stats.mem_total, 16314432 * 1024);
        assert_eq!(stats.cores, 2);
        let mounts: Vec<&str> = stats.disks.iter().map(|disk| disk.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/boot/efi", "/data/my files"]);
        assert_eq!(stats.disks[0].used, 476133332 * 1024);
        let names: Vec<&str> = stats
            .interfaces
            .iter()
            .map(|net| net.name.as_str())
            .collect();
        assert_eq!(names, ["eth0", "docker0"]);
        assert_eq!(
            (stats.interfaces[0].rx, stats.interfaces[0].tx),
            (5000, 7000)
        );
        assert_eq!(stats.uptime_secs, 86400);
        assert_eq!(stats.kernel, "Linux 6.8.0-147-generic");
        assert_eq!(stats.hostname, "netcup");
        assert_eq!(stats.os, "Ubuntu 24.04.4 LTS");
    }

    #[test]
    fn tolerates_a_missing_os_release_and_rejects_other_output() {
        let without_os = SAMPLE.split("PRETTY_NAME").next().unwrap();
        assert_eq!(parse(without_os).unwrap().os, "");
        assert_eq!(parse("not linux\n"), None);
        assert_eq!(parse(&SAMPLE.replace("0.52", "x")), None);
    }
}
