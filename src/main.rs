//! procman — a task manager for your terminal (a Windows Task Manager counterpart).

mod ansi;
mod cmd;
mod collector;
mod json;
mod model;
mod table;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};

use cmd::info::InfoArgs;
use cmd::kill::KillArgs;
use cmd::list::ListArgs;
use cmd::summary::SummaryArgs;
use cmd::tree::TreeArgs;
use cmd::watch::WatchArgs;

#[derive(Parser, Debug)]
#[command(
    name = "procman",
    version,
    about = "procman — a task manager for your terminal",
    long_about = "procman is a console counterpart of the Windows Task Manager:\n\
                  process list, ending tasks, process tree,\n\
                  performance summary and a live monitor.",
    after_help = "EXAMPLES:\n  \
                  procman list                     process list (the Processes tab)\n  \
                  procman list -n 15 --sort mem    top 15 processes by memory\n  \
                  procman list -f chrome -c        search by name and show command lines\n  \
                  procman watch                    live monitor, press q to quit\n  \
                  procman info 4321                details about a process\n  \
                  procman tree --filter chrome     process tree with a filter\n  \
                  procman kill notepad.exe -y      end a task without confirmation\n  \
                  procman summary --top 10         performance summary",
    disable_help_subcommand = true
)]
struct Cli {
    /// Do not use colours (also disabled by the `NO_COLOR` environment variable)
    #[arg(long, global = true)]
    no_color: bool,
    /// ASCII borders instead of Unicode box drawing
    #[arg(long, global = true)]
    ascii: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List processes with sorting and filters
    #[command(alias = "ps", alias = "ls")]
    List(ListArgs),
    /// Live monitor refreshed in real time
    #[command(alias = "top", alias = "mon", alias = "live")]
    Watch(WatchArgs),
    /// Detailed information about a process (PID or name)
    Info(InfoArgs),
    /// Process tree (parent → children)
    #[command(alias = "pstree")]
    Tree(TreeArgs),
    /// End one or more processes by PID or name
    #[command(alias = "stop", alias = "taskkill")]
    Kill(KillArgs),
    /// System summary: CPU, memory, disks, network
    #[command(alias = "sys", alias = "perf")]
    Summary(SummaryArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    ansi::set_enabled(!cli.no_color && ansi::detect());
    table::set_ascii(cli.ascii);

    match cli.command {
        Command::List(args) => cmd::list::run(args),
        Command::Watch(args) => cmd::watch::run(args),
        Command::Info(args) => cmd::info::run(args),
        Command::Tree(args) => cmd::tree::run(args),
        Command::Kill(args) => cmd::kill::run(args),
        Command::Summary(args) => cmd::summary::run(args),
    }
}
