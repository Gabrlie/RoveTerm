use std::collections::HashSet;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rove_session::{CpuTimes, Disk, NetInterface, ServerStats};
use sysinfo::{Disks, Networks, System};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(250);
const CPU_TICK_SCALE: u64 = 10_000;
const CPU_BASELINE: CpuTimes = CpuTimes {
    total: CPU_TICK_SCALE,
    idle: CPU_TICK_SCALE,
};

#[derive(Default)]
struct State {
    sys: System,
    networks: Networks,
    disks: Disks,
    cpu: CpuTimes,
    cpus: Vec<CpuTimes>,
    initialized: bool,
    last_sample_at: Option<Instant>,
    last_stats: Option<ServerStats>,
}

/// Samples host status for local desktop sessions.
#[derive(Default)]
pub struct LocalMonitor {
    state: Mutex<State>,
}

impl LocalMonitor {
    /// Creates a monitor that refreshes on demand.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a status sample, reusing samples less than 250 ms old.
    pub fn sample(&self) -> ServerStats {
        let mut state = self.state.lock();
        if state
            .last_sample_at
            .is_some_and(|time| time.elapsed() < SAMPLE_INTERVAL)
            && let Some(stats) = &state.last_stats
        {
            return stats.clone();
        }

        state.sys.refresh_cpu_usage();
        state.sys.refresh_memory();
        state.networks.refresh(true);
        state.disks.refresh(true);
        state.update_cpu_times();

        let sys = &state.sys;
        let mem_total = sys.total_memory();
        let mem_available = match sys.available_memory() {
            0 if mem_total > 0 => mem_total.saturating_sub(sys.used_memory()),
            available => available.min(mem_total),
        };
        let load = System::load_average();
        let mut interfaces: Vec<_> = state
            .networks
            .iter()
            .filter(|(name, _)| !is_loopback(name))
            .map(|(name, data)| NetInterface {
                name: name.clone(),
                rx: data.total_received(),
                tx: data.total_transmitted(),
            })
            .collect();
        interfaces.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        let raw_disks = state.disks.iter().map(|disk| RawDisk {
            filesystem: disk.name().to_string_lossy().into_owned(),
            fs_type: disk.file_system().to_string_lossy().into_owned(),
            mount: disk.mount_point().to_string_lossy().into_owned(),
            total: disk.total_space(),
            available: disk.available_space(),
        });
        let stats = ServerStats {
            load: [load.one, load.five, load.fifteen],
            cpu: state.cpu,
            cpus: state.cpus.clone(),
            cores: u32::try_from(sys.cpus().len()).unwrap_or(u32::MAX),
            mem_total,
            mem_available,
            swap_total: sys.total_swap(),
            swap_free: sys.free_swap(),
            disks: collect_disks(raw_disks, std::env::consts::OS),
            uptime_secs: System::uptime(),
            interfaces,
            kernel: kernel_version(),
            hostname: System::host_name().unwrap_or_default(),
            os: System::long_os_version()
                .or_else(System::name)
                .unwrap_or_default(),
            latency_us: Vec::new(),
        };
        state.last_sample_at = Some(Instant::now());
        state.last_stats = Some(stats.clone());
        stats
    }
}

impl State {
    fn update_cpu_times(&mut self) {
        let cores = self.sys.cpus();
        if self.initialized {
            self.cpu = advance_cpu_times(self.cpu, self.sys.global_cpu_usage());
            for (times, core) in self.cpus.iter_mut().zip(cores) {
                *times = advance_cpu_times(*times, core.cpu_usage());
            }
        } else {
            self.cpu = CPU_BASELINE;
            self.initialized = true;
        }
        self.cpus.resize(cores.len(), CPU_BASELINE);
    }
}

fn advance_cpu_times(previous: CpuTimes, usage_percent: f32) -> CpuTimes {
    let idle_delta = ((100.0 - usage_percent.clamp(0.0, 100.0)) * 100.0).round() as u64;
    CpuTimes {
        total: previous.total + CPU_TICK_SCALE,
        idle: previous.idle + idle_delta.min(CPU_TICK_SCALE),
    }
}

fn kernel_version() -> String {
    let version = System::kernel_version().unwrap_or_default();
    let name = match std::env::consts::OS {
        "macos" | "ios" => "Darwin",
        "linux" | "android" => "Linux",
        "windows" => "Windows",
        _ => return version,
    };
    if version.is_empty() {
        name.to_owned()
    } else if version.starts_with(name) {
        version
    } else {
        format!("{name} {version}")
    }
}

fn is_loopback(name: &str) -> bool {
    matches!(name, "lo" | "lo0")
        || name
            .get(.."loopback".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("loopback"))
}

struct RawDisk {
    filesystem: String,
    fs_type: String,
    mount: String,
    total: u64,
    available: u64,
}

fn collect_disks(raw_disks: impl IntoIterator<Item = RawDisk>, platform: &str) -> Vec<Disk> {
    let mut disks: Vec<_> = raw_disks
        .into_iter()
        .filter(|disk| disk.total > 0 && is_data_disk(disk, platform))
        .map(|disk| Disk {
            filesystem: if disk.filesystem.is_empty() {
                disk.mount.clone()
            } else {
                disk.filesystem
            },
            mount: disk.mount,
            total: disk.total,
            used: disk.total.saturating_sub(disk.available),
        })
        .collect();
    // Prefer the root mount when a filesystem appears more than once.
    disks.sort_by_key(|disk| disk.mount != "/");
    let mut seen = HashSet::new();
    disks.retain(|disk| {
        let mount = if platform == "windows" {
            disk.mount.as_str()
        } else {
            ""
        };
        seen.insert((disk.filesystem.clone(), mount.to_owned()))
    });
    disks
}

fn is_data_disk(disk: &RawDisk, platform: &str) -> bool {
    let mount = &disk.mount;
    let under = |path: &str| {
        mount == path
            || mount
                .strip_prefix(path)
                .is_some_and(|suffix| suffix.starts_with('/'))
    };
    let fs_type = disk.fs_type.as_str();
    let filesystem = disk.filesystem.as_str();
    match platform {
        "macos" => {
            !["/dev", "/System/Volumes", "/Volumes/Recovery"]
                .into_iter()
                .any(under)
                && !matches!(fs_type, "devfs" | "autofs")
                && !matches!(filesystem, "devfs" | "autofs")
                && !filesystem.starts_with("map ")
        }
        "linux" => {
            const PSEUDO: &[&str] = &[
                "tmpfs", "devtmpfs", "overlay", "shm", "udev", "none", "efivarfs", "cgroup", "proc",
            ];
            !PSEUDO.contains(&fs_type)
                && !PSEUDO.contains(&filesystem)
                && !filesystem.starts_with("/dev/loop")
                && !["/dev", "/run", "/sys", "/proc", "/snap", "/var/lib/docker"]
                    .into_iter()
                    .any(under)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_ticks_encode_interval_usage() {
        for (usage, idle_delta) in [(12.34, 8766), (-1.0, 10_000), (101.0, 0), (50.0, 5000)] {
            let updated = advance_cpu_times(CPU_BASELINE, usage);
            assert_eq!(updated.total, CPU_BASELINE.total + CPU_TICK_SCALE);
            assert_eq!(updated.idle, CPU_BASELINE.idle + idle_delta);
        }
    }

    fn disk(filesystem: &str, mount: &str, fs_type: &str, total: u64, available: u64) -> RawDisk {
        RawDisk {
            filesystem: filesystem.into(),
            fs_type: fs_type.into(),
            mount: mount.into(),
            total,
            available,
        }
    }

    #[test]
    fn macos_keeps_root_and_external_volumes() {
        let disks = collect_disks(
            [
                disk("/dev/disk3s1", "/Volumes/Duplicate", "apfs", 100, 20),
                disk("/dev/disk3s1", "/", "apfs", 100, 60),
                disk("/dev/disk3s2", "/System/Volumes/Data", "apfs", 100, 60),
                disk("/dev/disk3s3", "/System/Volumes/Preboot", "apfs", 100, 60),
                disk("/dev/disk3s4", "/Volumes/Recovery", "apfs", 100, 20),
                disk("/dev/disk3s5", "/Volumes/Recovery/Image", "apfs", 100, 20),
                disk("devfs", "/dev", "devfs", 1, 0),
                disk("/dev/example", "/dev/extra", "apfs", 1, 0),
                disk("map auto_home", "/home", "autofs", 1, 0),
                disk("map auto_share", "/shares", "other", 1, 0),
                disk("/dev/disk4s1", "/Volumes/External", "apfs", 200, 175),
                disk("/dev/disk5s1", "/Volumes/Empty", "exfat", 0, 0),
            ],
            "macos",
        );
        assert_eq!(disks.len(), 2);
        assert_eq!((disks[0].mount.as_str(), disks[0].used), ("/", 40));
        assert_eq!(
            (disks[1].mount.as_str(), disks[1].used),
            ("/Volumes/External", 25)
        );
    }

    #[test]
    fn linux_filters_pseudo_filesystems_and_runtime_mounts() {
        let pseudo = [
            "tmpfs", "devtmpfs", "overlay", "shm", "udev", "none", "efivarfs", "cgroup", "proc",
        ]
        .map(|fs| disk("source", "/hidden", fs, 10, 0));
        let runtime_mounts = [
            "/dev",
            "/run/user/1000",
            "/sys",
            "/proc",
            "/snap/app",
            "/var/lib/docker/volumes",
        ]
        .map(|mount| disk("/dev/vdb1", mount, "ext4", 10, 0));
        let data = [
            disk("/dev/loop0", "/mnt/loop", "squashfs", 10, 0),
            disk("/dev/vda1", "/", "ext4", 10, 3),
            disk("/dev/vda1", "/bind", "ext4", 10, 3),
            disk("/dev/vdb1", "/runner", "ext4", 10, 11),
        ];
        let disks = collect_disks(
            pseudo.into_iter().chain(runtime_mounts).chain(data),
            "linux",
        );
        assert_eq!(disks.len(), 2);
        assert_eq!((disks[0].mount.as_str(), disks[0].used), ("/", 7));
        assert_eq!((disks[1].mount.as_str(), disks[1].used), ("/runner", 0));
    }

    #[test]
    fn windows_keeps_volumes_with_matching_labels_on_distinct_drives() {
        let disks = collect_disks(
            [
                disk("Data", "D:\\", "NTFS", 10, 5),
                disk("Data", "D:\\", "NTFS", 10, 5),
                disk("Data", "E:\\", "NTFS", 20, 5),
                disk("", "F:\\", "NTFS", 20, 5),
            ],
            "windows",
        );
        assert_eq!(disks.len(), 3);
        assert_eq!(disks[2].filesystem, "F:\\");
    }

    #[test]
    fn filters_loopback_by_interface_name() {
        for name in ["lo", "lo0", "Loopback Pseudo-Interface 1", "LOOPBACK"] {
            assert!(is_loopback(name), "{name}");
        }
        for name in ["lo1", "en0", "eth0", "Local Area Connection", "éthernet"] {
            assert!(!is_loopback(name), "{name}");
        }
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn samples_the_local_host() {
        let monitor = LocalMonitor::new();
        let first = monitor.sample();
        assert_eq!(monitor.sample(), first);
        assert!(!first.hostname.is_empty());
        assert!(!first.os.is_empty());
        assert!(!first.kernel.is_empty());
        assert!(first.cores > 0);
        assert!(first.mem_total > 0);
        assert!(first.mem_available <= first.mem_total);
        #[cfg(target_os = "macos")]
        assert!(first.mem_available > 0);
        assert!(first.latency_us.is_empty());
        assert_eq!(first.cpu, CPU_BASELINE);
        assert_eq!(first.cpus, vec![CPU_BASELINE; first.cores as usize]);
        assert!(first.disks.iter().all(|disk| {
            !disk.filesystem.is_empty()
                && !disk.mount.is_empty()
                && disk.total > 0
                && disk.used <= disk.total
        }));
        #[cfg(target_os = "macos")]
        assert!(first.disks.iter().any(|disk| disk.mount == "/"));

        std::thread::sleep(Duration::from_millis(260));
        let second = monitor.sample();
        for times in std::iter::once(second.cpu).chain(second.cpus) {
            assert_eq!(times.total, 2 * CPU_TICK_SCALE);
            assert!((CPU_BASELINE.idle..=times.total).contains(&times.idle));
        }
    }
}
