# Command reference

Global flags for every command: `--no-color` (turn colour off; the `NO_COLOR`
environment variable does the same) and `--ascii` (ASCII borders instead of
Unicode box drawing).

```
procman <COMMAND> [FLAGS]
```

Aliases: `ps`, `ls` → `list`; `top`, `mon`, `live` → `watch`; `pstree` → `tree`;
`stop`, `taskkill` → `kill`; `sys`, `perf` → `summary`.

---

## `procman list` — the process list

Aliases: `ps`, `ls`.

```
procman list [--sort <KEY>] [-n <N>] [-f <TEXT>] [-u <NAME>] [--asc] [-c] [--json] [--no-header]
```

| Flag | Description |
|------|-------------|
| `-s`, `--sort <KEY>` | sort key: `cpu` (default), `mem`, `pid`, `name`, `user`, `run`, `read`, `write` |
| `-n`, `--top <N>` | show only N processes |
| `-f`, `--filter <TEXT>` | filter by substring: name, command line, PID or user (case insensitive) |
| `-u`, `--user <NAME>` | only processes owned by this user |
| `--asc` | sort ascending (numeric keys are descending by default; `pid`, `name` and `user` are ascending) |
| `-c`, `--cmd` | add a column with the command line |
| `--json` | print JSON |
| `--no-header` | do not print the system summary header |

Examples:

```sh
procman list -n 15 --sort mem        # top 15 by memory
procman list -f chrome -c            # every chrome process with command lines
procman list -s run --no-header      # no header, sorted by run time
procman list --json -n 5             # JSON for scripts
```

The output starts with a header (OS, host, cores, uptime), load bars for CPU and
memory, a line with totals across processes and then the table itself.

---

## `procman watch` — the live monitor

Aliases: `top`, `mon`, `live`.

```
procman watch [--interval <SEC>] [--sort <KEY>] [--asc] [-n <N>] [-f <TEXT>] [-u <NAME>] [-c]
```

| Flag | Description |
|------|-------------|
| `-i`, `--interval <SEC>` | refresh interval (default 1.0, minimum 0.2) |
| `-s`, `--sort <KEY>` | initial sort key |
| `--asc` | initial sort direction |
| `-n`, `--top <N>` | how many processes to keep in the list (default 500) |
| `-f`, `--filter <TEXT>` | initial filter |
| `-u`, `--user <NAME>` | only processes of this user |
| `-c`, `--cmd` | show command lines |

Keys are listed in the [README](../README.md#what-it-looks-like). An interactive
terminal is required: when the output is a file or a pipe the command reports
this and exits with code 1 (use `list` for pipes).

---

## `procman info` — details about a process

```
procman info <TARGET> [--json]
```

`TARGET` is a PID (a number) or a process name. The lookup order is: PID →
exact name → name substring → command line or user substring. When several
processes match, the heaviest one is shown and the others are listed on stderr.

What it shows: state, owner, parent and process chain, the number of child
processes and of the whole branch, CPU usage (as a share of all cores and of one
core), resident and virtual memory, run time, start time (UTC), disk read and
write, executable, working directory and command line.

```sh
procman info 4321
procman info firefox --json
```

---

## `procman tree` — the process tree

Alias: `pstree`.

```
procman tree [-r <TARGET>] [-f <TEXT>] [-u <NAME>] [-s <KEY>] [--depth <N>] [--json]
```

| Flag | Description |
|------|-------------|
| `-r`, `--root <TARGET>` | show only the branch of this process (PID or name) |
| `-f`, `--filter <TEXT>` | keep only branches that contain the substring |
| `-u`, `--user <NAME>` | only processes of this user |
| `-s`, `--sort <KEY>` | sort sibling nodes (CPU by default) |
| `--depth <N>` | maximum depth (default 12) |
| `--json` | a nested structure in JSON |

Each line shows the name, PID, CPU, the total memory of the branch and how many
processes it contains. Roots are processes whose parent is not part of the
selection. Cycles (which do occur in real systems) do not cause an endless
recursion — they are marked instead.

```sh
procman tree --filter chrome
procman tree --root explorer
procman tree --depth 2 --root 1234 --json
```

---

## `procman kill` — ending processes

Aliases: `stop`, `taskkill`.

```
procman kill <TARGET> [<TARGET> ...] [-y] [--dry-run] [-t] [--with-children] [--json]
```

| Flag | Description |
|------|-------------|
| `-y`, `--yes` | do not ask for confirmation |
| `--dry-run` | only show what would be ended |
| `-t`, `--terminate` | graceful termination (SIGTERM); when the platform does not support it (Windows), a forced one is applied |
| `--with-children` | also end all child processes |
| `--json` | report in JSON (requires `-y`) |

Behaviour:

- targets are matched by PID, then by exact name, then by a name substring, and
  only then by command line or user;
- the list of targets is printed and a confirmation is asked (`y`/`yes`);
- procman's own process never enters the plan (a warning is printed);
- after the attempt every process is checked again: "ended" or "ending";
- if something could not be ended (rights), the command exits with code 1.

```sh
procman kill 4321                        # will ask for confirmation
procman kill notepad.exe -y              # every process with that name, no questions
procman kill 1234 --with-children --dry-run
procman kill explorer -t                 # graceful termination
```

---

## `procman summary` — performance

Aliases: `sys`, `perf`.

```
procman summary [--interval <SEC>] [--fast] [-n <N>] [--json]
```

| Flag | Description |
|------|-------------|
| `-i`, `--interval <SEC>` | how long to watch the network to compute rates (default 1.0) |
| `--fast` | do not wait for the interval (network rates will be zero) |
| `-n`, `--top <N>` | how many of the busiest processes to show (default 5) |
| `--json` | print JSON |

Sections: CPU (model, maximum frequency, total load and per-core load), memory
and swap, disks (size, used, free, percentage), network (received and sent
totals, download and upload rates, packets) and the busiest processes.

```sh
procman summary
procman summary --fast --top 10
procman summary --json | jq '.disks[] | {mount, used_percent}'
```

---

## Exit codes

| Code | When |
|------|------|
| 0 | success (including an empty filter result) |
| 1 | an error: a process was not found, termination was refused, `watch` without a terminal |
| 2 | argument parsing failed (handled by clap) |