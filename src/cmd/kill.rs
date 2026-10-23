//! `procman kill` — завершение процессов по PID или имени (аналог «Снять задачу»).

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
    /// PID (число) или имя процесса; можно указать несколько значений
    #[arg(required = true, value_name = "ЦЕЛЬ")]
    pub targets: Vec<String>,
    /// Не спрашивать подтверждение
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// Только показать, что будет завершено, ничего не делать
    #[arg(long)]
    pub dry_run: bool,
    /// Мягкое завершение (SIGTERM). Если платформа его не поддерживает
    /// (Windows), будет применено принудительное завершение
    #[arg(short = 't', long)]
    pub terminate: bool,
    /// Завершить также все дочерние процессы
    #[arg(long)]
    pub with_children: bool,
    /// Вывести отчёт в JSON
    #[arg(long)]
    pub json: bool,
}

/// Итог по одному процессу — в человекочитаемом и JSON-виде.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Сигнал отправлен и процесс уже завершился.
    Finished,
    /// Отправлен мягкий сигнал, процесс ещё завершается.
    Graceful,
    /// Система отказала в завершении.
    Denied,
    /// Процесса уже нет.
    Missing,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Self::Finished => "завершён",
            Self::Graceful => "отправлен SIGTERM",
            Self::Denied => "отказано",
            Self::Missing => "не найден",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Finished => "принудительно",
            Self::Graceful => "мягкое завершение",
            Self::Denied => "недостаточно прав или процесс защищён",
            Self::Missing => "процесс уже завершился",
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
    Ok(matches!(answer.as_str(), "y" | "yes" | "д" | "да" | "1"))
}

fn plan_table(plan: &[Proc]) -> Table {
    let cols = vec![
        Col::new("PID", Align::Right, Some(8)).min_width(5),
        Col::new("ИМЯ", Align::Left, None).min_width(14).flexible(),
        Col::new("CPU", Align::Right, Some(7)).min_width(6),
        Col::new("ПАМЯТЬ", Align::Right, Some(9)).min_width(6),
        Col::new("ПОЛЬЗОВАТЕЛЬ", Align::Left, Some(20)).min_width(8),
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

/// Убирает дубликаты PID, сохраняя порядок появления.
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

/// Выполняет подкоманду `kill`.
pub fn run(args: KillArgs) -> Result<()> {
    if args.json && !args.yes {
        bail!("в режиме --json подтверждение невозможно: добавьте -y");
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
        bail!("процессы не найдены: {}", missing.join(", "));
    }
    if !missing.is_empty() {
        eprintln!(
            "{}",
            ansi::yellow(&format!("Не найдено: {}", missing.join(", ")))
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
            bail!("отказ: единственная цель — сам procman (PID {self_pid})");
        }
        eprintln!(
            "{}",
            ansi::yellow("Собственный процесс procman пропущен — завершить его нельзя")
        );
    }

    eprintln!(
        "{}",
        ansi::bold(&format!(
            "К завершению: {}{}",
            crate::util::plural(plan.len() as u64, "процесс", "процесса", "процессов"),
            if args.terminate {
                " (мягкий режим)"
            } else {
                ""
            }
        ))
    );
    for line in plan_table(&plan).render_tight(table::term_width(), None) {
        eprintln!("{line}");
    }

    if args.dry_run {
        eprintln!(
            "{}",
            ansi::yellow("Режим проверки: ни один процесс не завершён.")
        );
        return Ok(());
    }

    if !args.yes
        && !confirm(&format!(
            "Завершить {}? [y/N] ",
            crate::util::plural(plan.len() as u64, "процесс", "процесса", "процессов")
        ))?
    {
        println!("{}", ansi::yellow("Отменено."));
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

    // Проверяем фактическое состояние: процесс мог завершаться асинхронно.
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
                "завершается"
            } else {
                "завершён"
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
            Col::new("ИМЯ", Align::Left, None).min_width(14).flexible(),
            Col::new("ИТОГ", Align::Left, Some(18)).min_width(8),
            Col::new("ПРИМЕЧАНИЕ", Align::Left, None)
                .min_width(10)
                .flexible(),
        ];
        let mut table = Table::new(cols);
        for (pid, name, outcome) in &report {
            let style = if still_alive.contains(pid) {
                Style::Yellow
            } else {
                outcome.style()
            };
            let note = if still_alive.contains(pid) {
                "завершается"
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
            "Готово: завершено {}, ещё завершается {}, неудач {}",
            thousands(finished.len() as u64),
            thousands(remaining as u64),
            thousands(failures as u64),
        );
        if failures > 0 {
            bail!(
                "не удалось завершить {} — запустите терминал с повышенными правами",
                crate::util::plural(failures as u64, "процесс", "процесса", "процессов")
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
            "не удалось завершить {} — запустите терминал с повышенными правами",
            crate::util::plural(failures as u64, "процесс", "процесса", "процессов")
        );
    }
    Ok(())
}
