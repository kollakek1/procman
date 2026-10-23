//! Collecting system data through `sysinfo`.
//!
//! The collector lives for the whole run of a command: it keeps state between
//! refreshes, which is required for correct CPU percentages (`sysinfo` computes
//! them as the difference between two samples) and for network rates.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, Process, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, Signal, System, UpdateKind, Users,
};

use crate::model::{CoreInfo, CpuInfo, DiskInfo, HostInfo, MemInfo, NetInfo, Proc, Snapshot};

/// Prints a stage timing to stderr when diagnostics are enabled (`PROCMAN_TIMING=1`).
fn trace(message: &str) {
    if std::env::var_os("PROCMAN_TIMING").is_some() {
        eprintln!("[timing] {message}");
    }
}

/// The result of an attempt to end a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillOutcome {
    /// The signal was sent (the process may take a moment to actually exit).
    Sent,
    /// The system refused (not enough rights, or the process is protected).
    Denied,
    /// That PID no longer exists.
    NotFound,
}

/// Exactly what happened while ending a process.
#[derive(Clone, Copy, Debug)]
pub struct KillReport {
    pub outcome: KillOutcome,
    /// Whether a graceful termination was applied instead of a forced one.
    pub graceful_used: bool,
}

pub struct Collector {
    sys: System,
    users: Users,
    disks: Disks,
    nets: Networks,
    last: Instant,
    /// Whether the global CPU percentage is wanted (it also brings the CPU
    /// brand and frequency along).
    want_global_cpu: bool,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    /// Creates a collector and warms the counters up (see [`Collector::warm_up`]).
    pub fn new() -> Self {
        let t0 = Instant::now();
        // `System::new_all()` is deliberately avoided: it reads the environment
        // of every process and immediately performs an expensive CPU sample
        // (about 0.7 s of extra work per run on this machine). Neither is needed
        // at startup — CPU data is loaded on demand by `with_global_cpu`.
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing())
                .with_memory(MemoryRefreshKind::nothing().with_ram().with_swap()),
        );
        let users = Users::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        let nets = Networks::new_with_refreshed_list();
        trace(&format!("init: {} ms", t0.elapsed().as_millis()));
        let mut collector = Self {
            sys,
            users,
            disks,
            nets,
            last: Instant::now(),
            want_global_cpu: false,
        };
        collector.warm_up();
        trace(&format!("warm-up: {} ms total", t0.elapsed().as_millis()));
        collector
    }

    /// Warm-up before the first snapshot.
    ///
    /// `sysinfo` computes a process load from the difference between two
    /// samples, and the first two differences in a row always come out as zero
    /// (verified on Windows: a core busy at 100% reports 0.0% and then 0.1%).
    /// Three samples are therefore required, spaced by at least
    /// `MINIMUM_CPU_UPDATE_INTERVAL`: a base sample, an intermediate one whose
    /// result is thrown away, and the working one produced by
    /// [`Collector::refresh`]. All samples must request the same set of fields.
    fn warm_up(&mut self) {
        self.sample_processes(); // base sample
        self.wait_for_interval();
        self.sample_processes(); // intermediate (its result is discarded)
    }

    /// Takes a process sample and records its moment: the interval to the next
    /// sample is measured from there. Otherwise the first (slowest) sample eats
    /// the pause and the following measurement turns out too short.
    fn sample_processes(&mut self) {
        self.refresh_processes();
        self.last = Instant::now();
    }

    /// Enables the global CPU percentage. It also loads the CPU brand and
    /// frequency, because in `sysinfo` that is the same expensive first call
    /// (~0.6 s here: the first touch of the performance counters). Only `list`,
    /// `summary` and `watch` ask for it.
    ///
    /// The sample itself is always garbage (it reports 100%): it merely serves
    /// as the base point for the honest measurement taken in
    /// [`Collector::refresh`]. No pause is needed here — the base point already
    /// present in the counters will do.
    #[must_use]
    pub fn with_global_cpu(mut self) -> Self {
        if !self.want_global_cpu {
            self.cpu_sample();
            self.want_global_cpu = true;
        }
        self
    }

    /// Takes a CPU sample and records its moment — the interval to the next
    /// measurement is counted from there.
    fn cpu_sample(&mut self) {
        self.sys.refresh_cpu_all();
        self.last = Instant::now();
    }

    /// Reads the current CPU state.
    fn read_cpu(&self) -> CpuInfo {
        let cpus = self.sys.cpus();
        CpuInfo {
            brand: cpus
                .first()
                .map(|c| c.brand().to_string())
                .unwrap_or_default(),
            global: self.sys.global_cpu_usage(),
            cores: cpus
                .iter()
                .map(|c| CoreInfo {
                    name: c.name().to_string(),
                    usage: c.cpu_usage(),
                    frequency: c.frequency(),
                })
                .collect(),
        }
    }

    /// Refreshes process data with the required set of fields.
    ///
    /// `OnlyIfNotSet` means the expensive fields (executable path, command line,
    /// working directory, owner) are read once per process.
    fn refresh_processes(&mut self) {
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet)
                .with_user(UpdateKind::OnlyIfNotSet),
        );
    }

    /// The minimum pause between samples: CPU percentages are unreliable without it.
    ///
    /// A small margin is added to `MINIMUM_CPU_UPDATE_INTERVAL`: `sysinfo`
    /// skips the computation when the interval comes out slightly below the
    /// minimum, and the Windows scheduler ticks roughly every 15 ms.
    fn wait_for_interval(&self) {
        let target = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL + Duration::from_millis(25);
        let elapsed = self.last.elapsed();
        if elapsed < target {
            std::thread::sleep(target.saturating_sub(elapsed));
        }
    }

    /// Refreshes the data and returns a new snapshot.
    ///
    /// `extended` adds disks and networks — they are more expensive to gather,
    /// so the plain process list does not request them.
    pub fn refresh(&mut self, extended: bool) -> Snapshot {
        let t0 = Instant::now();
        self.wait_for_interval();
        let elapsed = self.last.elapsed().as_secs_f64();

        // The CPU load is read immediately: the process sample that follows
        // recomputes and overwrites this value.
        let cpu = if self.want_global_cpu {
            self.sys.refresh_cpu_usage();
            Some(self.read_cpu())
        } else {
            None
        };
        self.sys
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram().with_swap());
        let t_cpu = t0.elapsed().as_millis();
        self.refresh_processes();
        let t_proc = t0.elapsed().as_millis();
        self.last = Instant::now();
        if extended {
            self.disks.refresh(true);
            self.nets.refresh(true);
        }
        let t_ext = t0.elapsed().as_millis();

        trace(&format!(
            "snapshot{}: prep {t_cpu} ms, processes {t_proc} ms, extra {t_ext} ms",
            if extended { " (extended)" } else { "" }
        ));
        self.build(extended, elapsed, cpu)
    }

    /// A pause between two snapshots (for honest network and CPU rates).
    pub fn pause(duration: Duration) {
        std::thread::sleep(duration);
    }

    /// `cpu` is a CPU load sampled in advance (see [`Collector::refresh`]).
    fn build(&self, extended: bool, elapsed: f64, cpu: Option<CpuInfo>) -> Snapshot {
        let t0 = Instant::now();
        let cpus = self.sys.cpus();
        let cpu = cpu.unwrap_or_else(|| self.read_cpu());
        let mem = MemInfo {
            total: self.sys.total_memory(),
            used: self.sys.used_memory(),
            available: self.sys.available_memory(),
            free: self.sys.free_memory(),
            total_swap: self.sys.total_swap(),
            used_swap: self.sys.used_swap(),
        };
        // On Windows `System::load_average()` computes nothing (it returns
        // zeros) but its first call costs about 0.7 s, so it is simply skipped there.
        let load = if cfg!(windows) {
            sysinfo::LoadAvg {
                one: 0.0,
                five: 0.0,
                fifteen: 0.0,
            }
        } else {
            System::load_average()
        };
        let os_name = System::name();
        let os_version = System::os_version();
        let long_os = System::long_os_version();
        let hostname = System::host_name();
        let kernel = System::kernel_version();
        let physical = System::physical_core_count().unwrap_or(0);
        let host = HostInfo {
            os_name,
            os_version,
            long_os,
            hostname,
            kernel,
            uptime: System::uptime(),
            physical_cores: physical,
            logical_cores: cpus.len(),
            load_avg: (load.one, load.five, load.fifteen),
        };

        let mut procs: Vec<Proc> = self
            .sys
            .processes()
            .iter()
            .map(|(pid, process)| self.to_proc(*pid, process))
            .collect();

        // The number of descendants — like the count next to a name in Task Manager.
        let mut children: HashMap<u32, usize> = HashMap::new();
        for proc_ in &procs {
            if let Some(parent) = proc_.ppid {
                *children.entry(parent).or_insert(0) += 1;
            }
        }
        for proc_ in &mut procs {
            proc_.children = children.get(&proc_.pid).copied().unwrap_or(0);
        }

        let (disks, nets) = if extended {
            (
                self.disks
                    .iter()
                    .map(|disk| DiskInfo {
                        name: disk.name().to_string_lossy().into_owned(),
                        mount: disk.mount_point().to_string_lossy().into_owned(),
                        file_system: disk.file_system().to_string_lossy().into_owned(),
                        total: disk.total_space(),
                        available: disk.available_space(),
                        removable: disk.is_removable(),
                    })
                    .collect(),
                self.nets
                    .iter()
                    .map(|(interface, data)| NetInfo {
                        interface: interface.clone(),
                        total_received: data.total_received(),
                        total_transmitted: data.total_transmitted(),
                        received_delta: data.received(),
                        transmitted_delta: data.transmitted(),
                        packets_received: data.packets_received(),
                        packets_transmitted: data.packets_transmitted(),
                    })
                    .collect(),
            )
        } else {
            (Vec::new(), Vec::new())
        };

        let snapshot = Snapshot {
            host,
            cpu,
            mem,
            disks,
            nets,
            procs,
            elapsed,
        };
        trace(&format!("snapshot build: {} ms", t0.elapsed().as_millis()));
        snapshot
    }

    fn to_proc(&self, pid: Pid, process: &Process) -> Proc {
        let user = process
            .user_id()
            .and_then(|uid| self.users.get_user_by_id(uid))
            .map_or_else(|| "—".to_owned(), |user| user.name().to_owned());
        let cmd = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let disk = process.disk_usage();
        // `sysinfo` reports a single core's load (it can exceed 100%), so the
        // value is divided by the number of logical cores — the way Task Manager counts it.
        let raw = process.cpu_usage();
        let cores = self.sys.cpus().len().max(1) as f32;
        Proc {
            pid: pid.as_u32(),
            ppid: process.parent().map(Pid::as_u32),
            name: process.name().to_string_lossy().into_owned(),
            cpu: raw / cores,
            cpu_core: raw,
            mem: process.memory(),
            virt: process.virtual_memory(),
            status: format!("{:?}", process.status()),
            user,
            exe: process.exe().map(std::path::Path::to_path_buf),
            cwd: process.cwd().map(std::path::Path::to_path_buf),
            cmd,
            start_time: process.start_time(),
            run_time: process.run_time(),
            read_bytes: disk.read_bytes,
            written_bytes: disk.written_bytes,
            children: 0,
        }
    }

    /// Ends a process. `graceful` attempts a soft termination (SIGTERM); when
    /// the platform does not support it (Windows), a forced one is used.
    pub fn kill(&self, pid: u32, graceful: bool) -> KillReport {
        let id = Pid::from_u32(pid);
        let Some(process) = self.sys.process(id) else {
            return KillReport {
                outcome: KillOutcome::NotFound,
                graceful_used: false,
            };
        };
        if graceful {
            match process.kill_with(Signal::Term) {
                Some(true) => {
                    return KillReport {
                        outcome: KillOutcome::Sent,
                        graceful_used: true,
                    };
                }
                Some(false) => {
                    return KillReport {
                        outcome: KillOutcome::Denied,
                        graceful_used: true,
                    };
                }
                None => {}
            }
        }
        KillReport {
            outcome: if process.kill() {
                KillOutcome::Sent
            } else {
                KillOutcome::Denied
            },
            graceful_used: false,
        }
    }

    /// Checks whether a process is still alive, refreshing only that one.
    pub fn alive(&mut self, pid: u32) -> bool {
        let id = Pid::from_u32(pid);
        self.sys
            .refresh_processes(ProcessesToUpdate::Some(&[id]), true);
        self.sys.process(id).is_some()
    }
}
