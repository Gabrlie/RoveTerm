//! Shared host status samples.

use serde::Serialize;

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
    /// Latency of each segment of the path to the server, in microseconds: one entry for a
    /// direct or proxied connection, one more per jump host. See
    /// `SshConnection::hop_latency`.
    pub latency_us: Vec<u64>,
}
