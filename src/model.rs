//! Data model: a snapshot of the system and its processes.

use std::path::PathBuf;

use clap::ValueEnum;

/// Sort keys for the process table.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    /// CPU usage
    Cpu,
    /// Memory usage
    Mem,
    /// Process identifier
    Pid,
    /// Process name
    Name,
    /// Owner
    User,
    /// Run time
    Run,
    /// Bytes read from disk
    Read,
    /// Bytes written to disk
    Write,
}

impl SortBy {
    /// Label for the `watch` interface.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Mem => "MEMORY",
            Self::Pid => "PID",
            Self::Name => "NAME",
            Self::User => "USER",
            Self::Run => "TIME",
            Self::Read => "READ",
            Self::Write => "WRITE",
        }
    }

    /// The next key in the cycle — used by the `s` key of `watch`.
    pub fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Mem,
            Self::Mem => Self::Pid,
            Self::Pid => Self::Name,
            Self::Name => Self::User,
            Self::User => Self::Run,
            Self::Run => Self::Read,
            Self::Read => Self::Write,
            Self::Write => Self::Cpu,
        }
    }

    /// Numeric values are sorted descending by default, while identifiers and
    /// names are sorted ascending.
    pub fn default_ascending(self) -> bool {
        matches!(self, Self::Pid | Self::Name | Self::User)
    }
}

/// A single process in a print-friendly shape.
#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    /// CPU usage as a share of all cores (the way Task Manager counts it).
    pub cpu: f32,
    /// CPU usage as a share of a single core (the raw `sysinfo` value).
    pub cpu_core: f32,
    /// Resident memory in bytes.
    pub mem: u64,
    /// Virtual memory in bytes.
    pub virt: u64,
    pub status: String,
    pub user: String,
    pub exe: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub cmd: String,
    pub start_time: u64,
    pub run_time: u64,
    pub read_bytes: u64,
    pub written_bytes: u64,
    pub children: usize,
}

impl Proc {
    /// Whether the needle occurs in the name, command line, PID or owner.
    pub fn matches(&self, needle_lower: &str) -> bool {
        self.name.to_lowercase().contains(needle_lower)
            || self.cmd.to_lowercase().contains(needle_lower)
            || self.user.to_lowercase().contains(needle_lower)
            || self.pid.to_string().contains(needle_lower)
    }

    /// The command line, or the name when the command line is unavailable.
    pub fn display_cmd(&self) -> &str {
        if self.cmd.is_empty() {
            &self.name
        } else {
            &self.cmd
        }
    }
}

/// Information about a CPU core.
#[derive(Clone, Debug)]
pub struct CoreInfo {
    pub name: String,
    pub usage: f32,
    pub frequency: u64,
}

/// CPU load.
#[derive(Clone, Debug, Default)]
pub struct CpuInfo {
    pub brand: String,
    pub global: f32,
    pub cores: Vec<CoreInfo>,
}

/// RAM and swap.
#[derive(Clone, Debug, Default)]
pub struct MemInfo {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub free: u64,
    pub total_swap: u64,
    pub used_swap: u64,
}

impl MemInfo {
    pub fn used_percent(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f64 / self.total as f64 * 100.0) as f32
        }
    }

    pub fn swap_percent(&self) -> f32 {
        if self.total_swap == 0 {
            0.0
        } else {
            (self.used_swap as f64 / self.total_swap as f64 * 100.0) as f32
        }
    }
}

/// General facts about the machine.
#[derive(Clone, Debug, Default)]
pub struct HostInfo {
    pub os_name: Option<String>,
    pub os_version: Option<String>,
    pub long_os: Option<String>,
    pub hostname: Option<String>,
    pub kernel: Option<String>,
    pub uptime: u64,
    pub physical_cores: usize,
    pub logical_cores: usize,
    pub load_avg: (f64, f64, f64),
}

/// A disk.
#[derive(Clone, Debug)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub file_system: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
}

impl DiskInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn used_percent(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used() as f64 / self.total as f64 * 100.0) as f32
        }
    }
}

/// A network interface: totals plus deltas since the previous refresh.
#[derive(Clone, Debug)]
pub struct NetInfo {
    pub interface: String,
    pub total_received: u64,
    pub total_transmitted: u64,
    pub received_delta: u64,
    pub transmitted_delta: u64,
    pub packets_received: u64,
    pub packets_transmitted: u64,
}

/// A full snapshot of the system taken at one moment.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub host: HostInfo,
    pub cpu: CpuInfo,
    pub mem: MemInfo,
    pub disks: Vec<DiskInfo>,
    pub nets: Vec<NetInfo>,
    pub procs: Vec<Proc>,
    /// Time between two consecutive refreshes (used to compute rates).
    pub elapsed: f64,
}

impl Snapshot {
    /// Looks up a process by PID.
    pub fn by_pid(&self, pid: u32) -> Option<&Proc> {
        self.procs.iter().find(|p| p.pid == pid)
    }

    /// Processes with the given name (case insensitive).
    pub fn by_name(&self, name: &str) -> Vec<&Proc> {
        let needle = name.to_lowercase();
        self.procs
            .iter()
            .filter(|p| p.name.to_lowercase() == needle)
            .collect()
    }

    /// Total CPU usage across all processes.
    pub fn total_cpu(&self) -> f32 {
        self.procs.iter().map(|p| p.cpu).sum()
    }

    /// Total memory used by the processes.
    pub fn total_mem(&self) -> u64 {
        self.procs.iter().map(|p| p.mem).sum()
    }
}
