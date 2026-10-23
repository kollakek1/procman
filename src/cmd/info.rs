//! `procman info` — подробности об одном процессе (аналог окна «Свойства»).

use anyhow::Result;
use clap::Args;

use crate::ansi;
use crate::cmd::collect_descendants;
use crate::cmd::list::{status_style, status_text};
use crate::cmd::resolve_one;
use crate::collector::Collector;
use crate::json;
use crate::model::{Proc, Snapshot};
use crate::table::{self, Align, Cell, Col, Style, Table};
use crate::util::{bar, bytes_long, bytes_short, duration, epoch_utc, sanitize, thousands};

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// PID (число) или имя процесса
    #[arg(value_name = "ЦЕЛЬ")]
    pub target: String,
    /// Вывести данные в формате JSON
    #[arg(long)]
    pub json: bool,
}

/// Строит цепочку «родитель ← родитель родителя ← …».
fn parent_chain(snapshot: &Snapshot, pid: u32) -> Option<String> {
    let mut parts = Vec::new();
    let mut current = Some(pid);
    let mut guard = 0;
    while let Some(id) = current {
        guard += 1;
        if guard > 16 {
            parts.push("…".to_owned());
            break;
        }
        let Some(proc_) = snapshot.by_pid(id) else {
            break;
        };
        parts.push(format!("{} ({})", proc_.name, proc_.pid));
        current = proc_.ppid.filter(|parent| *parent != id);
        if parts.len() > 1 && current == Some(pid) {
            break;
        }
    }
    if parts.len() < 2 {
        return None;
    }
    Some(parts.join(" ← "))
}

fn json_info(snapshot: &Snapshot, proc_: &Proc) -> String {
    let descendants = collect_descendants(snapshot, proc_.pid);
    let parent = proc_
        .ppid
        .and_then(|pid| snapshot.by_pid(pid))
        .map(|parent| format!("{} ({})", parent.name, parent.pid));
    json::object(vec![
        ("pid", json::number(proc_.pid)),
        ("ppid", json::number_opt(proc_.ppid)),
        ("parent", json::string_opt(parent.as_deref())),
        (
            "parent_chain",
            json::string_opt(parent_chain(snapshot, proc_.pid).as_deref()),
        ),
        ("children_direct", json::number(proc_.children)),
        (
            "children_total",
            json::number(descendants.len().saturating_sub(1)),
        ),
        ("name", json::string(&proc_.name)),
        ("status", json::string(&proc_.status)),
        ("user", json::string(&proc_.user)),
        ("cpu_percent", json::number(format!("{:.2}", proc_.cpu))),
        (
            "cpu_core_percent",
            json::number(format!("{:.1}", proc_.cpu_core)),
        ),
        ("memory_bytes", json::number(proc_.mem)),
        ("virtual_bytes", json::number(proc_.virt)),
        ("run_time_secs", json::number(proc_.run_time)),
        ("start_time", json::number(proc_.start_time)),
        ("start_time_utc", json::string(&epoch_utc(proc_.start_time))),
        ("read_bytes", json::number(proc_.read_bytes)),
        ("written_bytes", json::number(proc_.written_bytes)),
        (
            "exe",
            json::string_opt(proc_.exe.as_deref().map(|p| p.to_string_lossy()).as_deref()),
        ),
        (
            "cwd",
            json::string_opt(proc_.cwd.as_deref().map(|p| p.to_string_lossy()).as_deref()),
        ),
        ("cmd", json::string(&proc_.cmd)),
    ])
}

/// Выполняет подкоманду `info`.
pub fn run(args: InfoArgs) -> Result<()> {
    let mut collector = Collector::new();
    let snapshot = collector.refresh(true);
    let proc_ = resolve_one(&snapshot, &args.target)?;

    if args.json {
        println!("{}", json::pretty(&json_info(&snapshot, &proc_)));
        return Ok(());
    }

    let parent = proc_.ppid.and_then(|pid| snapshot.by_pid(pid));
    let descendants = collect_descendants(&snapshot, proc_.pid)
        .len()
        .saturating_sub(1);

    println!(
        "{} {}",
        ansi::bold(&format!("{} (PID {})", proc_.name, proc_.pid)),
        status_style(&proc_.status).paint(status_text(&proc_.status))
    );

    let cpu = proc_.cpu;
    let mem_percent = if snapshot.mem.total == 0 {
        0.0
    } else {
        (proc_.mem as f64 / snapshot.mem.total as f64 * 100.0) as f32
    };
    let bar_width = 16;
    println!(
        "CPU {} {:.2}%   ПАМЯТЬ {} {} ({:.1}% ОЗУ)",
        ansi::paint(ansi::load_code(cpu), &bar(cpu, bar_width, table::ascii())),
        cpu,
        ansi::paint(
            ansi::load_code(mem_percent),
            &bar(mem_percent, bar_width, table::ascii())
        ),
        bytes_long(proc_.mem),
        mem_percent
    );
    println!();

    let mut fields: Vec<(String, String, Style)> = vec![
        (
            "Идентификатор (PID)".to_owned(),
            proc_.pid.to_string(),
            Style::Plain,
        ),
        (
            "Родительский процесс".to_owned(),
            parent.map_or_else(
                || "не найден".to_owned(),
                |parent| format!("{} (PID {})", parent.name, parent.pid),
            ),
            Style::Plain,
        ),
    ];
    if let Some(chain) = parent_chain(&snapshot, proc_.pid) {
        fields.push(("Цепочка процессов".to_owned(), chain, Style::Dim));
    }
    fields.extend([
        (
            "Дочерних процессов".to_owned(),
            proc_.children.to_string(),
            Style::Plain,
        ),
        (
            "Всего в ветке".to_owned(),
            format!("{} (включая дочерние)", thousands(descendants as u64)),
            Style::Dim,
        ),
        (
            "Состояние".to_owned(),
            status_text(&proc_.status).to_owned(),
            status_style(&proc_.status),
        ),
        ("Пользователь".to_owned(), proc_.user.clone(), Style::Plain),
        (
            "Загрузка CPU".to_owned(),
            format!(
                "{:.2}% от всех ядер ({:.1}% одного ядра)",
                proc_.cpu, proc_.cpu_core
            ),
            Style::for_load(proc_.cpu),
        ),
        (
            "Память (RSS)".to_owned(),
            bytes_long(proc_.mem),
            Style::Plain,
        ),
        (
            "Виртуальная память".to_owned(),
            bytes_long(proc_.virt),
            Style::Plain,
        ),
        (
            "Время работы".to_owned(),
            duration(proc_.run_time),
            Style::Plain,
        ),
        (
            "Запущен (UTC)".to_owned(),
            format!(
                "{} ({} с назад)",
                epoch_utc(proc_.start_time),
                proc_.run_time
            ),
            Style::Plain,
        ),
        (
            "Прочитано с диска".to_owned(),
            bytes_short(proc_.read_bytes),
            Style::Plain,
        ),
        (
            "Записано на диск".to_owned(),
            bytes_short(proc_.written_bytes),
            Style::Plain,
        ),
    ]);

    let cols = vec![
        Col::new("ПАРАМЕТР", Align::Left, Some(26)).min_width(18),
        Col::new("ЗНАЧЕНИЕ", Align::Left, None)
            .min_width(20)
            .flexible(),
    ];
    let mut table = Table::plain(cols);
    for (key, value, style) in fields {
        table.push(vec![
            Cell::styled(key, Style::Dim),
            Cell::styled(sanitize(&value), style),
        ]);
    }
    for line in table.render_tight(table::term_width(), None) {
        println!("{line}");
    }

    println!();
    for (label, value) in [
        (
            "Исполняемый файл",
            proc_.exe.as_ref().map(|p| p.to_string_lossy().into_owned()),
        ),
        (
            "Рабочий каталог",
            proc_.cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
        ),
    ] {
        match value {
            Some(value) => println!("{} {}", ansi::dim(&format!("{label}:")), sanitize(&value)),
            None => println!(
                "{} {}",
                ansi::dim(&format!("{label}:")),
                ansi::grey("недоступно")
            ),
        }
    }
    if proc_.cmd.is_empty() {
        println!(
            "{} {}",
            ansi::dim("Командная строка:"),
            ansi::grey("недоступна")
        );
    } else {
        println!(
            "{} {}",
            ansi::dim("Командная строка:"),
            sanitize(&proc_.cmd)
        );
    }
    Ok(())
}
