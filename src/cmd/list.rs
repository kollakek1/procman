//! `procman list` — the process table (the Processes tab of Task Manager).

use std::fmt::Write as _;

use anyhow::Result;
use clap::Args;

use crate::ansi;
use crate::cmd::{filter_procs, sort_procs};
use crate::collector::Collector;
use crate::json;
use crate::model::{Proc, Snapshot, SortBy};
use crate::table::{self, Align, Cell, Col, Style, Table};
use crate::util::{bar, bytes_long, bytes_short, duration, thousands};

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Sort key
    #[arg(short, long, value_enum, default_value_t = SortBy::Cpu)]
    pub sort: SortBy,
    /// Show only the N heaviest processes
    #[arg(short = 'n', long, value_name = "N")]
    pub top: Option<usize>,
    /// Filter by name, command line, PID or user
    #[arg(short, long, value_name = "TEXT")]
    pub filter: Option<String>,
    /// Only processes owned by this user
    #[arg(short, long, value_name = "NAME")]
    pub user: Option<String>,
    /// Sort ascending
    #[arg(long)]
    pub asc: bool,
    /// Show process command lines
    #[arg(short = 'c', long)]
    pub cmd: bool,
    /// Print the data as JSON
    #[arg(long)]
    pub json: bool,
    /// Do not print the system summary header
    #[arg(long)]
    pub no_header: bool,
}

/// Human readable process state.
pub fn status_text(status: &str) -> &'static str {
    match status {
        "Run" | "Running" => "Running",
        "Sleep" | "Idle" => "Sleeping",
        "Stop" => "Stopped",
        "Zombie" | "Dead" => "Zombie",
        "UninterruptibleDiskSleep" => "Disk wait",
        "Waking" | "Wakekill" | "Parked" | "LockBlocked" => "Waking",
        "Tracing" | "Unknown" => "Unknown",
        _ => "Other",
    }
}

/// A style for a process state.
pub fn status_style(status: &str) -> Style {
    match status {
        "Run" | "Running" => Style::BrightGreen,
        "Zombie" | "Dead" => Style::BrightRed,
        "Stop" => Style::Yellow,
        _ => Style::Grey,
    }
}

/// The process table — shared by `list` and `watch`.
pub fn process_table(procs: &[Proc], with_cmd: bool) -> Table {
    let mut cols = vec![
        Col::new("PID", Align::Right, Some(8)).min_width(5),
        Col::new("NAME", Align::Left, None).min_width(12).flexible(),
        Col::new("CPU", Align::Right, Some(7)).min_width(6),
        Col::new("MEMORY", Align::Right, Some(9)).min_width(6),
        Col::new("USER", Align::Left, Some(20)).min_width(8),
        Col::new("STATE", Align::Left, Some(14)).min_width(8),
        Col::new("TIME", Align::Right, Some(13)).min_width(6),
    ];
    if with_cmd {
        cols.push(
            Col::new("COMMAND LINE", Align::Left, None)
                .min_width(16)
                .flexible(),
        );
    }

    let mut table = Table::new(cols);
    for proc_ in procs {
        let mut row = vec![
            Cell::new(proc_.pid.to_string()),
            Cell::styled(proc_.name.clone(), Style::Bold),
            Cell::load(format!("{:.1}%", proc_.cpu), proc_.cpu),
            Cell::new(bytes_short(proc_.mem)),
            Cell::new(anon_user(&proc_.user)),
            Cell::styled(status_text(&proc_.status), status_style(&proc_.status)),
            Cell::new(duration(proc_.run_time)),
        ];
        if with_cmd {
            row.push(Cell::styled(proc_.display_cmd().to_owned(), Style::Dim));
        }
        table.push(row);
    }
    table
}

/// Strips the domain from a user name (`DOMAIN\user` -> `user`).
fn anon_user(user: &str) -> String {
    match user.rsplit_once('\\') {
        Some((_, name)) if !name.is_empty() => name.to_owned(),
        _ => user.to_owned(),
    }
}

/// Prints the system summary header, in the spirit of the Performance tab.
pub fn print_header(snapshot: &Snapshot, matched: usize, shown: usize, width: usize) {
    let os = snapshot
        .host
        .long_os
        .clone()
        .or_else(|| snapshot.host.os_name.clone())
        .unwrap_or_else(|| "unknown OS".to_owned());
    let host = snapshot.host.hostname.clone().unwrap_or_default();
    let mut first = os;
    if !host.is_empty() {
        let _ = write!(first, " · {host}");
    }
    let _ = write!(
        first,
        " · cores: {}/{}",
        snapshot.host.physical_cores, snapshot.host.logical_cores
    );
    let _ = write!(
        first,
        " · uptime: {}",
        crate::util::uptime(snapshot.host.uptime)
    );
    println!("{}", ansi::bold(&first));

    let cpu = snapshot.cpu.global;
    let mem = snapshot.mem.used_percent();
    let bar_width = (width.saturating_sub(60) / 4).clamp(6, 20);
    let cpu_bar = ansi::paint(ansi::load_code(cpu), &bar(cpu, bar_width, table::ascii()));
    let mem_bar = ansi::paint(ansi::load_code(mem), &bar(mem, bar_width, table::ascii()));
    println!(
        "CPU {cpu_bar} {}   MEMORY {mem_bar} {} / {}   PROCESSES {} of {}",
        ansi::bold(&format!("{cpu:.1}%")),
        bytes_long(snapshot.mem.used),
        bytes_long(snapshot.mem.total),
        thousands(shown as u64),
        thousands(matched as u64),
    );
    println!(
        "{}",
        ansi::dim(&format!(
            "totals across processes: CPU {:.1}% · memory {}",
            snapshot.total_cpu(),
            bytes_long(snapshot.total_mem())
        ))
    );
}

fn json_procs(procs: &[Proc], with_cmd: bool) -> String {
    let items = procs
        .iter()
        .map(|p| {
            let exe = p
                .exe
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned());
            let mut fields = vec![
                ("pid", json::number(p.pid)),
                ("ppid", json::number_opt(p.ppid)),
                ("name", json::string(&p.name)),
                ("cpu", json::number(format!("{:.2}", p.cpu))),
                (
                    "cpu_core_percent",
                    json::number(format!("{:.1}", p.cpu_core)),
                ),
                ("memory_bytes", json::number(p.mem)),
                ("virtual_bytes", json::number(p.virt)),
                ("user", json::string(&p.user)),
                ("status", json::string(&p.status)),
                ("children", json::number(p.children)),
                ("start_time", json::number(p.start_time)),
                ("run_time_secs", json::number(p.run_time)),
                ("read_bytes", json::number(p.read_bytes)),
                ("written_bytes", json::number(p.written_bytes)),
                ("exe", json::string_opt(exe.as_deref())),
            ];
            if with_cmd {
                fields.push(("cmd", json::string(&p.cmd)));
            }
            json::object(fields)
        })
        .collect();
    json::array(items)
}

/// Runs the `list` subcommand.
pub fn run(args: ListArgs) -> Result<()> {
    let mut collector = Collector::new().with_global_cpu();
    let snapshot = collector.refresh(false);

    let mut procs = snapshot.procs.clone();
    filter_procs(&mut procs, args.filter.as_deref(), args.user.as_deref());
    let matched = procs.len();

    let ascending = if args.asc {
        true
    } else {
        args.sort.default_ascending()
    };
    sort_procs(&mut procs, args.sort, ascending);

    if let Some(top) = args.top {
        procs.truncate(top);
    }

    if args.json {
        println!("{}", json::pretty(&json_procs(&procs, args.cmd)));
        return Ok(());
    }

    if !args.no_header {
        print_header(&snapshot, matched, procs.len(), table::term_width());
    }

    if procs.is_empty() {
        println!("{}", ansi::yellow("No processes found — check the filter."));
        return Ok(());
    }

    let table = process_table(&procs, args.cmd);
    for line in table.render_tight(table::term_width(), None) {
        println!("{line}");
    }

    if matched > procs.len() {
        println!(
            "{}",
            ansi::dim(&format!(
                "… showing {} of {} processes (sorted by {})",
                procs.len(),
                matched,
                args.sort.label()
            ))
        );
    }
    Ok(())
}
