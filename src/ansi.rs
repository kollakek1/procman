//! Minimal ANSI colouring without external dependencies.
//!
//! Colour is enabled automatically when `stdout` is a terminal and `NO_COLOR`
//! is not set. It can be forced on with the `CLICOLOR_FORCE` environment
//! variable or turned off with the `--no-color` flag.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables coloured output.
pub fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
}

/// Current colouring state.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Whether colour should be used: `NO_COLOR` unset and `stdout` is a terminal.
pub fn detect() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
        return true;
    }
    std::io::stdout().is_terminal()
}

/// ANSI codes used by the program.
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

/// Wraps text in an ANSI code (when colour is enabled).
pub fn paint(code: Code, text: &str) -> String {
    if enabled() {
        format!("\x1b[{}m{}\x1b[0m", code as u8, text)
    } else {
        text.to_owned()
    }
}

/// Inverts colours — used for the selected row in `watch`.
pub fn reverse(text: &str) -> String {
    if enabled() {
        format!("\x1b[7m{text}\x1b[0m")
    } else {
        text.to_owned()
    }
}

/// Colour for a load level: green below 50%, yellow below 80%, red above.
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
