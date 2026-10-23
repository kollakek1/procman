//! `procman tree` — дерево процессов «родитель → потомки».

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use anyhow::Result;
use clap::Args;

use crate::ansi;
use crate::cmd::{compare_procs, filter_procs, resolve_one};
use crate::collector::Collector;
use crate::json;
use crate::model::{Proc, SortBy};
use crate::util::{bytes_short, sanitize, thousands};

#[derive(Args, Debug)]
pub struct TreeArgs {
    /// Показать только ветку этого процесса (PID или имя)
    #[arg(short, long, value_name = "ЦЕЛЬ")]
    pub root: Option<String>,
    /// Оставить только ветки, в которых встречается подстрока
    #[arg(short, long, value_name = "ТЕКСТ")]
    pub filter: Option<String>,
    /// Только процессы указанного пользователя
    #[arg(short, long, value_name = "ИМЯ")]
    pub user: Option<String>,
    /// Сортировка соседних узлов
    #[arg(short, long, value_enum, default_value_t = SortBy::Cpu)]
    pub sort: SortBy,
    /// Максимальная глубина дерева
    #[arg(long, default_value_t = 12)]
    pub depth: usize,
    /// Вывести данные в формате JSON (вложенная структура)
    #[arg(long)]
    pub json: bool,
}

/// Суммарные показатели ветки.
#[derive(Clone, Copy, Debug, Default)]
struct Agg {
    cpu: f32,
    mem: u64,
    count: usize,
}

/// Индекс «родитель → дети» плюс агрегаты по ветвям.
struct Forest<'a> {
    procs: &'a [Proc],
    pids: HashSet<u32>,
    children: HashMap<u32, Vec<usize>>,
}

impl<'a> Forest<'a> {
    fn build(procs: &'a [Proc]) -> Self {
        let pids: HashSet<u32> = procs.iter().map(|p| p.pid).collect();
        let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
        for (position, proc_) in procs.iter().enumerate() {
            if let Some(parent) = proc_.ppid {
                if parent != proc_.pid && pids.contains(&parent) {
                    children.entry(parent).or_default().push(position);
                }
            }
        }
        Self {
            procs,
            pids,
            children,
        }
    }

    fn sort_children(&mut self, by: SortBy) {
        let procs = self.procs;
        for kids in self.children.values_mut() {
            kids.sort_by(|left, right| compare_procs(&procs[*left], &procs[*right], by, false));
        }
    }

    fn kids_of(&self, pid: u32) -> &[usize] {
        self.children.get(&pid).map_or(&[], Vec::as_slice)
    }

    /// Корни — процессы, чей родитель отсутствует в выборке.
    fn roots(&self) -> Vec<usize> {
        let roots: Vec<usize> = (0..self.procs.len())
            .filter(|idx| {
                let proc_ = &self.procs[*idx];
                match proc_.ppid {
                    None => true,
                    Some(parent) if parent == proc_.pid => true,
                    Some(parent) => !self.pids.contains(&parent),
                }
            })
            .collect();
        if roots.is_empty() {
            (0..self.procs.len()).collect()
        } else {
            roots
        }
    }

    /// Есть ли в ветке процесс, подходящий под фильтр.
    fn subtree_matches(&self, idx: usize, needle: &str, memo: &mut HashMap<u32, bool>) -> bool {
        let pid = self.procs[idx].pid;
        if let Some(cached) = memo.get(&pid) {
            return *cached;
        }
        let mut matched = self.procs[idx].matches(needle);
        if !matched {
            // Предварительное значение защищает от циклов в дереве.
            memo.insert(pid, false);
            for &kid in self.kids_of(pid) {
                if self.subtree_matches(kid, needle, memo) {
                    matched = true;
                    break;
                }
            }
        }
        memo.insert(pid, matched);
        matched
    }

    /// Агрегаты по ветвям (защита от циклов через предварительную запись в memo).
    fn aggregates(&self) -> HashMap<u32, Agg> {
        let mut memo = HashMap::new();
        for idx in 0..self.procs.len() {
            self.aggregate_of(idx, &mut memo);
        }
        memo
    }

    fn aggregate_of(&self, idx: usize, memo: &mut HashMap<u32, Agg>) -> Agg {
        let proc_ = &self.procs[idx];
        if let Some(cached) = memo.get(&proc_.pid) {
            return *cached;
        }
        let mut agg = Agg {
            cpu: proc_.cpu,
            mem: proc_.mem,
            count: 1,
        };
        memo.insert(proc_.pid, agg);
        for &kid in self.kids_of(proc_.pid) {
            let child = self.aggregate_of(kid, memo);
            agg.cpu += child.cpu;
            agg.mem += child.mem;
            agg.count += child.count;
        }
        memo.insert(proc_.pid, agg);
        agg
    }
}

/// Построитель текстового дерева.
struct Renderer<'a> {
    forest: &'a Forest<'a>,
    aggregates: HashMap<u32, Agg>,
    total_mem: u64,
    needle: Option<String>,
    match_memo: HashMap<u32, bool>,
    visited: HashSet<u32>,
    lines: Vec<String>,
    max_depth: usize,
}

impl Renderer<'_> {
    fn node(&mut self, idx: usize, prefix: &str, is_last: bool, depth: usize, top_level: bool) {
        if depth > self.max_depth {
            self.lines
                .push(format!("{prefix}{}", ansi::dim("… ветка скрыта (--depth)")));
            return;
        }
        let pid = self.forest.procs[idx].pid;
        if !self.visited.insert(pid) {
            self.lines.push(format!(
                "{prefix}{}",
                ansi::red("⟲ цикл в дереве процессов")
            ));
            return;
        }
        let needle = self.needle.clone();
        if let Some(needle) = needle.as_deref() {
            if !self
                .forest
                .subtree_matches(idx, needle, &mut self.match_memo)
            {
                self.visited.remove(&pid);
                return;
            }
        }

        let (connector, child_prefix) = if top_level {
            (String::new(), String::new())
        } else if is_last {
            ("└─ ".to_owned(), format!("{prefix}   "))
        } else {
            ("├─ ".to_owned(), format!("{prefix}│  "))
        };

        let proc_ = &self.forest.procs[idx];
        let agg = self.aggregates.get(&pid).copied().unwrap_or_default();
        let mem_percent = if self.total_mem == 0 {
            0.0
        } else {
            (agg.mem as f64 / self.total_mem as f64 * 100.0) as f32
        };
        let mut line = format!("{prefix}{connector}");
        line.push_str(&ansi::bold(&sanitize(&proc_.name)));
        line.push_str(&ansi::dim(&format!(" ({})", proc_.pid)));
        let _ = write!(
            line,
            "  CPU {}  ПАМ {}",
            ansi::paint(ansi::load_code(proc_.cpu), &format!("{:>6.2}%", proc_.cpu)),
            ansi::paint(
                ansi::load_code(mem_percent),
                &format!("{:>6}", bytes_short(agg.mem))
            )
        );
        if agg.count > 1 {
            line.push_str(&ansi::grey(&format!(
                "  ветка: {}",
                thousands(agg.count as u64)
            )));
        }
        self.lines.push(line);

        let kids: Vec<usize> = {
            let needle = self.needle.clone();
            let mut memo = std::mem::take(&mut self.match_memo);
            let filtered = self
                .forest
                .kids_of(pid)
                .iter()
                .copied()
                .filter(|kid| {
                    needle
                        .as_deref()
                        .is_none_or(|needle| self.forest.subtree_matches(*kid, needle, &mut memo))
                })
                .collect();
            self.match_memo = memo;
            filtered
        };
        let last = kids.len().saturating_sub(1);
        for (position, kid) in kids.iter().enumerate() {
            self.node(*kid, &child_prefix, position == last, depth + 1, false);
        }
        self.visited.remove(&pid);
    }
}

fn json_node(
    forest: &Forest<'_>,
    aggregates: &HashMap<u32, Agg>,
    idx: usize,
    depth: usize,
) -> String {
    let proc_ = &forest.procs[idx];
    let agg = aggregates.get(&proc_.pid).copied().unwrap_or_default();
    let kids: Vec<String> = if depth >= 10 {
        Vec::new()
    } else {
        forest
            .kids_of(proc_.pid)
            .iter()
            .map(|kid| json_node(forest, aggregates, *kid, depth + 1))
            .collect()
    };
    json::object(vec![
        ("pid", json::number(proc_.pid)),
        ("ppid", json::number_opt(proc_.ppid)),
        ("name", json::string(&proc_.name)),
        ("cpu", json::number(format!("{:.1}", proc_.cpu))),
        ("memory_bytes", json::number(proc_.mem)),
        ("tree_cpu", json::number(format!("{:.1}", agg.cpu))),
        ("tree_memory_bytes", json::number(agg.mem)),
        ("tree_processes", json::number(agg.count)),
        ("children", json::array(kids)),
    ])
}

/// Выполняет подкоманду `tree`.
pub fn run(args: TreeArgs) -> Result<()> {
    let mut collector = Collector::new();
    let snapshot = collector.refresh(false);

    let mut procs = snapshot.procs.clone();
    if args.user.is_some() {
        filter_procs(&mut procs, None, args.user.as_deref());
    }

    let needle = args
        .filter
        .as_ref()
        .map(|text| text.trim().to_lowercase())
        .filter(|text| !text.is_empty());

    let mut forest = Forest::build(&procs);
    forest.sort_children(args.sort);
    let aggregates = forest.aggregates();

    let root_indices: Vec<usize> = match args.root.as_deref() {
        Some(target) => {
            let found = resolve_one(&snapshot, target)?;
            procs
                .iter()
                .position(|p| p.pid == found.pid)
                .into_iter()
                .collect()
        }
        None => forest.roots(),
    };

    if args.json {
        let items = root_indices
            .iter()
            .map(|idx| json_node(&forest, &aggregates, *idx, 0))
            .collect();
        println!("{}", json::pretty(&json::array(items)));
        return Ok(());
    }

    println!(
        "{} {}",
        ansi::bold("ДЕРЕВО ПРОЦЕССОВ"),
        ansi::dim(&format!(
            "· всего: {} · корней: {}{}",
            thousands(procs.len() as u64),
            thousands(root_indices.len() as u64),
            args.filter
                .as_ref()
                .map_or_else(String::new, |text| format!(" · фильтр: {text}"))
        ))
    );

    let mut renderer = Renderer {
        forest: &forest,
        aggregates,
        total_mem: snapshot.mem.total,
        needle,
        match_memo: HashMap::new(),
        visited: HashSet::new(),
        lines: Vec::new(),
        max_depth: args.depth,
    };
    for idx in &root_indices {
        renderer.node(*idx, "", true, 0, true);
    }
    if renderer.lines.is_empty() {
        println!("{}", ansi::yellow("Ничего не найдено — проверьте фильтр."));
        return Ok(());
    }
    for line in renderer.lines {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Proc;

    fn proc_(pid: u32, name: &str, cpu: f32, mem: u64, ppid: Option<u32>) -> Proc {
        Proc {
            pid,
            ppid,
            name: name.to_owned(),
            cpu,
            cpu_core: cpu,
            mem,
            virt: 0,
            status: "Run".to_owned(),
            user: "tester".to_owned(),
            exe: None,
            cwd: None,
            cmd: String::new(),
            start_time: 0,
            run_time: 0,
            read_bytes: 0,
            written_bytes: 0,
            children: 0,
        }
    }

    #[test]
    fn aggregates_include_descendants() {
        let procs = vec![
            proc_(1, "root", 1.0, 100, None),
            proc_(2, "child", 2.0, 200, Some(1)),
            proc_(3, "grand", 3.0, 300, Some(2)),
        ];
        let forest = Forest::build(&procs);
        let agg = forest.aggregates();
        let root = agg.get(&1).copied().unwrap();
        assert_eq!(root.count, 3);
        assert_eq!(root.mem, 600);
        assert!((root.cpu - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn roots_detects_orphans_and_self_parents() {
        let procs = vec![
            proc_(1, "root", 0.0, 0, None),
            proc_(2, "child", 0.0, 0, Some(1)),
            proc_(3, "orphan", 0.0, 0, Some(999)),
            proc_(4, "self", 0.0, 0, Some(4)),
        ];
        let forest = Forest::build(&procs);
        let mut roots = forest.roots();
        roots.sort_unstable();
        assert_eq!(roots, vec![0, 2, 3]);
    }

    #[test]
    fn filter_keeps_matching_subtrees() {
        let procs = vec![
            proc_(1, "root", 0.0, 0, None),
            proc_(2, "chrome.exe", 0.0, 0, Some(1)),
            proc_(3, "notepad.exe", 0.0, 0, Some(1)),
        ];
        let forest = Forest::build(&procs);
        let mut memo = HashMap::new();
        assert!(forest.subtree_matches(0, "chrome", &mut memo));
        assert!(!forest.subtree_matches(2, "chrome", &mut memo));
    }

    #[test]
    fn children_are_drawn_with_connectors() {
        let procs = vec![
            proc_(1, "root", 1.0, 100, None),
            proc_(2, "child-a", 1.0, 100, Some(1)),
            proc_(3, "child-b", 1.0, 100, Some(1)),
            proc_(4, "grand", 1.0, 100, Some(2)),
        ];
        let forest = Forest::build(&procs);
        let aggregates = forest.aggregates();
        let mut renderer = Renderer {
            forest: &forest,
            aggregates,
            total_mem: 1024,
            needle: None,
            match_memo: HashMap::new(),
            visited: HashSet::new(),
            lines: Vec::new(),
            max_depth: 5,
        };
        renderer.node(0, "", true, 0, true);
        let text = renderer.lines.join("\n");
        assert!(text.contains("root (1)"), "{text}");
        assert!(text.contains("├─ child-a (2)"), "{text}");
        assert!(text.contains("└─ child-b (3)"), "{text}");
        assert!(text.contains("│  └─ grand (4)"), "{text}");
    }

    #[test]
    fn cycles_do_not_hang() {
        let procs = vec![
            proc_(1, "a", 0.0, 10, Some(2)),
            proc_(2, "b", 0.0, 10, Some(1)),
        ];
        let forest = Forest::build(&procs);
        let agg = forest.aggregates();
        assert!(agg.contains_key(&1));
        assert!(agg.contains_key(&2));
    }
}
