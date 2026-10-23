//! procman — диспетчер задач в терминале (аналог Windows Task Manager).

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
    about = "procman — диспетчер задач в терминале",
    long_about = "procman — консольный аналог Диспетчера задач Windows:\n\
                  список процессов, завершение задач, дерево процессов,\n\
                  сводка по производительности и живой монитор.",
    after_help = "ПРИМЕРЫ:\n  \
                  procman list                     список процессов (как вкладка «Процессы»)\n  \
                  procman list -n 15 --sort mem    топ-15 процессов по памяти\n  \
                  procman list -f chrome -c        поиск по имени и показ командных строк\n  \
                  procman watch                    живой монитор, выход по q\n  \
                  procman info 4321                подробности о процессе\n  \
                  procman tree --filter chrome     дерево процессов c фильтром\n  \
                  procman kill notepad.exe -y      снять задачу без подтверждения\n  \
                  procman summary --top 10         сводка по производительности",
    disable_help_subcommand = true
)]
struct Cli {
    /// Не использовать цветной вывод (цвет также отключается переменной `NO_COLOR`)
    #[arg(long, global = true)]
    no_color: bool,
    /// ASCII-рамки вместо Unicode-псевдографики
    #[arg(long, global = true)]
    ascii: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Список процессов с сортировкой и фильтрами
    #[command(alias = "ps", alias = "ls")]
    List(ListArgs),
    /// Живой монитор с обновлением в реальном времени
    #[command(alias = "top", alias = "mon", alias = "live")]
    Watch(WatchArgs),
    /// Подробная информация о процессе (PID или имя)
    Info(InfoArgs),
    /// Дерево процессов «родитель → потомки»
    #[command(alias = "pstree")]
    Tree(TreeArgs),
    /// Завершить процесс(ы) по PID или имени
    #[command(alias = "stop", alias = "taskkill")]
    Kill(KillArgs),
    /// Сводка по системе: CPU, память, диски, сеть
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
