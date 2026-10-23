//! Минимальная ANSI-раскраска без внешних зависимостей.
//!
//! Цвет включается автоматически, если `stdout` — терминал и не задан `NO_COLOR`.
//! Принудительно включить можно переменной `CLICOLOR_FORCE` или флагом `--no-color`
//! (выключает).

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Включает или выключает цветной вывод.
pub fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
}

/// Текущее состояние раскраски.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Стоит ли включать цвет: `NO_COLOR` не задан и `stdout` подключён к терминалу.
pub fn detect() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
        return true;
    }
    std::io::stdout().is_terminal()
}

/// Коды ANSI, которые использует программа.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Bold = 1,
    Dim = 2,
    Red = 31,
    Green = 32,
    Yellow = 33,
    Cyan = 36,
    Grey = 90,
    BrightRed = 91,
    BrightGreen = 92,
    BrightYellow = 93,
}

/// Оборачивает текст ANSI-кодом (если цвет включён).
pub fn paint(code: Code, text: &str) -> String {
    if enabled() {
        format!("\x1b[{}m{}\x1b[0m", code as u8, text)
    } else {
        text.to_owned()
    }
}

/// Инвертирует цвета — используется для выделенной строки в `watch`.
pub fn reverse(text: &str) -> String {
    if enabled() {
        format!("\x1b[7m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

/// Цвет для уровня нагрузки: зелёный < 50%, жёлтый < 80%, иначе красный.
pub fn load_code(percent: f32) -> Code {
    if percent >= 80.0 {
        Code::BrightRed
    } else if percent >= 50.0 {
        Code::Yellow
    } else {
        Code::Green
    }
}

pub fn bold(text: &str) -> String {
    paint(Code::Bold, text)
}

pub fn dim(text: &str) -> String {
    paint(Code::Dim, text)
}

pub fn red(text: &str) -> String {
    paint(Code::Red, text)
}

pub fn yellow(text: &str) -> String {
    paint(Code::Yellow, text)
}

pub fn grey(text: &str) -> String {
    paint(Code::Grey, text)
}
