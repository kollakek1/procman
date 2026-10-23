//! Форматирование значений для терминала: размеры, время, полоски загрузки.
//!
//! Здесь нет внешних зависимостей, поэтому календарные функции считают дату
//! самостоятельно (алгоритм Хауарда Хиннанта «`civil_from_days`»), в UTC.

/// Количество «символов» строки (не байт) — важно для выравнивания таблиц
/// с кириллицей и рамками.
pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// Обрезает строку до `width` символов, добавляя многоточие.
pub fn truncate(text: &str, width: usize, ascii: bool) -> String {
    if width == 0 {
        return String::new();
    }
    let len = char_len(text);
    if len <= width {
        return text.to_owned();
    }
    let keep = width.saturating_sub(1);
    let suffix = if ascii { "~" } else { "…" };
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(suffix);
    out
}

/// Дополняет строку пробелами справа до `width` символов.
pub fn pad_right(text: &str, width: usize) -> String {
    let len = char_len(text);
    if len >= width {
        return text.to_owned();
    }
    format!("{text}{}", " ".repeat(width - len))
}

/// Дополняет строку пробелами слева до `width` символов.
pub fn pad_left(text: &str, width: usize) -> String {
    let len = char_len(text);
    if len >= width {
        return text.to_owned();
    }
    format!("{}{text}", " ".repeat(width - len))
}

/// Убирает управляющие символы (имена и командные строки процессов могут
/// содержать переводы строк и табуляции, которые ломают таблицу).
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_bytes(bytes: u64) -> (f64, usize) {
    const UNITS: [&str; 6] = ["Б", "КиБ", "МиБ", "ГиБ", "ТиБ", "ПиБ"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    (value, unit)
}

/// Компактный размер для таблиц: `512K`, `1.2G`, `0`.
///
/// Единицы латиницей — как в диспетчере задач (`512M`, `1.2G`).
pub fn bytes_short(bytes: u64) -> String {
    const SHORT: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes == 0 {
        return "0".to_owned();
    }
    let (value, unit) = split_bytes(bytes);
    let short = SHORT[unit];
    if unit == 0 {
        format!("{bytes}B")
    } else if value >= 100.0 {
        format!("{value:.0}{short}")
    } else if value >= 10.0 {
        format!("{value:.1}{short}")
    } else {
        format!("{value:.2}{short}")
    }
}

/// Подробный размер для сводок: `1.23 ГиБ`.
pub fn bytes_long(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["Б", "КиБ", "МиБ", "ГиБ", "ТиБ", "ПиБ"];
    let (value, unit) = split_bytes(bytes);
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Разделяет разряды пробелом: `1234567` -> `1 234 567`.
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0 && (digits.len() - idx) % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// Длительность в формате диспетчера задач: `0:42`, `1:02:03`, `3д 04:05:06`.
pub fn duration(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let seconds = secs % 60;
    if days > 0 {
        format!("{days}д {hours:02}:{minutes:02}:{seconds:02}")
    } else if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Человеческий аптайм: `2д 5ч 3мин`.
pub fn uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let parts = [
        (days, "д"),
        (hours, "ч"),
        (minutes, "мин"),
        (secs % 60, "с"),
    ];
    let text = parts
        .iter()
        .filter(|(value, _)| *value > 0)
        .take(3)
        .map(|(value, unit)| format!("{value}{unit}"))
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        "0с".to_owned()
    } else {
        text
    }
}

/// Русское согласование числительного: `1 процесс`, `2 процесса`, `5 процессов`.
pub fn plural(count: u64, one: &str, few: &str, many: &str) -> String {
    let mod100 = count % 100;
    let mod10 = count % 10;
    let form = if (11..=14).contains(&mod100) {
        many
    } else if mod10 == 1 {
        one
    } else if (2..=4).contains(&mod10) {
        few
    } else {
        many
    };
    format!("{count} {form}")
}

/// Полоска заполнения: `████░░░░`.
pub fn bar(percent: f32, width: usize, ascii: bool) -> String {
    let ratio = (percent / 100.0).clamp(0.0, 1.0);
    let filled = (ratio * width as f32).round() as usize;
    let filled = filled.min(width);
    let (full, empty) = if ascii { ('#', '-') } else { ('█', '░') };
    format!(
        "{}{}",
        full.to_string().repeat(filled),
        empty.to_string().repeat(width - filled)
    )
}

/// Спакрлайн из блоков разной высоты для истории загрузки.
pub fn sparkline(values: &[f32], ascii: bool) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const ASCII: [char; 8] = ['.', ':', '-', '=', '+', '*', '#', '@'];
    let glyphs = if ascii { &ASCII } else { &BLOCKS };
    values
        .iter()
        .map(|value| {
            let ratio = (value / 100.0).clamp(0.0, 1.0);
            let idx = (ratio * 7.0).round() as usize;
            glyphs[idx.min(7)]
        })
        .collect()
}

/// Дата и время из Unix-времени (UTC), формат `2026-10-23 14:05:33`.
pub fn epoch_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// Дни от 1970-01-01 -> (год, месяц, день). Алгоритм Хиннанта.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Текущее время в секундах Unix.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plural_forms() {
        assert_eq!(plural(1, "процесс", "процесса", "процессов"), "1 процесс");
        assert_eq!(plural(2, "процесс", "процесса", "процессов"), "2 процесса");
        assert_eq!(plural(5, "процесс", "процесса", "процессов"), "5 процессов");
        assert_eq!(
            plural(11, "процесс", "процесса", "процессов"),
            "11 процессов"
        );
        assert_eq!(plural(21, "процесс", "процесса", "процессов"), "21 процесс");
        assert_eq!(
            plural(112, "процесс", "процесса", "процессов"),
            "112 процессов"
        );
    }

    #[test]
    fn bytes_are_readable() {
        assert_eq!(bytes_short(0), "0");
        assert_eq!(bytes_short(1024), "1.00K");
        assert_eq!(bytes_short(1024 * 1024 * 512), "512M");
        assert_eq!(bytes_long(1024 * 1024 * 1024), "1.00 ГиБ");
    }

    #[test]
    fn thousand_separators() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1234), "1 234");
        assert_eq!(thousands(1_234_567_890), "1 234 567 890");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(5), "0:05");
        assert_eq!(duration(3_725), "1:02:05");
        assert_eq!(duration(90_061), "1д 01:01:01");
        assert_eq!(uptime(0), "0с");
        assert_eq!(uptime(90_061), "1д 1ч 1мин");
    }

    #[test]
    fn utc_dates() {
        assert_eq!(epoch_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(epoch_utc(1_000_000_000), "2001-09-09 01:46:40 UTC");
        assert_eq!(epoch_utc(1_774_000_000), "2026-03-20 09:46:40 UTC");
    }

    #[test]
    fn truncation_is_char_aware() {
        assert_eq!(truncate("привет", 10, false), "привет");
        assert_eq!(truncate("привет мир", 6, false), "приве…");
        assert_eq!(truncate("привет мир", 6, true), "приве~");
        assert_eq!(char_len("привет"), 6);
    }

    #[test]
    fn bars_and_sparklines() {
        assert_eq!(bar(50.0, 4, true), "##--");
        assert_eq!(bar(100.0, 3, true), "###");
        assert_eq!(bar(0.0, 3, false), "░░░");
        assert_eq!(sparkline(&[0.0, 100.0], true).chars().count(), 2);
    }
}
