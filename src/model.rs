//! Модель данных: снимок системы и процессов.

use std::path::PathBuf;

use clap::ValueEnum;

/// Ключи сортировки таблицы процессов.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    /// Загрузка процессора
    Cpu,
    /// Потребление памяти
    Mem,
    /// Идентификатор процесса
    Pid,
    /// Имя процесса
    Name,
    /// Пользователь
    User,
    /// Время работы
    Run,
    /// Прочитано с диска
    Read,
    /// Записано на диск
    Write,
}

impl SortBy {
    /// Подпись для интерфейса `watch`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Mem => "ПАМЯТЬ",
            Self::Pid => "PID",
            Self::Name => "ИМЯ",
            Self::User => "ПОЛЬЗОВАТЕЛЬ",
            Self::Run => "ВРЕМЯ",
            Self::Read => "ЧТЕНИЕ",
            Self::Write => "ЗАПИСЬ",
        }
    }

    /// Следующий ключ по кругу — для переключения в `watch` клавишей `s`.
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

    /// По умолчанию числовые показатели сортируются по убыванию,
    /// а идентификаторы и имена — по возрастанию.
    pub fn default_ascending(self) -> bool {
        matches!(self, Self::Pid | Self::Name | Self::User)
    }
}

/// Один процесс в удобном для печати виде.
#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: String,
    /// Загрузка CPU в процентах от суммарной мощности всех ядер (как в диспетчере задач).
    pub cpu: f32,
    /// Загрузка CPU в процентах одного ядра (сырое значение `sysinfo`).
    pub cpu_core: f32,
    /// Резидентная память, байты.
    pub mem: u64,
    /// Виртуальная память, байты.
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
    /// Есть ли подстрока в имени, командной строке, PID или пользователе.
    pub fn matches(&self, needle_lower: &str) -> bool {
        self.name.to_lowercase().contains(needle_lower)
            || self.cmd.to_lowercase().contains(needle_lower)
            || self.user.to_lowercase().contains(needle_lower)
            || self.pid.to_string().contains(needle_lower)
    }

    /// Командная строка либо имя, если командная строка недоступна.
    pub fn display_cmd(&self) -> &str {
        if self.cmd.is_empty() {
            &self.name
        } else {
            &self.cmd
        }
    }
}

/// Информация о ядре процессора.
#[derive(Clone, Debug)]
pub struct CoreInfo {
    pub name: String,
    pub usage: f32,
    pub frequency: u64,
}

/// Загрузка процессора.
#[derive(Clone, Debug, Default)]
pub struct CpuInfo {
    pub brand: String,
    pub global: f32,
    pub cores: Vec<CoreInfo>,
}

/// Оперативная память и файл подкачки.
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

/// Общие сведения о машине.
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

/// Диск.
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

/// Сетевой интерфейс: накопленные объёмы и дельты с прошлого обновления.
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

/// Полный снимок состояния системы на момент выборки.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub host: HostInfo,
    pub cpu: CpuInfo,
    pub mem: MemInfo,
    pub disks: Vec<DiskInfo>,
    pub nets: Vec<NetInfo>,
    pub procs: Vec<Proc>,
    /// Время между двумя последовательными обновлениями (для расчёта скоростей).
    pub elapsed: f64,
}

impl Snapshot {
    /// Ищет процесс по PID.
    pub fn by_pid(&self, pid: u32) -> Option<&Proc> {
        self.procs.iter().find(|p| p.pid == pid)
    }

    /// Процессы с указанным именем (без учёта регистра).
    pub fn by_name(&self, name: &str) -> Vec<&Proc> {
        let needle = name.to_lowercase();
        self.procs
            .iter()
            .filter(|p| p.name.to_lowercase() == needle)
            .collect()
    }

    /// Суммарная загрузка CPU по всем процессам.
    pub fn total_cpu(&self) -> f32 {
        self.procs.iter().map(|p| p.cpu).sum()
    }

    /// Суммарная память процессов.
    pub fn total_mem(&self) -> u64 {
        self.procs.iter().map(|p| p.mem).sum()
    }
}
