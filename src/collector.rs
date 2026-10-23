//! Сбор данных о системе через `sysinfo`.
//!
//! Коллектор живёт всё время работы команды: он хранит состояние между
//! обновлениями, что нужно для корректных процентов CPU (sysinfo считает их
//! как разницу между двумя выборками) и скоростей сети.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, Process, ProcessRefreshKind,
    ProcessesToUpdate, RefreshKind, Signal, System, UpdateKind, Users,
};

use crate::model::{CoreInfo, CpuInfo, DiskInfo, HostInfo, MemInfo, NetInfo, Proc, Snapshot};

/// Выводит время этапа в stderr при включённой диагностике (`PROCMAN_TIMING=1`).
fn trace(message: &str) {
    if std::env::var_os("PROCMAN_TIMING").is_some() {
        eprintln!("[timing] {message}");
    }
}

/// Результат попытки завершить процесс.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillOutcome {
    /// Сигнал отправлен (процесс может завершаться ещё несколько мгновений).
    Sent,
    /// Система отказала (недостаточно прав или процесс защищён).
    Denied,
    /// Такого PID больше нет.
    NotFound,
}

/// Что именно произошло при завершении.
#[derive(Clone, Copy, Debug)]
pub struct KillReport {
    pub outcome: KillOutcome,
    /// Удалось ли применить мягкое завершение, а не принудительное.
    pub graceful_used: bool,
}

pub struct Collector {
    sys: System,
    users: Users,
    disks: Disks,
    nets: Networks,
    last: Instant,
    /// Нужен ли общий процент загрузки CPU (он же даёт модель и частоту процессора).
    want_global_cpu: bool,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    /// Создаёт коллектор и прогревает счётчики (см. [`Collector::warm_up`]).
    pub fn new() -> Self {
        let t0 = Instant::now();
        // Не используем `System::new_all()`: он читает переменные окружения всех
        // процессов и сразу же делает дорогой замер CPU (на этом компьютере —
        // примерно 0.7 с лишнего времени на каждый запуск). Ни то, ни другое не
        // нужно на старте: процессорные данные догружает `with_global_cpu`.
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing())
                .with_memory(MemoryRefreshKind::nothing().with_ram().with_swap()),
        );
        let users = Users::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        let nets = Networks::new_with_refreshed_list();
        trace(&format!("инициализация: {} мс", t0.elapsed().as_millis()));
        let mut collector = Self {
            sys,
            users,
            disks,
            nets,
            last: Instant::now(),
            want_global_cpu: false,
        };
        collector.warm_up();
        trace(&format!("прогрев: {} мс всего", t0.elapsed().as_millis()));
        collector
    }

    /// Прогрев перед первым снимком.
    ///
    /// `sysinfo` считает загрузку процесса по разнице двух выборок, причём
    /// первые две разницы подряд всегда получаются нулевыми (проверено на Windows:
    /// ядро, занятое на 100%, показывает 0.0% и 0.1%). Поэтому нужны три выборки
    /// с паузами не меньше `MINIMUM_CPU_UPDATE_INTERVAL`: базовая, промежуточная
    /// (её результат отбрасывается) и рабочая — она приходит из [`Collector::refresh`].
    /// Набор полей у всех выборок должен совпадать.
    fn warm_up(&mut self) {
        self.sample_processes(); // базовая выборка
        self.wait_for_interval();
        self.sample_processes(); // промежуточная (её результат отбрасывается)
    }

    /// Делает выборку процессов и запоминает момент выборки: именно от него
    /// отсчитывается интервал до следующей — иначе первая (самая долгая) выборка
    /// «съедает» паузу, и следующий замер оказывается слишком коротким.
    fn sample_processes(&mut self) {
        self.refresh_processes();
        self.last = Instant::now();
    }

    /// Включает расчёт общего процента загрузки CPU. Заодно загружает модель и
    /// частоту процессора — в `sysinfo` это один и тот же дорогой первый вызов
    /// (на этом компьютере ~0.6 с: обращение к счётчикам производительности),
    /// поэтому его запрашивают только `list`, `summary` и `watch`.
    ///
    /// Сам замер всегда получается мусорным (показывает 100%): это лишь базовая
    /// точка для честного замера в [`Collector::refresh`]. Пауза здесь не нужна —
    /// подойдёт и та базовая точка, что уже есть в счётчиках.
    #[must_use]
    pub fn with_global_cpu(mut self) -> Self {
        if !self.want_global_cpu {
            // `refresh_cpu_all` заодно достаёт модель и частоту процессора — это
            // тот самый дорогой первый вызов, о котором сказано выше.
            self.cpu_sample();
            self.want_global_cpu = true;
        }
        self
    }

    /// Делает замер CPU и отмечает его время — от него считается интервал до
    /// следующего замера.
    fn cpu_sample(&mut self) {
        self.sys.refresh_cpu_all();
        self.last = Instant::now();
    }

    /// Читает текущее состояние процессора.
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

    /// Обновляет данные процессов по нужному набору полей.
    ///
    /// `OnlyIfNotSet` означает, что «дорогие» поля (путь, командная строка,
    /// рабочий каталог, владелец) читаются один раз для каждого процесса.
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

    /// Минимальная пауза между выборками: без неё проценты CPU недостоверны.
    ///
    /// К `MINIMUM_CPU_UPDATE_INTERVAL` добавляется небольшой запас: sysinfo
    /// пропускает расчёт, если интервал получился чуть короче минимума, а
    /// системный таймер Windows даёт шаг около 15 мс.
    fn wait_for_interval(&self) {
        let target = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL + Duration::from_millis(25);
        let elapsed = self.last.elapsed();
        if elapsed < target {
            std::thread::sleep(target.saturating_sub(elapsed));
        }
    }

    /// Обновляет данные и возвращает новый снимок.
    ///
    /// `extended` включает диски и сети — они дороже в сборе, поэтому
    /// список процессов их не запрашивает.
    pub fn refresh(&mut self, extended: bool) -> Snapshot {
        let t0 = Instant::now();
        self.wait_for_interval();
        let elapsed = self.last.elapsed().as_secs_f64();

        // Загрузку CPU снимаем сразу: следующий за ним замер процессов
        // пересчитывает и перезаписывает это значение.
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
            "снимок{}: подготовка {t_cpu} мс, процессы {t_proc} мс, доп. {t_ext} мс",
            if extended {
                " (расширенный)"
            } else {
                ""
            }
        ));
        self.build(extended, elapsed, cpu)
    }

    /// Пауза между двумя снимками (для честных скоростей сети и CPU).
    pub fn pause(duration: Duration) {
        std::thread::sleep(duration);
    }

    /// `cpu` — заранее снятая загрузка процессора (см. [`Collector::refresh`]).
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
        // `System::load_average()` на Windows ничего не считает (возвращает нули),
        // но первый вызов стоит около 0.7 с — поэтому там он просто не нужен.
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

        // Количество потомков — как в диспетчере задач рядом с именем.
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
        trace(&format!("сборка снимка: {} мс", t0.elapsed().as_millis()));
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
        // sysinfo отдаёт загрузку одного ядра (может превышать 100%),
        // поэтому делим на число логических ядер — как считает диспетчер задач.
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

    /// Завершает процесс. `graceful` — попытка мягкого завершения (SIGTERM);
    /// если платформа его не поддерживает (Windows), используется принудительное.
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

    /// Проверяет, жив ли процесс, обновив только его.
    pub fn alive(&mut self, pid: u32) -> bool {
        let id = Pid::from_u32(pid);
        self.sys
            .refresh_processes(ProcessesToUpdate::Some(&[id]), true);
        self.sys.process(id).is_some()
    }
}
