//! `procman watch` — a live monitor refreshed in real time.
//!
//! Controls: arrows/`j`/`k` — selection, `s` — sort key, `r` — direction,
//! `f` — filter, `x` — end task, `space` — pause, `+`/`-` — interval,
//! `Enter` — details, `?` — help, `q` — quit.

use std::collections::VecDeque;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::Args;
use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal;

use crate::ansi::{self, Code};
use crate::cmd::list::process_table;
use crate::cmd::{filter_procs, sort_procs};
use crate::collector::{Collector, KillOutcome};
use crate::model::{Proc, Snapshot, SortBy};
use crate::table::{self, Line};
use crate::util::{
    bar, bytes_long, bytes_short, duration, epoch_utc, pad_right, sanitize, sparkline, thousands,
    truncate, uptime as human_uptime,
};

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Refresh interval, seconds
    #[arg(short, long, default_value_t = 1.0, value_name = "SEC")]
    pub interval: f64,
    /// Initial sort key
    #[arg(short, long, value_enum, default_value_t = SortBy::Cpu)]
    pub sort: SortBy,
    /// Sort ascending
    #[arg(long)]
    pub asc: bool,
    /// How many processes to keep in the list
    #[arg(short = 'n', long, default_value_t = 500, value_name = "N")]
    pub top: usize,
    /// Initial filter by name, PID or user
    #[arg(short, long, value_name = "TEXT")]
    pub filter: Option<String>,
    /// Only processes owned by this user
    #[arg(short, long, value_name = "NAME")]
    pub user: Option<String>,
    /// Show command lines
    #[arg(short = 'c', long)]
    pub cmd: bool,
}

/// What to do after handling a key.
enum Action {
    Quit,
    Kill(u32),
}

struct State {
    sort: SortBy,
    ascending: bool,
    top: usize,
    filter: Option<String>,
    user: Option<String>,
    with_cmd: bool,
    interval: Duration,
    paused: bool,
    selected: usize,
    offset: usize,
    history: VecDeque<f32>,
    ticks: u64,
    status: Option<String>,
    pending_kill: Option<(u32, String)>,
    input: Option<String>,
    overlay: Option<Vec<Line>>,
    help: bool,
}

impl State {
    fn new(args: &WatchArgs) -> Self {
        let ascending = if args.asc {
            true
        } else {
            args.sort.default_ascending()
        };
        Self {
            sort: args.sort,
            ascending,
            top: args.top.max(1),
            filter: args.filter.clone(),
            user: args.user.clone(),
            with_cmd: args.cmd,
            interval: Duration::from_secs_f64(args.interval.clamp(0.2, 60.0)),
            paused: false,
            selected: 0,
            offset: 0,
            history: VecDeque::new(),
            ticks: 0,
            status: None,
            pending_kill: None,
            input: None,
            overlay: None,
            help: false,
        }
    }

    fn say(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }

    fn history_values(&self) -> Vec<f32> {
        self.history.iter().copied().collect()
    }

    /// Keeps the selection inside the list and adjusts the scroll window.
    fn clamp_selection(&mut self, rows: usize, visible_rows: usize) {
        if rows == 0 {
            self.selected = 0;
            self.offset = 0;
            return;
        }
        self.selected = self.selected.min(rows - 1);
        if self.selected < self.offset {
            self.offset = self.selected;
        }
        if visible_rows > 0 && self.selected >= self.offset + visible_rows {
            self.offset = self.selected + 1 - visible_rows;
        }
        self.offset = self.offset.min(rows.saturating_sub(visible_rows));
    }
}

/// An RAII terminal wrapper: it restores the screen and the mode on exit,
/// including on errors.
struct Terminal;

impl Terminal {
    fn enter(out: &mut io::Stdout) -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable raw mode")?;
        execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Self)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn heading(text: &str) -> Line {
    Line::plain(format!("{text:<8}"))
}

/// Current time in UTC (hours:minutes:seconds).
fn clock() -> String {
    let now = crate::util::unix_now();
    crate::util::epoch_utc(now)[11..19].to_owned()
}

fn visible_rows(state: &State, snapshot: &Snapshot) -> Vec<Proc> {
    let mut rows = snapshot.procs.clone();
    filter_procs(&mut rows, state.filter.as_deref(), state.user.as_deref());
    sort_procs(&mut rows, state.sort, state.ascending);
    rows.truncate(state.top);
    rows
}

fn header_lines(state: &State, snapshot: &Snapshot, width: usize) -> Vec<Line> {
    let mut lines = Vec::with_capacity(5);

    let os = snapshot
        .host
        .long_os
        .clone()
        .or_else(|| snapshot.host.os_name.clone())
        .unwrap_or_else(|| "unknown OS".to_owned());
    let title = Line::new()
        .styled(Code::Bold, "procman")
        .push(" — task manager   ")
        .styled(
            Code::Dim,
            format!(
                "{os} · {} · uptime {} · cores {}/{} · processes {} · {}",
                snapshot.host.hostname.clone().unwrap_or_default(),
                human_uptime(snapshot.host.uptime),
                snapshot.host.physical_cores,
                snapshot.host.logical_cores,
                thousands(snapshot.procs.len() as u64),
                clock()
            ),
        );
    lines.push(title);

    let cpu = snapshot.cpu.global;
    let bar_width = 24;
    let freq = snapshot
        .cpu
        .cores
        .iter()
        .map(|core| core.frequency)
        .max()
        .unwrap_or(0);
    lines.push(
        heading("CPU")
            .styled(ansi::load_code(cpu), bar(cpu, bar_width, table::ascii()))
            .push(" ")
            .styled(ansi::load_code(cpu), format!("{cpu:>5.1}%"))
            .push("  ")
            .styled(
                Code::Cyan,
                sparkline(&state.history_values(), table::ascii()),
            )
            .styled(Code::Dim, format!("  up to {freq} MHz")),
    );

    let mem = &snapshot.mem;
    let mem_percent = mem.used_percent();
    lines.push(
        heading("MEMORY")
            .styled(
                ansi::load_code(mem_percent),
                bar(mem_percent, bar_width, table::ascii()),
            )
            .push(" ")
            .styled(ansi::load_code(mem_percent), format!("{mem_percent:>5.1}%"))
            .push("  ")
            .styled(
                Code::Dim,
                format!(
                    "{} / {}   swap {:.1}%   available {}",
                    bytes_long(mem.used),
                    bytes_long(mem.total),
                    mem.swap_percent(),
                    bytes_long(mem.available)
                ),
            ),
    );

    lines.push(
        heading("LIST")
            .styled(
                Code::Bold,
                format!(
                    "{}{}",
                    state.sort.label(),
                    if state.ascending { " ↑" } else { " ↓" }
                ),
            )
            .styled(
                Code::Dim,
                format!(
                    "   interval {:.1} s{}   filter: {}{}   refreshes {}",
                    state.interval.as_secs_f64(),
                    if state.paused { "   [PAUSED]" } else { "" },
                    state.filter.clone().unwrap_or_else(|| "—".to_owned()),
                    state
                        .user
                        .clone()
                        .map_or_else(String::new, |user| format!("   user: {user}")),
                    state.ticks
                ),
            )
            .styled(
                Code::Dim,
                if state.with_cmd {
                    "   +command lines"
                } else {
                    ""
                },
            ),
    );

    lines.push(Line::plain(
        (if table::ascii() { '-' } else { '─' })
            .to_string()
            .repeat(width),
    ));
    lines
}

fn footer_lines(state: &State, width: usize) -> Vec<Line> {
    let hints = Line::colored(
        Code::Dim,
        "↑/↓ select · s sort · r order · f filter · c reset · x end task · \
         Enter details · space pause · +/- interval · ? help · q quit",
    );

    let status = if let Some(input) = &state.input {
        Line::new()
            .styled(Code::Bold, "Filter: ")
            .styled(Code::BrightYellow, sanitize(input))
            .styled(Code::BrightYellow, "_")
            .styled(Code::Dim, "   Enter — apply, Esc — cancel")
    } else if let Some((pid, name)) = &state.pending_kill {
        Line::new()
            .styled(Code::Bold, "End task: ")
            .styled(Code::BrightRed, format!("{} (PID {pid})", sanitize(name)))
            .styled(Code::Dim, "   End it? [y/N]")
    } else if let Some(message) = &state.status {
        Line::plain(sanitize(message))
    } else {
        Line::plain("")
    };
    vec![
        hints.pad_to(width).truncated(width, table::ascii()),
        status.pad_to(width).truncated(width, table::ascii()),
    ]
}

/// Draws a bordered panel of the given width.
fn box_panel(title: &str, body: &[String], width: usize) -> Vec<Line> {
    let (tl, tr, bl, br, h, v) = if table::ascii() {
        ('+', '+', '+', '+', '-', '|')
    } else {
        ('┌', '┐', '└', '┘', '─', '│')
    };
    let width = width.clamp(24, 200);
    let inner = width.saturating_sub(6);
    let mut lines = Vec::with_capacity(body.len() + 3);
    lines.push(Line::plain(format!(
        "{tl}{}{tr}",
        h.to_string().repeat(width - 2)
    )));
    lines.push(
        Line::new()
            .push(format!("{v}  "))
            .styled(Code::Bold, pad_right(title, inner))
            .push(format!("  {v}")),
    );
    lines.push(Line::plain(format!(
        "{v}  {}  {v}",
        h.to_string().repeat(inner)
    )));
    for text in body {
        let cell = truncate(text, inner, table::ascii());
        lines.push(
            Line::new()
                .push(format!("{v}  "))
                .push(pad_right(&cell, inner))
                .push(format!("  {v}")),
        );
    }
    lines.push(Line::plain(format!(
        "{bl}{}{br}",
        h.to_string().repeat(width - 2)
    )));
    lines
}

fn help_panel(width: usize) -> Vec<Line> {
    let body = [
        "↑ / ↓ / j / k        select a process",
        "Home / End / g / G   jump to top / bottom",
        "PageUp / PageDown    move a page",
        "s                    next sort key",
        "r                    change sort direction",
        "f                    filter by name, PID or user",
        "c                    clear the filter",
        "x / Delete           end the selected process",
        "Enter                details about the process",
        "space                pause / resume",
        "+ / -                increase / decrease the interval",
        "a                    show / hide command lines",
        "q / Esc              quit",
    ]
    .iter()
    .map(|line| (*line).to_owned())
    .collect::<Vec<_>>();
    box_panel("Help", &body, width.min(80))
}

fn process_panel(snapshot: &Snapshot, proc_: &Proc, width: usize) -> Vec<Line> {
    let parent = proc_.ppid.and_then(|pid| snapshot.by_pid(pid)).map_or_else(
        || "—".to_owned(),
        |parent| format!("{} ({})", sanitize(&parent.name), parent.pid),
    );
    let body = vec![
        format!("PID                 {}", proc_.pid),
        format!("Name                {}", sanitize(&proc_.name)),
        format!(
            "State               {}",
            crate::cmd::list::status_text(&proc_.status)
        ),
        format!("User                {}", sanitize(&proc_.user)),
        format!("Parent              {parent}"),
        format!("Children            {}", proc_.children),
        format!("CPU                 {:.1}%", proc_.cpu),
        format!("Memory (RSS)        {}", bytes_long(proc_.mem)),
        format!("Virtual memory      {}", bytes_long(proc_.virt)),
        format!("Run time            {}", duration(proc_.run_time)),
        format!("Started (UTC)       {}", epoch_utc(proc_.start_time)),
        format!("Disk read           {}", bytes_short(proc_.read_bytes)),
        format!("Disk write          {}", bytes_short(proc_.written_bytes)),
        format!(
            "File                {}",
            proc_
                .exe
                .as_ref()
                .map_or_else(|| "—".to_owned(), |path| sanitize(&path.to_string_lossy()))
        ),
        format!("Command line        {}", sanitize(proc_.display_cmd())),
    ];
    box_panel(
        &format!("{} (PID {})", sanitize(&proc_.name), proc_.pid),
        &body,
        width,
    )
}

fn build_frame(
    state: &mut State,
    snapshot: &Snapshot,
    rows: &[Proc],
    width: usize,
    height: usize,
) -> Vec<Line> {
    let chunk = height.saturating_sub(7).max(1);
    state.clamp_selection(rows.len(), chunk);
    let window = &rows[state.offset..(state.offset + chunk).min(rows.len())];

    let mut lines: Vec<Line> = header_lines(state, snapshot, width)
        .into_iter()
        .map(|line| line.truncated(width, table::ascii()))
        .collect();

    if window.is_empty() {
        lines.push(
            Line::colored(
                Code::Yellow,
                "No processes found — change the filter (f) or reset it (c).",
            )
            .truncated(width, table::ascii()),
        );
    } else {
        let table = process_table(window, state.with_cmd);
        let mut rendered = table.render(width, None);
        let highlight = table.header_lines() + state.selected.saturating_sub(state.offset);
        if let Some(line) = rendered.get(highlight).cloned() {
            if ansi::enabled() {
                rendered[highlight] = line.reversed();
            } else {
                let rest: String = line.plain_text().chars().skip(1).collect();
                rendered[highlight] = Line::plain(format!(">{rest}"));
            }
        }
        for line in rendered {
            lines.push(line.truncated(width, table::ascii()));
        }
    }

    if let Some(panel) = &state.overlay {
        lines = overlay(lines, panel.clone(), width, height);
    } else if state.help {
        lines = overlay(lines, help_panel(width.min(80)), width, height);
    }

    lines.extend(footer_lines(state, width));

    lines.truncate(height);
    while lines.len() < height {
        lines.push(Line::plain("").pad_to(width));
    }
    lines
}

/// Draws a panel centred over the frame.
fn overlay(mut base: Vec<Line>, panel: Vec<Line>, width: usize, height: usize) -> Vec<Line> {
    if panel.len() + 2 >= height {
        return panel
            .into_iter()
            .map(|line| line.truncated(width, table::ascii()).pad_to(width))
            .collect();
    }
    let start = (height - panel.len()) / 2;
    for (offset, line) in panel.into_iter().enumerate() {
        let index = start + offset;
        let line = line.truncated(width, table::ascii()).pad_to(width);
        if index < base.len() {
            base[index] = line;
        } else {
            base.push(line);
        }
    }
    base
}

fn draw(out: &mut io::Stdout, lines: &[Line], width: usize) -> io::Result<()> {
    let mut buffer = String::with_capacity(width * lines.len() + 16);
    buffer.push_str("\x1b[H");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            buffer.push_str("\r\n");
        }
        buffer.push_str(&line.text());
    }
    buffer.push_str("\x1b[J");
    out.write_all(buffer.as_bytes())?;
    out.flush()
}

fn on_key(state: &mut State, key: KeyEvent) -> Option<Action> {
    // The filter input mode swallows every key.
    if let Some(buffer) = state.input.as_mut() {
        match key.code {
            KeyCode::Enter => {
                let text = buffer.trim().to_owned();
                state.filter = if text.is_empty() { None } else { Some(text) };
                state.input = None;
                state.selected = 0;
                state.offset = 0;
                state.say("Filter updated");
            }
            KeyCode::Esc => state.input = None,
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(ch) => buffer.push(ch),
            _ => {}
        }
        return None;
    }

    if state.pending_kill.is_some() {
        if matches!(key.code, KeyCode::Char('y' | 'Y')) {
            if let Some((pid, _)) = state.pending_kill.take() {
                return Some(Action::Kill(pid));
            }
        } else {
            state.pending_kill = None;
            state.say("Cancelled");
        }
        return None;
    }

    if state.overlay.is_some() || state.help {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q' | '?' | 'h') => {
                state.overlay = None;
                state.help = false;
            }
            _ => {}
        }
        return None;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Some(Action::Quit),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Some(Action::Quit);
        }
        KeyCode::Char('?' | 'h') => state.help = true,
        KeyCode::Char('j') | KeyCode::Down => state.selected = state.selected.saturating_add(1),
        KeyCode::Char('k') | KeyCode::Up => state.selected = state.selected.saturating_sub(1),
        KeyCode::Char('g') | KeyCode::Home => state.selected = 0,
        KeyCode::Char('G') | KeyCode::End => state.selected = usize::MAX / 2,
        KeyCode::PageDown => state.selected = state.selected.saturating_add(10),
        KeyCode::PageUp => state.selected = state.selected.saturating_sub(10),
        KeyCode::Char('s') => {
            state.sort = state.sort.next();
            state.ascending = state.sort.default_ascending();
            state.say(format!("Sort: {}", state.sort.label()));
        }
        KeyCode::Char('r') => {
            state.ascending = !state.ascending;
            state.say(format!(
                "Order: {}",
                if state.ascending {
                    "ascending"
                } else {
                    "descending"
                }
            ));
        }
        KeyCode::Char('f' | '/') => state.input = Some(String::new()),
        KeyCode::Char('c') => {
            state.filter = None;
            state.user = None;
            state.say("Filter cleared");
        }
        KeyCode::Char(' ') => {
            state.paused = !state.paused;
            state.say(if state.paused {
                "Paused"
            } else {
                "Refresh resumed"
            });
        }
        KeyCode::Char('+' | '=') => {
            state.interval =
                Duration::from_secs_f64((state.interval.as_secs_f64() + 0.5).min(60.0));
            state.say(format!("Interval: {:.1} s", state.interval.as_secs_f64()));
        }
        KeyCode::Char('-') => {
            state.interval = Duration::from_secs_f64((state.interval.as_secs_f64() - 0.5).max(0.2));
            state.say(format!("Interval: {:.1} s", state.interval.as_secs_f64()));
        }
        KeyCode::Char('a') => {
            state.with_cmd = !state.with_cmd;
            state.say(if state.with_cmd {
                "Command lines shown"
            } else {
                "Command lines hidden"
            });
        }
        _ => {}
    }
    None
}

/// Runs the `watch` subcommand.
pub fn run(args: WatchArgs) -> Result<()> {
    if !io::stdout().is_terminal() {
        bail!("`watch` requires an interactive terminal; use `procman list` to write to a file");
    }

    let mut out = io::stdout();
    let guard = Terminal::enter(&mut out)?;

    let mut collector = Collector::new().with_global_cpu();
    let mut state = State::new(&args);
    let mut snapshot = collector.refresh(false);
    state.history.push_back(snapshot.cpu.global);
    let mut next_refresh = Instant::now() + state.interval;

    loop {
        let width = table::term_width();
        let height = table::term_height();

        if !state.paused && Instant::now() >= next_refresh {
            snapshot = collector.refresh(false);
            state.history.push_back(snapshot.cpu.global);
            while state.history.len() > 80 {
                state.history.pop_front();
            }
            state.ticks += 1;
            next_refresh = Instant::now() + state.interval;
        }

        let rows = visible_rows(&state, &snapshot);
        let frame = build_frame(&mut state, &snapshot, &rows, width, height);
        draw(&mut out, &frame, width)?;

        let timeout = if state.paused {
            Duration::from_millis(250)
        } else {
            next_refresh
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(250))
        };
        if !event::poll(timeout)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let plain_mode = state.input.is_none()
            && state.pending_kill.is_none()
            && state.overlay.is_none()
            && !state.help;

        // Enter — a panel with details about the selected process.
        if plain_mode && key.code == KeyCode::Enter {
            match rows.get(state.selected) {
                Some(proc_) => state.overlay = Some(process_panel(&snapshot, proc_, width)),
                None => state.say("Nothing to show — the list is empty"),
            }
            continue;
        }

        // x / Delete — offer to end the task.
        if plain_mode && matches!(key.code, KeyCode::Delete | KeyCode::Char('x')) {
            match rows.get(state.selected) {
                Some(proc_) if proc_.pid == std::process::id() => {
                    state.say("Refusing: this is procman itself");
                }
                Some(proc_) => state.pending_kill = Some((proc_.pid, proc_.name.clone())),
                None => state.say("Nothing selected"),
            }
            continue;
        }

        match on_key(&mut state, key) {
            Some(Action::Quit) => break,
            Some(Action::Kill(pid)) => {
                let report = collector.kill(pid, false);
                state.say(match report.outcome {
                    KillOutcome::Sent => format!("Process {pid} ended"),
                    KillOutcome::Denied => {
                        format!("Process {pid}: denied — administrator rights are required")
                    }
                    KillOutcome::NotFound => format!("Process {pid} has already exited"),
                });
                snapshot = collector.refresh(false);
                next_refresh = Instant::now() + state.interval;
            }
            None => {}
        }
    }

    drop(guard);
    println!("procman: watch session finished");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CoreInfo, CpuInfo, HostInfo, MemInfo};

    fn args(top: usize) -> WatchArgs {
        WatchArgs {
            interval: 1.0,
            sort: SortBy::Cpu,
            asc: false,
            top,
            filter: None,
            user: None,
            cmd: false,
        }
    }

    fn proc_(pid: u32, name: &str, cpu: f32, mem: u64) -> Proc {
        Proc {
            pid,
            ppid: None,
            name: name.to_owned(),
            cpu,
            cpu_core: cpu * 16.0,
            mem,
            virt: mem * 2,
            status: "Run".to_owned(),
            user: "tester".to_owned(),
            exe: None,
            cwd: None,
            cmd: format!("{name} --service"),
            start_time: 1_700_000_000,
            run_time: 3_725,
            read_bytes: 2048,
            written_bytes: 1024,
            children: 0,
        }
    }

    fn snapshot(count: u32) -> Snapshot {
        Snapshot {
            host: HostInfo {
                os_name: Some("Windows".to_owned()),
                hostname: Some("test-host".to_owned()),
                uptime: 90_061,
                physical_cores: 8,
                logical_cores: 16,
                ..HostInfo::default()
            },
            cpu: CpuInfo {
                brand: "Test CPU".to_owned(),
                global: 42.5,
                cores: (0..16)
                    .map(|index| CoreInfo {
                        name: format!("CPU {index}"),
                        usage: index as f32,
                        frequency: 3200,
                    })
                    .collect(),
            },
            mem: MemInfo {
                total: 16 * 1024 * 1024 * 1024,
                used: 8 * 1024 * 1024 * 1024,
                available: 8 * 1024 * 1024 * 1024,
                free: 4 * 1024 * 1024 * 1024,
                total_swap: 4 * 1024 * 1024 * 1024,
                used_swap: 1024 * 1024 * 1024,
            },
            procs: (0..count)
                .map(|index| {
                    proc_(
                        1000 + index,
                        &format!("proc-{index}.exe"),
                        index as f32,
                        1024 * 1024,
                    )
                })
                .collect(),
            ..Snapshot::default()
        }
    }

    #[test]
    fn frame_fits_terminal_even_when_small() {
        let snapshot = snapshot(200);
        for (width, height) in [(24usize, 5usize), (80, 24), (200, 60)] {
            let mut state = State::new(&args(500));
            let rows = visible_rows(&state, &snapshot);
            let frame = build_frame(&mut state, &snapshot, &rows, width, height);
            assert_eq!(frame.len(), height, "frame height at {width}x{height}");
            for line in &frame {
                assert!(
                    line.visible() <= width,
                    "line is wider than {width}: {:?}",
                    line.plain_text()
                );
            }
        }
    }

    #[test]
    fn selection_scrolls_and_stays_visible() {
        let snapshot = snapshot(100);
        let mut state = State::new(&args(100));
        state.selected = 60;
        let rows = visible_rows(&state, &snapshot);
        let frame = build_frame(&mut state, &snapshot, &rows, 100, 24);
        let selected = rows[state.selected].name.clone();
        assert!(
            frame
                .iter()
                .any(|line| line.plain_text().contains(&selected)),
            "the selected process must be in the frame"
        );
        assert!(state.offset > 0, "the window must have scrolled");
    }

    #[test]
    fn empty_filter_shows_hint() {
        let snapshot = snapshot(10);
        let mut state = State::new(&args(100));
        state.filter = Some("no-such-process".to_owned());
        let rows = visible_rows(&state, &snapshot);
        assert!(rows.is_empty());
        let frame = build_frame(&mut state, &snapshot, &rows, 80, 20);
        assert!(
            frame
                .iter()
                .any(|line| line.plain_text().contains("No processes found"))
        );
        assert_eq!(frame.len(), 20);
    }

    #[test]
    fn panels_and_help_fit_width() {
        for width in [40usize, 80, 120] {
            let mut state = State::new(&args(10));
            state.help = true;
            let snapshot = snapshot(5);
            let rows = visible_rows(&state, &snapshot);
            let frame = build_frame(&mut state, &snapshot, &rows, width, 30);
            assert_eq!(frame.len(), 30);
            for line in &frame {
                assert!(
                    line.visible() <= width,
                    "panel is wider than {width}: {:?}",
                    line.plain_text()
                );
            }
            let panel = process_panel(&snapshot, &snapshot.procs[0], width);
            for line in panel {
                assert!(
                    line.visible() <= width.max(30),
                    "process panel is wider than {width}"
                );
            }
        }
    }

    #[test]
    fn keys_change_state() {
        let mut state = State::new(&args(10));
        let key = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);

        assert!(on_key(&mut state, key(KeyCode::Char('s'))).is_none());
        assert_eq!(state.sort, SortBy::Mem);
        on_key(&mut state, key(KeyCode::Char('r')));
        assert!(state.ascending);
        on_key(&mut state, key(KeyCode::Char(' ')));
        assert!(state.paused);
        on_key(&mut state, key(KeyCode::Char('a')));
        assert!(state.with_cmd);

        // Filter: typing, Backspace, Enter and Esc.
        on_key(&mut state, key(KeyCode::Char('f')));
        assert!(state.input.is_some());
        on_key(&mut state, key(KeyCode::Char('a')));
        on_key(&mut state, key(KeyCode::Char('b')));
        on_key(&mut state, key(KeyCode::Backspace));
        on_key(&mut state, key(KeyCode::Enter));
        assert_eq!(state.filter.as_deref(), Some("a"));
        on_key(&mut state, key(KeyCode::Char('f')));
        on_key(&mut state, key(KeyCode::Char('z')));
        on_key(&mut state, key(KeyCode::Esc));
        assert_eq!(state.filter.as_deref(), Some("a"));

        // Confirming a kill returns the PID.
        state.pending_kill = Some((4242, "victim.exe".to_owned()));
        match on_key(&mut state, key(KeyCode::Char('y'))) {
            Some(Action::Kill(pid)) => assert_eq!(pid, 4242),
            _ => panic!("a kill confirmation was expected"),
        }
        state.pending_kill = Some((4242, "victim.exe".to_owned()));
        assert!(on_key(&mut state, key(KeyCode::Char('n'))).is_none());
        assert!(state.pending_kill.is_none());

        assert!(matches!(
            on_key(&mut state, key(KeyCode::Char('q'))),
            Some(Action::Quit)
        ));
    }
}
