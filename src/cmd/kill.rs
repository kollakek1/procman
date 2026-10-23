//! `procman kill` — ending processes by PID or name (the End task action).

use std::collections::HashSet;
use std::io::Write as _;

use anyhow::{Result, bail};
use clap::Args;

use crate::ansi;
use crate::cmd::{collect_descendants, resolve_targets};
use crate::collector::{Collector, KillOutcome};
use crate::json;
use crate::model::Proc;
use crate::table::{self, Align, Cell, Col, Style, Table};
use crate::util::{bytes_short, sanitize, thousands};

#[derive(Args, Debug)]
pub struct KillArgs {
    /// PID (a number) or process name; several values are allowed
    #[arg(required = true, value_name = "TARGET")]
    pub targets: Vec<String>,
    /// Do not ask for confirmation
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// Only show what would be ended, do nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Graceful termination (SIGTERM). When the platform does not support it
    /// (Windows), a forced termination is used instead
    #[arg(short = 't', long)]
    pub terminate: bool,
    /// Also end all child processes
    #[arg(long)]
    pub with_children: bool,
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}

/// The result for a single process — in human readable and JSON form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// The signal was sent and the process has already exited.
    Finished,
    /// A graceful signal was sent and the process is still ending.
    Graceful,
    /// The system refused to end the process.
    Denied,
    /// The process is already gone.
    Missing,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Self::Finished => "ended",
            Self::Graceful => "SIGTERM sent",
            Self::Denied => "denied",
            Self::Missing => "gone",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Finished => "forced",
            Self::Graceful => "graceful termination",
            Self::Denied => "not enough rights or the process is protected",
            Self::Missing => "the process has already exited",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::Finished | Self::Graceful => Style::Green,
            Self::Denied => Style::Red,
            Self::Missing => Style::Grey,
        }
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes" | "1"))
}

fn plan_table(plan: &[Proc]) -> Table {
    let cols = vec![
        Col::new("PID", Align::Right, Some(8)).min_width(5),
        Col::new("NAME", Align::Left, None).min_width(14).flexible(),
        Col::new("CPU", Align::Right, Some(7)).min_width(6),
        Col::new("MEMORY", Align::Right, Some(9)).min_width(6),
        Col::new("USER", Align::Left, Some(20)).min_width(8),
    ];
    let mut table = Table::new(cols);
    for proc_ in plan {
        table.push(vec![
            Cell::new(proc_.pid.to_string()),
            Cell::styled(proc_.name.clone(), Style::Bold),
            Cell::load(format!("{:.1}%", proc_.cpu), proc_.cpu),
            Cell::new(bytes_short(proc_.mem)),
            Cell::new(proc_.user.clone()),
        ]);
    }
    table
}

/// Removes duplicate PIDs while keeping the order of appearance.
fn dedupe(procs: Vec<Proc>) -> Vec<Proc> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut result = Vec::with_capacity(procs.len());
    for proc_ in procs {
        if seen.insert(proc_.pid) {
            result.push(proc_);
        }
    }
    result
}

/// Runs the `kill` subcommand.
pub fn run(args: KillArgs) -> Result<()> {
    if args.json && !args.yes {
        bail!("confirmation is not possible in --json mode: add -y");
    }

    let mut collector = Collector::new();
    let snapshot = collector.refresh(false);

    let mut planned: Vec<Proc> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for target in &args.targets {
        let found = resolve_targets(&snapshot, target);
        if found.is_empty() {
            missing.push(target.clone());
        } else {
            planned.extend(found);
        }
    }

    if planned.is_empty() {
        bail!("no processes found: {}", missing.join(", "));
    }
    if !missing.is_empty() {
        eprintln!(
            "{}",
            ansi::yellow(&format!("Not found: {}", missing.join(", ")))
        );
    }

    if args.with_children {
        let mut expanded = Vec::new();
        for proc_ in &planned {
            for pid in collect_descendants(&snapshot, proc_.pid) {
                if let Some(child) = snapshot.by_pid(pid) {
                    expanded.push(child.clone());
                }
            }
        }
        planned.extend(expanded);
    }

    let mut plan = dedupe(planned);
    let self_pid = std::process::id();
    if plan.iter().any(|p| p.pid == self_pid) {
        plan.retain(|p| p.pid != self_pid);
        if plan.is_empty() {
            bail!("refusing: the only target is procman itself (PID {self_pid})");
        }
        eprintln!(
            "{}",
            ansi::yellow("Skipping procman's own process — it cannot be ended")
        );
    }

    eprintln!(
        "{}",
        ansi::bold(&format!(
            "About to end: {}{}",
            crate::util::plural(plan.len() as u64, "process", "processes"),
            if args.terminate {
                " (graceful mode)"
            } else {
                ""
            }
        ))
    );
    for line in plan_table(&plan).render_tight(table::term_width(), None) {
        eprintln!("{line}");
    }

    if args.dry_run {
        eprintln!("{}", ansi::yellow("Dry run: no process was ended."));
        return Ok(());
    }

    if !args.yes
        && !confirm(&format!(
            "End {}? [y/N] ",
            crate::util::plural(plan.len() as u64, "process", "processes")
        ))?
    {
        println!("{}", ansi::yellow("Cancelled."));
        return Ok(());
    }

    let mut report: Vec<(u32, String, Outcome)> = Vec::new();
    for proc_ in &plan {
        let result = collector.kill(proc_.pid, args.terminate);
        let outcome = match result.outcome {
            KillOutcome::Sent if result.graceful_used => Outcome::Graceful,
            KillOutcome::Sent => Outcome::Finished,
            KillOutcome::Denied => Outcome::Denied,
            KillOutcome::NotFound => Outcome::Missing,
        };
        report.push((proc_.pid, proc_.name.clone(), outcome));
    }

    // Verify the actual state: a process may exit asynchronously.
    let mut still_alive: Vec<u32> = Vec::new();
    let mut finished: Vec<u32> = Vec::new();
    for proc_ in &plan {
        if collector.alive(proc_.pid) {
            still_alive.push(proc_.pid);
        } else {
            finished.push(proc_.pid);
        }
    }

    let rows: Vec<String> = report
        .iter()
        .map(|(pid, name, outcome)| {
            let state = if still_alive.contains(pid) {
                "ending"
            } else {
                "ended"
            };
            json::object(vec![
                ("pid", json::number(*pid)),
                ("name", json::string(name)),
                ("killed", json::number(i32::from(finished.contains(pid)))),
                ("status", json::string(state)),
                ("detail", json::string(outcome.note())),
                ("method", json::string(outcome.label())),
            ])
        })
        .collect();
    let summary = json::object(vec![
        ("requested", json::number(plan.len())),
        ("finished", json::number(finished.len())),
        ("still_running", json::number(still_alive.len())),
        ("results", json::array(rows)),
    ]);

    if args.json {
        println!("{}", json::pretty(&summary));
    } else {
        let cols = vec![
            Col::new("PID", Align::Right, Some(8)).min_width(5),
            Col::new("NAME", Align::Left, None).min_width(14).flexible(),
            Col::new("RESULT", Align::Left, Some(18)).min_width(8),
            Col::new("NOTE", Align::Left, None).min_width(10).flexible(),
        ];
        let mut table = Table::new(cols);
        for (pid, name, outcome) in &report {
            let style = if still_alive.contains(pid) {
                Style::Yellow
            } else {
                outcome.style()
            };
            let note = if still_alive.contains(pid) {
                "ending"
            } else {
                outcome.note()
            };
            table.push(vec![
                Cell::new(pid.to_string()),
                Cell::styled(sanitize(name), Style::Bold),
                Cell::styled(outcome.label(), style),
                Cell::styled(note, Style::Dim),
            ]);
        }
        for line in table.render_tight(table::term_width(), None) {
            println!("{line}");
        }
        let remaining = still_alive.len();
        let failures = report
            .iter()
            .filter(|(_, _, outcome)| matches!(outcome, Outcome::Denied | Outcome::Missing))
            .filter(|(pid, _, _)| !finished.contains(pid))
            .count();
        println!(
            "Done: ended {}, still ending {}, failures {}",
            thousands(finished.len() as u64),
            thousands(remaining as u64),
            thousands(failures as u64),
        );
        if failures > 0 {
            bail!(
                "could not end {} — run the terminal with elevated rights",
                crate::util::plural(failures as u64, "process", "processes")
            );
        }
        return Ok(());
    }

    let failures = report
        .iter()
        .filter(|(_, _, outcome)| matches!(outcome, Outcome::Denied | Outcome::Missing))
        .filter(|(pid, _, _)| !finished.contains(pid))
        .count();
    if failures > 0 {
        bail!(
            "could not end {} — run the terminal with elevated rights",
            crate::util::plural(failures as u64, "process", "processes")
        );
    }
    Ok(())
}
