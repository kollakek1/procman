//! Подкоманды CLI.

pub mod info;
pub mod kill;
pub mod list;
pub mod summary;
pub mod tree;
pub mod watch;

use std::cmp::Ordering;

use anyhow::{Result, bail};

use crate::model::{Proc, Snapshot, SortBy};

/// Сравнивает два процесса по выбранному ключу (с выравниванием порядка).
pub fn compare_procs(a: &Proc, b: &Proc, by: SortBy, ascending: bool) -> Ordering {
    let order = match by {
        SortBy::Cpu => a.cpu.partial_cmp(&b.cpu).unwrap_or(Ordering::Equal),
        SortBy::Mem => a.mem.cmp(&b.mem),
        SortBy::Pid => a.pid.cmp(&b.pid),
        SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortBy::User => a.user.to_lowercase().cmp(&b.user.to_lowercase()),
        SortBy::Run => a.run_time.cmp(&b.run_time),
        SortBy::Read => a.read_bytes.cmp(&b.read_bytes),
        SortBy::Write => a.written_bytes.cmp(&b.written_bytes),
    };
    // Стабильный вторичный ключ: по памяти, затем по PID.
    let order = order
        .then_with(|| a.mem.cmp(&b.mem))
        .then_with(|| a.pid.cmp(&b.pid));
    if ascending { order } else { order.reverse() }
}

/// Сортирует процессы. `ascending = false` — по убыванию (как в диспетчере задач).
pub fn sort_procs(procs: &mut [Proc], by: SortBy, ascending: bool) {
    procs.sort_by(|a, b| compare_procs(a, b, by, ascending));
}

/// Фильтрует процессы по подстроке и/или пользователю.
pub fn filter_procs(procs: &mut Vec<Proc>, needle: Option<&str>, user: Option<&str>) {
    if let Some(needle) = needle {
        let needle = needle.trim().to_lowercase();
        if !needle.is_empty() {
            procs.retain(|p| p.matches(&needle));
        }
    }
    if let Some(user) = user {
        let user = user.trim().to_lowercase();
        if !user.is_empty() {
            procs.retain(|p| p.user.to_lowercase().contains(&user));
        }
    }
}

/// Ищет процессы по строке-цели: сначала как PID, затем как точное имя,
/// затем по подстроке в имени и лишь в последнюю очередь — по командной строке,
/// пользователю или номеру PID. Такой порядок не даёт случайно поймать процесс,
/// в командной строке которого просто упомянуто искомое слово.
pub fn resolve_targets(snapshot: &Snapshot, target: &str) -> Vec<Proc> {
    let trimmed = target.trim();
    if let Ok(pid) = trimmed.parse::<u32>() {
        return snapshot.by_pid(pid).cloned().into_iter().collect();
    }

    let exact: Vec<Proc> = snapshot.by_name(trimmed).into_iter().cloned().collect();
    if !exact.is_empty() {
        return exact;
    }

    let needle = trimmed.to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let by_name: Vec<Proc> = snapshot
        .procs
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    if !by_name.is_empty() {
        return by_name;
    }

    snapshot
        .procs
        .iter()
        .filter(|p| p.matches(&needle))
        .cloned()
        .collect()
}

/// Разбирает строку-цель, требуя ровно одно совпадение (для `info`).
pub fn resolve_one(snapshot: &Snapshot, target: &str) -> Result<Proc> {
    let mut found = resolve_targets(snapshot, target);
    if found.is_empty() {
        bail!("процесс «{target}» не найден");
    }
    found.sort_by_key(|p| p.pid);
    if found.len() == 1 {
        return Ok(found.remove(0));
    }
    // Если совпадение неоднозначно, предпочитаем самый «тяжёлый» процесс.
    found.sort_by_key(|proc_| std::cmp::Reverse(proc_.mem));
    let chosen = found.remove(0);
    let others: Vec<String> = found
        .iter()
        .take(5)
        .map(|p| format!("{} (PID {})", p.name, p.pid))
        .collect();
    eprintln!(
        "Найдено несколько совпадений, показан самый крупный: {} (PID {}).",
        chosen.name, chosen.pid
    );
    eprintln!("Другие совпадения: {}", others.join(", "));
    Ok(chosen)
}

/// Собирает PID процесса и всех его потомков (рекурсивно).
pub fn collect_descendants(snapshot: &Snapshot, root: u32) -> Vec<u32> {
    let mut result = vec![root];
    let mut cursor = 0;
    while cursor < result.len() {
        let parent = result[cursor];
        cursor += 1;
        for proc_ in &snapshot.procs {
            if proc_.ppid == Some(parent) && !result.contains(&proc_.pid) {
                result.push(proc_.pid);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
            cmd: format!("{name} --run"),
            start_time: 0,
            run_time: 10,
            read_bytes: 0,
            written_bytes: 0,
            children: 0,
        }
    }

    fn snapshot(procs: Vec<Proc>) -> Snapshot {
        Snapshot {
            procs,
            ..Snapshot::default()
        }
    }

    #[test]
    fn sorting_by_cpu_defaults_to_descending() {
        let mut procs = vec![
            proc_(1, "a", 1.0, 10, None),
            proc_(2, "b", 5.0, 20, None),
            proc_(3, "c", 3.0, 30, None),
        ];
        sort_procs(&mut procs, SortBy::Cpu, false);
        assert_eq!(
            procs.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
    }

    #[test]
    fn filtering_matches_name_and_pid() {
        let mut procs = vec![
            proc_(11, "chrome.exe", 0.0, 0, None),
            proc_(22, "notepad.exe", 0.0, 0, None),
        ];
        filter_procs(&mut procs, Some("chrome"), None);
        assert_eq!(procs.len(), 1);
        let mut procs = vec![
            proc_(11, "chrome.exe", 0.0, 0, None),
            proc_(22, "notepad.exe", 0.0, 0, None),
        ];
        filter_procs(&mut procs, Some("22"), None);
        assert_eq!(procs[0].pid, 22);
    }

    #[test]
    fn target_resolution_prefers_pid_then_exact_name() {
        let snap = snapshot(vec![
            proc_(42, "svc", 0.0, 0, None),
            proc_(7, "svc-extra", 0.0, 0, None),
        ]);
        assert_eq!(resolve_targets(&snap, "42")[0].pid, 42);
        assert_eq!(resolve_targets(&snap, "svc")[0].pid, 42);
        assert_eq!(resolve_targets(&snap, "extra")[0].pid, 7);
        assert!(resolve_targets(&snap, "нет-такого").is_empty());
    }

    #[test]
    fn descendants_are_collected_recursively() {
        let snap = snapshot(vec![
            proc_(1, "root", 0.0, 0, None),
            proc_(2, "child", 0.0, 0, Some(1)),
            proc_(3, "grandchild", 0.0, 0, Some(2)),
            proc_(4, "stranger", 0.0, 0, None),
        ]);
        let mut ids = collect_descendants(&snap, 1);
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3]);
    }
}
