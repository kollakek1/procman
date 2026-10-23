# Changelog

All notable changes to this project. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.2.0] — 2026-10-23

### Changed

- The entire project is now in English: the command output, `--help` texts,
  error messages, documentation, code comments and CI step names.
- Table numbers use comma separators (`1,234,567`) and byte units use the
  English names (`KiB`, `MiB`, `GiB`).
- Durations and uptimes read `1d 01:01:01` / `2d 5h 3m` instead of the Russian
  forms.

### Added

- A release workflow (`.github/workflows/release.yml`) that cross-compiles
  binaries for five targets and publishes them as a GitHub release on a `v*`
  tag: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`, `x86_64-apple-darwin`.
  Every archive ships with the binary, `README.md`, `LICENSE` and
  `CHANGELOG.md`, and a `SHA256SUMS.txt` file is attached to the release.
- `workflow_dispatch` support in the release workflow for building the full
  matrix without publishing.
- `docs/INTERNALS.md` documents the measured `sysinfo` behaviour that CPU
  percentages depend on.

### Notes

- The `list` and `summary` command lines are the slowest to start (~1.3 s and
  ~1.6 s on Windows) because of the first performance-counter call and the
  process warm-up; `info`, `tree` and `kill` finish in ~0.6 s.

## [0.1.0] — 2026-10-23

First working version: a console counterpart of the Windows Task Manager.

### Added

- `list` — the process table with sorting (CPU, memory, PID, name, user, run
  time, disk read and write), filters by text and user, an `-n` limit, optional
  command lines (`-c`) and a system summary header.
- `watch` — a live monitor: real-time refresh, CPU and memory bars, a sparkline
  of CPU history, scrolling and row selection, help and process-details panels,
  key bindings for sorting, filtering, ending tasks, pausing and changing the
  refresh interval.
- `info` — process details: state, user, parent and process chain, the number of
  child processes and of the whole branch, CPU as a share of all cores and of
  one core, resident and virtual memory, run time and start time, disk read and
  write, executable, working directory, command line.
- `tree` — the process tree with connector lines, per-branch aggregates (total
  CPU and memory, process count), substring filtering, a root selector and a
  depth limit; cycles and orphaned branches are handled.
- `kill` — ending processes by PID or name (several at once), with confirmation,
  a `--dry-run` mode, graceful termination (`--terminate`), ending children
  (`--with-children`) and a final report of what actually happened.
- `summary` — the performance summary: CPU model and frequency, total load and
  per-core load, memory and swap, disks with usage percentages, network with
  download and upload rates, and the busiest processes.
- JSON output (`--json`) for every command except the live monitor.
- Load-based colouring with terminal auto-detection, disabled by `--no-color` or
  `NO_COLOR`; an `--ascii` mode for box drawing.
- Table widths fitted to the terminal, with content truncation.
- CPU percentages normalised by the number of cores, the way Task Manager does.
- Stage diagnostics behind the `PROCMAN_TIMING=1` environment variable.
- 29 unit tests, a clean `cargo clippy --all-targets` (all + pedantic) and no
  `unsafe` code.

[0.2.0]: https://github.com/kollakek1/procman/releases/tag/v0.2.0
[0.1.0]: https://github.com/kollakek1/procman/releases/tag/v0.1.0