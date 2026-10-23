# Internals and debugging notes

This document describes how `procman` is built and — more importantly — which
`sysinfo` quirks had to be measured on Windows, because honest CPU percentages
depend on them directly.

## Data flow

```
Collector::new()            cheap initialisation + counter warm-up
   │
   ├─ refresh(extended)     sample: CPU, memory, processes, [disks, networks]
   │
   └─ Snapshot              an immutable snapshot: host, cpu, mem, [disks, nets], procs
         │
         └─ commands        filtering, sorting, aggregates, rendering
               │
               ├─ table.rs  Line — a line made of styled text chunks
               ├─ ansi.rs   colouring (only when writing to a terminal)
               └─ json.rs   JSON output without serde
```

`Snapshot` is the single place where data from `sysinfo` turns into our own
structures (`Proc`, `HostInfo`, `CpuInfo`…). Commands only work with those, so
all rendering logic can be tested without a live system (see
`cmd/watch.rs::tests`, which builds a synthetic `Snapshot`).

## CPU percentages: three samples and normalisation

`sysinfo` computes a process load as the difference of counters between two
samples. In practice (Windows 11, `sysinfo` 0.39) that is not enough:

| Sample | Value for a core busy at 100% |
|--------|-------------------------------|
| first sample after the `System` is created | 0.0% |
| second sample, 200 ms later | 0.1% |
| third sample, another 200 ms later | **100.0%** |

`Collector::new` therefore performs a warm-up: a base sample, a pause, one more
sample whose result is discarded. The working sample comes from
`Collector::refresh` — that is the third one.

Further rules found by measurement:

- the pause between samples must be at least `MINIMUM_CPU_UPDATE_INTERVAL`
  (200 ms) — at 100 ms the values stay at zero; the code adds a 25 ms margin
  because the Windows scheduler ticks roughly every 15 ms;
- the interval is counted from the **moment of the sample**, not from the start
  of the previous one: the first sample is the slowest (it reads executable
  paths, command lines and owners) and eats the pause, which makes the following
  measurement too short. That is why `Collector::sample_processes` records the
  time right after sampling;
- all samples must request the same set of fields;
- `ProcessRefreshKind::with_cpu()` alone does not help here, while
  `with_exe/cmd/cwd/user` with `UpdateKind::OnlyIfNotSet` is free on later
  samples (the expensive fields are read once per process).

Normalisation: `sysinfo` reports a load in percent of **one core** (a
single-threaded load reports ~100%). `procman` divides it by the number of
logical cores — the way Task Manager counts it — so that the sum across
processes is comparable to the total load. The raw value is kept in
`Proc::cpu_core` and is visible in `info`:

```
CPU usage            │ 6.25% of all cores (100.0% of one core)
```

The global CPU percentage needs care as well:

- the very first sample always reports a bogus 100%, so `with_global_cpu()`
  takes a base point and the honest value is read in `refresh()`;
- **it must be read immediately** after `refresh_cpu_usage()`: the process
  sample that follows recomputes and overwrites the value;
- `System::load_average()` always returns zeros on Windows, yet its first call
  costs about 0.7 s — so it is not called there at all.

## Why a cold start costs about a second

The first touch of the CPU performance counters (`refresh_cpu_all` inside
`with_global_cpu`) takes **~0.63 s** on the test machine — a one-off price per
process, after which such calls are free. Commands that do not need the global
percentage (`info`, `tree`, `kill`) never ask for it and finish in ~0.6 s.

Cold start (release build):

| Command | Time | Made of |
|---------|------|---------|
| `tree`, `info`, `kill` | ~0.6 s | two 225 ms pauses + ~30 ms of calls + ~15 ms for networks and disks |
| `list` | ~1.3 s | the same plus ~0.63 s for the first CPU sample |
| `summary` | ~1.6 s | the same plus the network observation interval (1 s by default) |

Stage timings: `PROCMAN_TIMING=1 procman list -n 1` (printed to stderr).

Gathering process data is split into a cheap initialisation
(`System::new_with_specifics` with `CpuRefreshKind::nothing()` — zero
milliseconds) and separate expensive calls. `System::new_all()` is not used: it
reads the environment of every process and immediately performs an expensive CPU
sample.

## Table rendering

`table::Line` keeps a line as chunks of `(style, text)` and knows its own
**visible** width — which is why colouring is applied after alignment and never
breaks the padding. Width fitting:

1. natural column widths are computed, respecting `Col::min`/`Col::max`;
2. if the sum exceeds the terminal width, the widest column is shrunk (but never
   below `min`);
3. the leftover space is distributed across "elastic" columns (`Col::flexible` —
   usually the name and the command line);
4. content is truncated by visible characters with an ellipsis.

`Line::truncated`/`pad_to`/`trimmed` are used by `watch` too: the frame is
assembled from finished lines and written with a single `write`, which avoids
flicker. `table::tests` check that no line exceeds the width at 20/40/80/200
columns.

## Live mode

`watch` uses `crossterm`: raw mode, the alternate screen, a hidden cursor. The
terminal is restored by `Drop` on the `Terminal` wrapper — the screen is restored
both on errors and on an early exit. Keys are handled only for
`KeyEventKind::Press` events (Windows also delivers releases), and every frame is
fitted to the terminal height exactly: extra lines are dropped, missing ones are
padded.

Frame rendering and key handling are covered by tests that need no terminal:
frame geometry at different sizes, selection scrolling, an empty filter, overlay
panels and key-driven state transitions.

## Ending processes

`Collector::kill` sends `Signal::Term` when `--terminate` is used (if the
platform supports it), otherwise it calls `Process::kill()` — a forced
termination (`TerminateProcess` on Windows). Afterwards every PID is checked
again (`Collector::alive`) and three states are distinguished: "ended",
"ending", "denied". Failures produce exit code 1.

## Tests

```
cargo test
```

29 unit tests: size, time and number formatting, dates from Unix time, JSON,
table rendering and fitting, process sorting and filtering, target lookup,
the process tree (aggregates, orphan roots, cycles, connector lines) and the
live monitor (frames and keys). Tests that touch global flags (colour, ASCII)
are serialised with a mutex so that they do not influence each other when run in
parallel.

## CI and releases

- [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, the tests and
  a release build on Windows, Linux and macOS for every push to `main` and for
  pull requests.
- [`.github/workflows/release.yml`](../.github/workflows/release.yml) runs on a
  `v*` tag: it cross-compiles binaries for `x86_64-unknown-linux-gnu`,
  `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`
  and `x86_64-apple-darwin`, packs each into an archive together with the docs,
  writes `SHA256SUMS.txt` and publishes everything as a GitHub release.
  `workflow_dispatch` runs the same matrix without publishing, which is handy
  for testing.