# procman — a task manager for your terminal

A console counterpart of the Windows Task Manager written in Rust: a process
list with honest CPU percentages, ending tasks, a process tree, a performance
summary and a live monitor that refreshes in real time.

```
Windows 11 Pro for Workstations · kolla · cores: 8/16 · uptime: 2h 43m 22s
CPU ██░░░░░░░░░░░░░ 11.1%   MEMORY █████████░░░░░░ 9.02 GiB / 14.81 GiB   PROCESSES 8 of 230
totals across processes: CPU 18.9% · memory 8.09 GiB
  PID │ NAME                                    │    CPU │ MEMORY │ USER         │ STATE    │   TIME
──────────────────────────────────────────────────────────────────────────────────────────────────
 5804 │ powershell.exe                          │   5.9% │  61.3M │ kolla        │ Running  │    0:05
 7800 │ procman.exe                             │   4.8% │  16.0M │ kolla        │ Running  │    0:02
 7680 │ explorer.exe                            │   4.0% │   291M │ kolla        │ Running  │ 2:43:13
16040 │ AMDRSServ.exe                           │   1.8% │   137M │ kolla        │ Running  │ 2:42:49
20784 │ opencode.exe                            │   0.9% │   283M │ kolla        │ Running  │ 1:32:48
```

## Features

| Command   | What it does                                                          |
|-----------|-----------------------------------------------------------------------|
| `list`    | process table with sorting, filters and a system summary header       |
| `watch`   | live monitor: refresh, scrolling, filtering, ending tasks from the keyboard |
| `info`    | process details: parent, chain, executable, working directory, command line |
| `tree`    | process tree (parent → children) with per-branch totals               |
| `kill`    | end processes by PID or name, with confirmation and a result report   |
| `summary` | the Performance tab: per-core CPU, memory, swap, disks, network       |

Also:

- **Honest CPU percentages.** Values are normalised to a share of all cores the
  way Task Manager counts them: a single-threaded load on a 16-core machine
  reports ~6% instead of 100%. The raw single-core value is shown as well
  (in `info` and in JSON).
- **JSON output** (`--json`) for `list`, `info`, `tree`, `kill` and `summary`,
  which makes scripting and monitoring easy.
- **Colour** turns on automatically when writing to a terminal and can be
  disabled with `--no-color` or the `NO_COLOR` variable; `--ascii` switches
  box drawing to plain ASCII.
- **No `unsafe` code** anywhere (`unsafe_code = "warn"`).
- Cross-platform: Windows, Linux and macOS (data is gathered with `sysinfo`).

## Install

Download a prebuilt archive from the
[latest release](https://github.com/kollakek1/procman/releases/latest):

| Platform | Archive |
|----------|---------|
| Windows x86_64 | `procman-<version>-x86_64-pc-windows-msvc.zip` |
| Linux x86_64 | `procman-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux aarch64 | `procman-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `procman-<version>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `procman-<version>-x86_64-apple-darwin.tar.gz` |

Each archive contains the binary plus `README.md`, `LICENSE` and `CHANGELOG.md`.
A `SHA256SUMS.txt` file with checksums is attached to every release.

From source (Rust 1.85 or newer):

```sh
git clone https://github.com/kollakek1/procman.git
cd procman
cargo install --path .
```

## Quick start

```sh
procman list                     # process list, sorted by CPU
procman list -n 15 --sort mem    # top 15 processes by memory
procman list -f chrome -c        # search by name and show command lines
procman watch                    # live monitor (press q to quit)
procman info 4321                # details about a process
procman tree --filter chrome     # process tree with a filter
procman kill notepad.exe -y      # end a task without confirmation
procman summary --top 10         # performance summary
```

See [docs/COMMANDS.md](docs/COMMANDS.md) for the full command reference.

## What it looks like

The process list with command lines and ASCII borders (`--ascii`):

```
  PID | NAME                |    CPU | MEMORY | USER         | STATE   |   TIME | COMMAND LINE
------------------------------------------------------------------------------------------------
 7964 | WindowsTerminal.exe |   0.5% |  97.3M | kolla        | Running | 1:28:25 | C:\Program Files\WindowsApps\Micro…
21196 | chrome.exe          |   0.3% |   487M | kolla        | Running | 1:35:03 | C:\Program Files\Google\Chrome\App…
```

Details about a process:

```
powershell.exe (PID 19768) Running
CPU █░░░░░░░░░░░░░░░ 6.25%   MEMORY ░░░░░░░░░░░░░░░░ 67.55 MiB (0.4% of RAM)

Process ID (PID)     │ 19768
Parent process       │ opencode.exe (PID 6648)
Process chain        │ powershell.exe (19768) ← opencode.exe (6648) ← …
State                │ Running
CPU usage            │ 6.25% of all cores (100.0% of one core)
Memory (RSS)         │ 67.55 MiB
Run time             │ 0:03
Started (UTC)        │ 2026-10-23 17:35:59 UTC (3 s ago)

Executable: C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe
Working directory: C:\Users\kolla\develop\projects\
Command line: C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.EXE -NoLogo …
```

The live monitor (`watch`) with load bars and a sparkline of CPU history:

```
procman — task manager   Windows 11 Pro for Workstations · kolla · uptime 1h 28m · cores 8/16
CPU     ███░░░░░░░░░░░░░░░░░░░░░░ 12.4%  ▁▂▃▅▂▇█  up to 2508 MHz
MEMORY  ██████████░░░░░░░░░░░░░░░ 42.1%  6.24 GiB / 14.81 GiB   swap 6.2%   available 8.07 GiB
LIST    CPU ↓   interval 1.0 s   filter: —   refreshes 12

↑/↓ select · s sort · r order · f filter · c reset · x end task · Enter details · space pause
```

Keys of the live monitor:

| Key                  | Action                                             |
|----------------------|----------------------------------------------------|
| `↑` / `↓`, `j` / `k` | move through the list                              |
| `Home` / `End`, `g` / `G` | jump to the top / bottom                      |
| `PageUp` / `PageDown`| move a page                                        |
| `s`                  | next sort key                                      |
| `r`                  | flip the sort direction                            |
| `f`                  | filter by name, PID or user                        |
| `c`                  | clear the filter                                   |
| `x` / `Delete`       | end the selected process (confirm with `y`/`n`)    |
| `Enter`              | panel with details about the process               |
| `space`              | pause / resume                                     |
| `+` / `-`            | increase / decrease the refresh interval           |
| `a`                  | show / hide command lines                          |
| `?`                  | help                                               |
| `q` / `Esc`          | quit                                               |

## JSON for scripts

```sh
procman list --json -n 5 | jq '.[] | {pid, name, cpu}'
procman summary --json | jq '.memory.used_percent'
procman kill 1234 -y --json | jq '.finished'
```

## Performance

The first snapshot is the expensive one:

| Command | Cold start (release build) |
|---------|----------------------------|
| `list`  | ~1.3 s |
| `summary`, `watch` | ~1.6 s (plus the sampling interval; `watch` then refreshes in ~20 ms) |
| `info`, `tree`, `kill` | ~0.6 s |

The reasons and the internals are described in
[docs/INTERNALS.md](docs/INTERNALS.md): the first touch of the Windows
performance counters costs ~0.6 s, and `sysinfo` needs three process samples to
compute honest CPU percentages.

Stage timings are available for diagnosis:

```sh
PROCMAN_TIMING=1 procman list -n 1
# [timing] init: 26 ms
# [timing] warm-up: 294 ms total
# [timing] snapshot: prep 0 ms, processes 12 ms, extra 12 ms
# [timing] snapshot build: 0 ms
```

## Development

```sh
cargo build                  # build
cargo test                   # 29 unit tests: formatting, tables, sorting,
                             # process tree, watch frames, key handling
cargo clippy --all-targets   # no warnings (all + pedantic)
cargo fmt
```

Layout:

```
src/
  main.rs        argument parsing (clap) and the subcommand dispatcher
  collector.rs   a wrapper over sysinfo: samples, warm-up, ending processes
  model.rs       the data model: Proc, Snapshot, HostInfo/CpuInfo/MemInfo/…
  cmd/           subcommands: list, watch, info, tree, kill, summary
  table.rs       table and line rendering that respects the visible width (Line)
  ansi.rs        minimal colouring (no dependencies)
  json.rs        JSON output (no serde)
  util.rs        sizes, time, bars, numbers
```

Dependencies: `clap` (arguments), `sysinfo` (system data), `crossterm` (the live
terminal mode), `anyhow` (errors). Colouring, JSON and tables are written by
hand — no `serde`, `colored` or `tabled`.

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs formatting, clippy
and the tests on Windows, Linux and macOS. Pushing a `v*` tag runs
[`.github/workflows/release.yml`](.github/workflows/release.yml), which builds
binaries for all supported targets and publishes them as a GitHub release.

## License

MIT — see [LICENSE](LICENSE).