//! `procman summary` — сводка по системе (аналог вкладки «Производительность»).

use std::time::Duration;

use anyhow::Result;
use clap::Args;

use crate::ansi;
use crate::cmd::sort_procs;
use crate::collector::Collector;
use crate::json;
use crate::model::{NetInfo, Snapshot, SortBy};
use crate::table::{self, Align, Cell, Col, Style, Table};
use crate::util::{bar, bytes_long, bytes_short, thousands, uptime as human_uptime};

#[derive(Args, Debug)]
pub struct SummaryArgs {
    /// Интервал наблюдения для расчёта скоростей сети, секунды
    #[arg(short, long, default_value_t = 1.0, value_name = "СЕК")]
    pub interval: f64,
    /// Не ждать интервал: мгновенный срез (скорости сети будут нулевыми)
    #[arg(long)]
    pub fast: bool,
    /// Сколько самых «горячих» процессов показать
    #[arg(short = 'n', long, default_value_t = 5, value_name = "N")]
    pub top: usize,
    /// Вывести данные в формате JSON
    #[arg(long)]
    pub json: bool,
}

fn section(title: &str) {
    println!("\n{}", ansi::bold(title));
}

fn rate(bytes: u64, seconds: f64) -> String {
    if seconds <= 0.0 {
        return "—".to_owned();
    }
    format!("{}/с", bytes_short((bytes as f64 / seconds) as u64))
}

fn cpu_lines(snapshot: &Snapshot, width: usize) -> Vec<String> {
    let cores = &snapshot.cpu.cores;
    if cores.is_empty() {
        return vec![ansi::grey("нет данных о ядрах")];
    }
    let per_row = if width >= 150 {
        4
    } else if width >= 110 {
        3
    } else {
        2
    };
    let cell_width = (width / per_row).clamp(30, 44);
    let bar_width = cell_width.saturating_sub(22).max(6);
    cores
        .chunks(per_row)
        .map(|chunk| {
            let mut line = String::new();
            for (index, core) in chunk.iter().enumerate() {
                let label = if core.name.is_empty() {
                    format!(
                        "ядро {}",
                        cores
                            .iter()
                            .position(|c| c.name == core.name)
                            .unwrap_or(index)
                    )
                } else {
                    core.name.clone()
                };
                let cell = format!(
                    "{:>7} {:>5.1}% {}",
                    label,
                    core.usage,
                    ansi::paint(
                        ansi::load_code(core.usage),
                        &bar(core.usage, bar_width, table::ascii())
                    )
                );
                line.push_str(&cell);
                if index + 1 < chunk.len() {
                    line.push_str("   ");
                }
            }
            line
        })
        .collect()
}

fn net_rate(net: &NetInfo, elapsed: f64) -> (String, String) {
    (
        rate(net.received_delta, elapsed),
        rate(net.transmitted_delta, elapsed),
    )
}

fn json_summary(snapshot: &Snapshot, top: &[crate::model::Proc]) -> String {
    let cores: Vec<String> = snapshot
        .cpu
        .cores
        .iter()
        .enumerate()
        .map(|(index, core)| {
            json::object(vec![
                ("index", json::number(index)),
                ("name", json::string(&core.name)),
                ("usage", json::number(format!("{:.1}", core.usage))),
                ("frequency_mhz", json::number(core.frequency)),
            ])
        })
        .collect();
    let disks: Vec<String> = snapshot
        .disks
        .iter()
        .map(|disk| {
            json::object(vec![
                ("name", json::string(&disk.name)),
                ("mount", json::string(&disk.mount)),
                ("file_system", json::string(&disk.file_system)),
                ("total_bytes", json::number(disk.total)),
                ("available_bytes", json::number(disk.available)),
                (
                    "used_percent",
                    json::number(format!("{:.1}", disk.used_percent())),
                ),
                (
                    "removable",
                    json::string(if disk.removable { "true" } else { "false" }),
                ),
            ])
        })
        .collect();
    let nets: Vec<String> = snapshot
        .nets
        .iter()
        .map(|net| {
            json::object(vec![
                ("interface", json::string(&net.interface)),
                ("received_bytes", json::number(net.total_received)),
                ("transmitted_bytes", json::number(net.total_transmitted)),
                (
                    "received_rate_bps",
                    json::number(rate_bps(net.received_delta, snapshot.elapsed)),
                ),
                (
                    "transmitted_rate_bps",
                    json::number(rate_bps(net.transmitted_delta, snapshot.elapsed)),
                ),
                ("packets_received", json::number(net.packets_received)),
                ("packets_transmitted", json::number(net.packets_transmitted)),
            ])
        })
        .collect();
    let procs: Vec<String> = top
        .iter()
        .map(|proc_| {
            json::object(vec![
                ("pid", json::number(proc_.pid)),
                ("name", json::string(&proc_.name)),
                ("cpu", json::number(format!("{:.2}", proc_.cpu))),
                (
                    "cpu_core_percent",
                    json::number(format!("{:.1}", proc_.cpu_core)),
                ),
                ("memory_bytes", json::number(proc_.mem)),
                ("user", json::string(&proc_.user)),
            ])
        })
        .collect();
    json::object(vec![
        (
            "host",
            json::object(vec![
                ("os", json::string_opt(snapshot.host.os_name.as_deref())),
                (
                    "os_full",
                    json::string_opt(snapshot.host.long_os.as_deref()),
                ),
                (
                    "os_version",
                    json::string_opt(snapshot.host.os_version.as_deref()),
                ),
                (
                    "hostname",
                    json::string_opt(snapshot.host.hostname.as_deref()),
                ),
                ("kernel", json::string_opt(snapshot.host.kernel.as_deref())),
                ("uptime_secs", json::number(snapshot.host.uptime)),
                ("physical_cores", json::number(snapshot.host.physical_cores)),
                ("logical_cores", json::number(snapshot.host.logical_cores)),
                (
                    "load_avg",
                    json::number(format!("{:.2}", snapshot.host.load_avg.0)),
                ),
            ]),
        ),
        (
            "cpu",
            json::object(vec![
                ("brand", json::string(&snapshot.cpu.brand)),
                (
                    "total_usage",
                    json::number(format!("{:.1}", snapshot.cpu.global)),
                ),
                ("cores", json::array(cores)),
            ]),
        ),
        (
            "memory",
            json::object(vec![
                ("total_bytes", json::number(snapshot.mem.total)),
                ("used_bytes", json::number(snapshot.mem.used)),
                ("available_bytes", json::number(snapshot.mem.available)),
                (
                    "used_percent",
                    json::number(format!("{:.1}", snapshot.mem.used_percent())),
                ),
            ]),
        ),
        (
            "swap",
            json::object(vec![
                ("total_bytes", json::number(snapshot.mem.total_swap)),
                ("used_bytes", json::number(snapshot.mem.used_swap)),
                (
                    "used_percent",
                    json::number(format!("{:.1}", snapshot.mem.swap_percent())),
                ),
            ]),
        ),
        ("disks", json::array(disks)),
        ("networks", json::array(nets)),
        ("process_count", json::number(snapshot.procs.len())),
        ("top_processes", json::array(procs)),
    ])
}

fn rate_bps(delta: u64, elapsed: f64) -> u64 {
    if elapsed <= 0.0 {
        0
    } else {
        (delta as f64 / elapsed) as u64
    }
}

/// Выполняет подкоманду `summary`.
pub fn run(args: SummaryArgs) -> Result<()> {
    let mut collector = Collector::new().with_global_cpu();
    let _warmup = collector.refresh(true);
    if !args.fast {
        Collector::pause(Duration::from_secs_f64(args.interval.clamp(0.05, 60.0)));
    }
    let snapshot = collector.refresh(true);

    let mut top: Vec<crate::model::Proc> = snapshot.procs.clone();
    sort_procs(&mut top, SortBy::Cpu, false);
    top.truncate(args.top);

    if args.json {
        println!("{}", json::pretty(&json_summary(&snapshot, &top)));
        return Ok(());
    }

    let width = table::term_width();
    let host = snapshot
        .host
        .long_os
        .clone()
        .or_else(|| snapshot.host.os_name.clone())
        .unwrap_or_else(|| "неизвестная ОС".to_owned());
    println!(
        "{} {}",
        ansi::bold("ПРОИЗВОДИТЕЛЬНОСТЬ"),
        ansi::dim(&format!(
            "· {host} · {} · аптайм {} · ядер {}/{} · процессов {}",
            snapshot.host.hostname.clone().unwrap_or_default(),
            human_uptime(snapshot.host.uptime),
            snapshot.host.physical_cores,
            snapshot.host.logical_cores,
            thousands(snapshot.procs.len() as u64)
        ))
    );

    // --- Процессор -------------------------------------------------------
    section("ПРОЦЕССОР");
    let freq = snapshot
        .cpu
        .cores
        .iter()
        .map(|c| c.frequency)
        .max()
        .unwrap_or(0);
    println!(
        "{} {}",
        ansi::bold(&snapshot.cpu.brand),
        ansi::dim(&format!("· до {freq} МГц"))
    );
    let cpu = snapshot.cpu.global;
    println!(
        "Итого {} {}",
        ansi::paint(ansi::load_code(cpu), &bar(cpu, 28, table::ascii())),
        ansi::bold(&format!("{cpu:.1}%"))
    );
    for line in cpu_lines(&snapshot, width) {
        println!("{line}");
    }
    if snapshot.host.load_avg.0 > 0.0 {
        println!(
            "{}",
            ansi::dim(&format!(
                "Средняя загрузка: {:.2} / {:.2} / {:.2}",
                snapshot.host.load_avg.0, snapshot.host.load_avg.1, snapshot.host.load_avg.2
            ))
        );
    }

    // --- Память ----------------------------------------------------------
    section("ПАМЯТЬ");
    let mem = snapshot.mem;
    let mem_percent = mem.used_percent();
    println!(
        "ОЗУ      {} {} {} / {}",
        ansi::paint(
            ansi::load_code(mem_percent),
            &bar(mem_percent, 28, table::ascii())
        ),
        ansi::bold(&format!("{mem_percent:.1}%")),
        bytes_long(mem.used),
        bytes_long(mem.total)
    );
    println!(
        "{}",
        ansi::dim(&format!(
            "         доступно {} · свободно {} · процессов {} / всего памяти {}",
            bytes_long(mem.available),
            bytes_long(mem.free),
            thousands(snapshot.procs.len() as u64),
            bytes_short(mem.total)
        ))
    );
    if mem.total_swap > 0 {
        let swap = mem.swap_percent();
        println!(
            "Подкачка {} {} {} / {}",
            ansi::paint(ansi::load_code(swap), &bar(swap, 28, table::ascii())),
            ansi::bold(&format!("{swap:.1}%")),
            bytes_long(mem.used_swap),
            bytes_long(mem.total_swap)
        );
    } else {
        println!("{}", ansi::dim("Подкачка: не используется"));
    }

    // --- Диски -----------------------------------------------------------
    section("ДИСКИ");
    if snapshot.disks.is_empty() {
        println!("{}", ansi::grey("диски не обнаружены"));
    } else {
        let cols = vec![
            Col::new("ТОМ", Align::Left, Some(22)).min_width(6),
            Col::new("ФАЙЛОВАЯ СИСТЕМА", Align::Left, Some(14)).min_width(8),
            Col::new("ВСЕГО", Align::Right, Some(10)).min_width(6),
            Col::new("ЗАНЯТО", Align::Right, Some(10)).min_width(6),
            Col::new("СВОБОДНО", Align::Right, Some(10)).min_width(8),
            Col::new("ЗАГРУЗКА", Align::Left, Some(28))
                .min_width(12)
                .flexible(),
        ];
        let mut table = Table::new(cols);
        for disk in &snapshot.disks {
            let percent = disk.used_percent();
            table.push(vec![
                Cell::styled(
                    if disk.mount.is_empty() {
                        disk.name.clone()
                    } else {
                        disk.mount.clone()
                    },
                    Style::Bold,
                ),
                Cell::new(disk.file_system.clone()),
                Cell::new(bytes_short(disk.total)),
                Cell::new(bytes_short(disk.used())),
                Cell::new(bytes_short(disk.available)),
                Cell::styled(
                    format!("{:>5.1}% {}", percent, bar(percent, 12, table::ascii())),
                    Style::for_load(percent),
                ),
            ]);
        }
        for line in table.render_tight(width, None) {
            println!("{line}");
        }
    }

    // --- Сеть ------------------------------------------------------------
    section("СЕТЬ");
    if snapshot.nets.is_empty() {
        println!("{}", ansi::grey("сетевые интерфейсы не обнаружены"));
    } else {
        let cols = vec![
            Col::new("ИНТЕРФЕЙС", Align::Left, Some(26))
                .min_width(10)
                .flexible(),
            Col::new("ПРИНЯТО", Align::Right, Some(11)).min_width(8),
            Col::new("ОТПРАВЛЕНО", Align::Right, Some(11)).min_width(8),
            Col::new("ПРИЁМ", Align::Right, Some(11)).min_width(8),
            Col::new("ПЕРЕДАЧА", Align::Right, Some(11)).min_width(8),
            Col::new("ПАКЕТЫ/с", Align::Right, Some(14)).min_width(8),
        ];
        let mut table = Table::new(cols);
        for net in &snapshot.nets {
            let (down, up) = net_rate(net, snapshot.elapsed);
            table.push(vec![
                Cell::styled(net.interface.clone(), Style::Bold),
                Cell::new(bytes_short(net.total_received)),
                Cell::new(bytes_short(net.total_transmitted)),
                Cell::styled(down, Style::Green),
                Cell::styled(up, Style::Cyan),
                Cell::new(thousands(net.packets_received + net.packets_transmitted)),
            ]);
        }
        for line in table.render_tight(width, None) {
            println!("{line}");
        }
    }

    // --- Топ процессов ---------------------------------------------------
    section("САМЫЕ АКТИВНЫЕ ПРОЦЕССЫ");
    let cols = vec![
        Col::new("PID", Align::Right, Some(8)).min_width(5),
        Col::new("ИМЯ", Align::Left, None).min_width(14).flexible(),
        Col::new("CPU", Align::Right, Some(8)).min_width(6),
        Col::new("ПАМЯТЬ", Align::Right, Some(10)).min_width(6),
        Col::new("ПОЛЬЗОВАТЕЛЬ", Align::Left, Some(20)).min_width(8),
    ];
    let mut table = Table::new(cols);
    for proc_ in &top {
        table.push(vec![
            Cell::new(proc_.pid.to_string()),
            Cell::styled(proc_.name.clone(), Style::Bold),
            Cell::load(format!("{:.1}%", proc_.cpu), proc_.cpu),
            Cell::new(bytes_short(proc_.mem)),
            Cell::new(proc_.user.clone()),
        ]);
    }
    for line in table.render_tight(width, None) {
        println!("{line}");
    }
    Ok(())
}
