//! Value formatting for the terminal: sizes, time, load bars.
//!
//! There are no external dependencies here, so the calendar helpers compute
//! dates themselves (Howard Hinnant's `civil_from_days` algorithm), in UTC.

/// Number of characters in a string (not bytes) — important for aligning
/// tables that contain wide characters and box drawing.
pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

/// Truncates a string to `width` characters, appending an ellipsis.
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

/// Pads a string with spaces on the right up to `width` characters.
pub fn pad_right(text: &str, width: usize) -> String {
    let len = char_len(text);
    if len >= width {
        return text.to_owned();
    }
    format!("{text}{}", " ".repeat(width - len))
}

/// Pads a string with spaces on the left up to `width` characters.
pub fn pad_left(text: &str, width: usize) -> String {
    let len = char_len(text);
    if len >= width {
        return text.to_owned();
    }
    format!("{}{text}", " ".repeat(width - len))
}

/// Removes control characters (process names and command lines may contain
/// newlines and tabs, which would break a table).
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_bytes(bytes: u64) -> (f64, usize) {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    (value, unit)
}

/// Compact size for tables: `512K`, `1.2G`, `0`.
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

/// Detailed size for summaries: `1.23 GiB`.
pub fn bytes_long(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let (value, unit) = split_bytes(bytes);
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Groups digits in threes: `1234567` -> `1,234,567`.
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0 && (digits.len() - idx) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Duration in the task manager style: `0:42`, `1:02:03`, `3d 04:05:06`.
pub fn duration(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let seconds = secs % 60;
    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}:{seconds:02}")
    } else if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Human readable uptime: `2d 5h 3m`.
pub fn uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    let parts = [(days, "d"), (hours, "h"), (minutes, "m"), (secs % 60, "s")];
    let text = parts
        .iter()
        .filter(|(value, _)| *value > 0)
        .take(3)
        .map(|(value, unit)| format!("{value}{unit}"))
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        "0s".to_owned()
    } else {
        text
    }
}

/// English plural agreement: `1 process`, `2 processes`.
pub fn plural(count: u64, singular: &str, plural: &str) -> String {
    let form = if count == 1 { singular } else { plural };
    format!("{count} {form}")
}

/// Fill bar: `████░░░░`.
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

/// Sparkline built from blocks of different height, for the load history.
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

/// Date and time from a Unix timestamp (UTC), formatted as `2026-10-23 14:05:33`.
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

/// Days since 1970-01-01 -> (year, month, day). Hinnant's algorithm.
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

/// Current time as a Unix timestamp.
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
        assert_eq!(plural(0, "process", "processes"), "0 processes");
        assert_eq!(plural(1, "process", "processes"), "1 process");
        assert_eq!(plural(2, "process", "processes"), "2 processes");
        assert_eq!(plural(21, "process", "processes"), "21 processes");
    }

    #[test]
    fn bytes_are_readable() {
        assert_eq!(bytes_short(0), "0");
        assert_eq!(bytes_short(1024), "1.00K");
        assert_eq!(bytes_short(1024 * 1024 * 512), "512M");
        assert_eq!(bytes_long(1024 * 1024 * 1024), "1.00 GiB");
    }

    #[test]
    fn thousand_separators() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(1_234_567_890), "1,234,567,890");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(5), "0:05");
        assert_eq!(duration(3_725), "1:02:05");
        assert_eq!(duration(90_061), "1d 01:01:01");
        assert_eq!(uptime(0), "0s");
        assert_eq!(uptime(90_061), "1d 1h 1m");
    }

    #[test]
    fn utc_dates() {
        assert_eq!(epoch_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(epoch_utc(1_000_000_000), "2001-09-09 01:46:40 UTC");
        assert_eq!(epoch_utc(1_774_000_000), "2026-03-20 09:46:40 UTC");
    }

    #[test]
    fn truncation_is_char_aware() {
        assert_eq!(truncate("hello", 10, false), "hello");
        assert_eq!(truncate("hello world", 6, false), "hello…");
        assert_eq!(truncate("hello world", 6, true), "hello~");
        assert_eq!(char_len("héllo"), 5);
    }

    #[test]
    fn bars_and_sparklines() {
        assert_eq!(bar(50.0, 4, true), "##--");
        assert_eq!(bar(100.0, 3, true), "###");
        assert_eq!(bar(0.0, 3, false), "░░░");
        assert_eq!(sparkline(&[0.0, 100.0], true).chars().count(), 2);
    }
}
