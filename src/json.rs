//! Крошечный писатель JSON — чтобы не тянуть serde только ради `--json`.

use std::fmt::Display;
use std::fmt::Write as _;

/// Экранирует строку для JSON.
pub fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Строковое значение JSON.
pub fn string(value: &str) -> String {
    format!("\"{}\"", escape(value))
}

/// Числовое значение JSON.
pub fn number<T: Display>(value: T) -> String {
    value.to_string()
}

/// Число или `null`.
pub fn number_opt<T: Display>(value: Option<T>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "null".to_owned(),
    }
}

/// Строка или `null`.
pub fn string_opt(value: Option<&str>) -> String {
    match value {
        Some(v) => string(v),
        None => "null".to_owned(),
    }
}

/// Объект из пар «ключ — уже готовое JSON-значение».
pub fn object(fields: Vec<(&str, String)>) -> String {
    let body = fields
        .into_iter()
        .map(|(key, value)| format!("{}:{}", string(key), value))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

/// Массив из уже готовых JSON-значений.
pub fn array(items: Vec<String>) -> String {
    format!("[{}]", items.join(","))
}

/// Печатает JSON с отступом (по 2 пробела), чтобы вывод был читаемым.
pub fn pretty(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 32);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for ch in value.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '{' | '[' => {
                depth += 1;
                out.push(ch);
                out.push('\n');
                out.push_str(&"  ".repeat(depth));
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                out.push('\n');
                out.push_str(&"  ".repeat(depth));
                out.push(ch);
            }
            ',' => {
                out.push(ch);
                out.push('\n');
                out.push_str(&"  ".repeat(depth));
            }
            ':' => out.push_str(": "),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(string("ок"), "\"ок\"");
        assert_eq!(number_opt(Option::<u32>::None), "null");
    }

    #[test]
    fn pretty_print() {
        let raw = object(vec![("a", number(1)), ("b", array(vec![number(2)]))]);
        assert_eq!(pretty(&raw), "{\n  \"a\": 1,\n  \"b\": [\n    2\n  ]\n}");
    }
}
