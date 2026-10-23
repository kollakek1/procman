//! Table and line rendering without external dependencies.
//!
//! The job of this module is to build aligned rows that fit the terminal
//! width: "elastic" columns (name, command line) shrink and grow, the rest
//! stay within the bounds given by `Col`.
//!
//! Colouring is applied AFTER alignment — otherwise ANSI escape sequences would
//! be counted as width. The primary output unit is therefore [`Line`]: it keeps
//! text as styled chunks and knows its own visible width.

use std::fmt::{self, Display};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ansi::{self, Code};
use crate::util::{char_len, pad_left, pad_right, truncate};

static ASCII: AtomicBool = AtomicBool::new(false);

/// Switches to ASCII borders and glyphs (for terminals without Unicode support).
pub fn set_ascii(value: bool) {
    ASCII.store(value, Ordering::Relaxed);
}

/// Whether ASCII borders are in use.
pub fn ascii() -> bool {
    ASCII.load(Ordering::Relaxed)
}

/// Terminal width. When the output is not a terminal (a pipe or a file), 120.
pub fn term_width() -> usize {
    crossterm::terminal::size()
        .map_or(120, |(cols, _)| cols as usize)
        .clamp(24, 400)
}

/// Terminal height. When it cannot be determined, 30 rows.
pub fn term_height() -> usize {
    crossterm::terminal::size()
        .map_or(30, |(_, rows)| rows as usize)
        .clamp(5, 300)
}

fn sep() -> &'static str {
    if ascii() { " | " } else { " │ " }
}

fn rule_char() -> char {
    if ascii() { '-' } else { '─' }
}

/// An output line: a set of styled text chunks.
#[derive(Clone, Debug, Default)]
pub struct Line {
    parts: Vec<(Option<Code>, String)>,
    reverse: bool,
}

impl Line {
    pub fn new() -> Self {
        Self::default()
    }

    /// A line without styling.
    pub fn plain(text: impl Into<String>) -> Self {
        Self::new().push(text)
    }

    /// A line made of a single styled chunk.
    pub fn colored(code: Code, text: impl Into<String>) -> Self {
        Self::new().styled(code, text)
    }

    /// Appends an unstyled text chunk.
    #[must_use]
    pub fn push(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        if !text.is_empty() {
            self.parts.push((None, text));
        }
        self
    }

    /// Appends a text chunk with the given colour.
    #[must_use]
    pub fn styled(mut self, code: Code, text: impl Into<String>) -> Self {
        let text = text.into();
        if !text.is_empty() {
            self.parts.push((Some(code), text));
        }
        self
    }

    /// Inverts the colours of the whole line (highlighting the selected row).
    #[must_use]
    pub fn reversed(mut self) -> Self {
        self.reverse = true;
        self
    }

    /// Visible width of the line (ANSI sequences excluded).
    pub fn visible(&self) -> usize {
        self.parts.iter().map(|(_, text)| char_len(text)).sum()
    }

    /// Truncates the line to `width` visible characters, adding an ellipsis.
    #[must_use]
    pub fn truncated(mut self, width: usize, ascii_borders: bool) -> Self {
        if self.visible() <= width {
            return self;
        }
        let reverse = self.reverse;
        let parts = std::mem::take(&mut self.parts);
        if width == 0 {
            return Self {
                parts: Vec::new(),
                reverse,
            };
        }
        let budget = width - 1;
        let mut used = 0;
        let mut kept: Vec<(Option<Code>, String)> = Vec::new();
        for (code, text) in parts {
            if used >= budget {
                break;
            }
            let len = char_len(&text);
            if used + len <= budget {
                used += len;
                kept.push((code, text));
            } else {
                let take = budget - used;
                kept.push((code, text.chars().take(take).collect()));
                break;
            }
        }
        kept.push((None, if ascii_borders { "~" } else { "…" }.to_owned()));
        Self {
            parts: kept,
            reverse,
        }
    }

    /// Pads the line with spaces up to `width` visible characters.
    #[must_use]
    pub fn pad_to(mut self, width: usize) -> Self {
        let visible = self.visible();
        if visible < width {
            self.parts.push((None, " ".repeat(width - visible)));
        }
        self
    }

    /// Removes trailing spaces, so that output written to a file has no padding.
    #[must_use]
    pub fn trimmed(mut self) -> Self {
        let mut index = self.parts.len();
        while index > 0 {
            index -= 1;
            let trimmed = self.parts[index].1.trim_end_matches(' ').to_owned();
            if trimmed.is_empty() {
                self.parts.remove(index);
                continue;
            }
            self.parts[index].1 = trimmed;
            break;
        }
        self
    }

    /// The finished text with ANSI codes.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (code, text) in &self.parts {
            match code {
                Some(code) => out.push_str(&ansi::paint(*code, text)),
                None => out.push_str(text),
            }
        }
        if self.reverse {
            ansi::reverse(&out)
        } else {
            out
        }
    }

    /// The text without ANSI codes.
    pub fn plain_text(&self) -> String {
        self.parts.iter().map(|(_, text)| text.as_str()).collect()
    }
}

impl Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

/// Alignment of a column's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A cell style — converted to an ANSI code when printed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Dim,
    Bold,
    Red,
    Green,
    Yellow,
    Cyan,
    Grey,
    BrightRed,
    BrightGreen,
}

impl Style {
    fn code(self) -> Option<Code> {
        match self {
            Self::Plain => None,
            Self::Dim => Some(Code::Dim),
            Self::Bold => Some(Code::Bold),
            Self::Red => Some(Code::Red),
            Self::Green => Some(Code::Green),
            Self::Yellow => Some(Code::Yellow),
            Self::Cyan => Some(Code::Cyan),
            Self::Grey => Some(Code::Grey),
            Self::BrightRed => Some(Code::BrightRed),
            Self::BrightGreen => Some(Code::BrightGreen),
        }
    }

    /// A style for a load level: green below 50%, yellow below 80%, red above.
    pub fn for_load(percent: f32) -> Self {
        if percent >= 80.0 {
            Self::BrightRed
        } else if percent >= 50.0 {
            Self::Yellow
        } else {
            Self::Green
        }
    }

    /// Applies the style to the text (a plain style returns it unchanged).
    pub fn paint(self, text: &str) -> String {
        match self.code() {
            Some(code) => ansi::paint(code, text),
            None => text.to_owned(),
        }
    }
}

/// A column description.
#[derive(Clone, Debug)]
pub struct Col {
    pub title: &'static str,
    pub align: Align,
    /// Minimum width (usually the width of the title).
    pub min: usize,
    /// Hard upper bound for the width.
    pub max: Option<usize>,
    /// Whether the column may give away or take width while fitting.
    pub flexible: bool,
}

impl Col {
    pub fn new(title: &'static str, align: Align, max: Option<usize>) -> Self {
        Self {
            title,
            align,
            min: char_len(title),
            max,
            flexible: false,
        }
    }

    #[must_use]
    pub fn flexible(mut self) -> Self {
        self.flexible = true;
        self
    }

    #[must_use]
    pub fn min_width(mut self, min: usize) -> Self {
        self.min = min.max(char_len(self.title));
        self
    }
}

/// A table cell.
#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    pub style: Style,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Style::Plain,
        }
    }

    pub fn styled(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    /// A percentage coloured automatically by its load level.
    pub fn load(text: impl Into<String>, percent: f32) -> Self {
        Self {
            text: text.into(),
            style: Style::for_load(percent),
        }
    }
}

/// A table: a set of columns and rows.
pub struct Table {
    cols: Vec<Col>,
    rows: Vec<Vec<Cell>>,
    header: bool,
    borders: bool,
}

impl Table {
    pub fn new(cols: Vec<Col>) -> Self {
        Self {
            cols,
            rows: Vec::new(),
            header: true,
            borders: true,
        }
    }

    /// A table without a header or borders (for example key — value pairs).
    pub fn plain(cols: Vec<Col>) -> Self {
        Self {
            cols,
            rows: Vec::new(),
            header: false,
            borders: false,
        }
    }

    pub fn push(&mut self, row: Vec<Cell>) {
        self.rows.push(row);
    }

    /// How many leading output lines the header and the rule occupy.
    pub fn header_lines(&self) -> usize {
        if self.header {
            1 + usize::from(self.borders)
        } else {
            0
        }
    }

    /// Like `render`, but without trailing spaces — for one-off output to a file.
    pub fn render_tight(&self, width: usize, selected: Option<usize>) -> Vec<Line> {
        self.render(width, selected)
            .into_iter()
            .map(Line::trimmed)
            .collect()
    }

    /// Renders into lines of the given width. `selected` is the highlighted row.
    pub fn render(&self, width: usize, selected: Option<usize>) -> Vec<Line> {
        let widths = self.fit(width);
        let mut lines = Vec::with_capacity(self.rows.len() + 3);
        if self.header {
            let mut head = Line::new();
            for (idx, col) in self.cols.iter().enumerate() {
                head = head.styled(Code::Bold, pad_cell(col.title, widths[idx], col.align));
                if idx + 1 < self.cols.len() {
                    head = head.push(sep());
                }
            }
            lines.push(head);
            if self.borders {
                let line_width = total(&widths) + sep_width(self.cols.len());
                lines.push(Line::plain(rule_char().to_string().repeat(line_width)));
            }
        }
        for (index, row) in self.rows.iter().enumerate() {
            let mut line = Line::new();
            for (idx, col) in self.cols.iter().enumerate() {
                let width = widths[idx];
                let raw = match row.get(idx) {
                    Some(cell) => pad_cell(&truncate(&cell.text, width, ascii()), width, col.align),
                    None => pad_cell("", width, col.align),
                };
                line = match row.get(idx).and_then(|cell| cell.style.code()) {
                    Some(code) => line.styled(code, raw),
                    None => line.push(raw),
                };
                if idx + 1 < self.cols.len() {
                    line = line.push(sep());
                }
            }
            if Some(index) == selected {
                line = line.reversed();
            }
            lines.push(line);
        }
        lines
    }

    /// Chooses column widths to fit the available space.
    fn fit(&self, width: usize) -> Vec<usize> {
        let count = self.cols.len();
        let separators = sep_width(count);
        let available = width.saturating_sub(separators).max(count);

        let mut widths = Vec::with_capacity(count);
        for (idx, col) in self.cols.iter().enumerate() {
            let mut natural = col.min;
            for row in &self.rows {
                if let Some(cell) = row.get(idx) {
                    natural = natural.max(char_len(&cell.text));
                }
            }
            if let Some(max) = col.max {
                natural = natural.min(max);
            }
            widths.push(natural.max(col.min));
        }

        // Shrink the widest columns until everything fits the screen width.
        let mut guard = 0;
        while total(&widths) + separators > width && guard < 4096 {
            guard += 1;
            let mut target: Option<usize> = None;
            for idx in 0..count {
                if widths[idx] > self.cols[idx].min
                    && target.is_none_or(|best| widths[idx] > widths[best])
                {
                    target = Some(idx);
                }
            }
            match target {
                Some(idx) => widths[idx] -= 1,
                None => break,
            }
        }

        // Give the leftover space to the "elastic" columns.
        let flexible: Vec<usize> = (0..count).filter(|idx| self.cols[*idx].flexible).collect();
        if !flexible.is_empty() {
            let mut cursor = 0usize;
            while total(&widths) + separators < available && cursor < 4096 {
                widths[flexible[cursor % flexible.len()]] += 1;
                cursor += 1;
            }
        }
        widths
    }
}

fn sep_width(count: usize) -> usize {
    sep().chars().count() * count.saturating_sub(1)
}

fn total(widths: &[usize]) -> usize {
    widths.iter().sum()
}

fn pad_cell(text: &str, width: usize, align: Align) -> String {
    match align {
        Align::Left => pad_right(text, width),
        Align::Right => pad_left(text, width),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests that touch global flags (ASCII mode, colours) run one at a time.
    static GLOBALS: Mutex<()> = Mutex::new(());

    fn globals() -> std::sync::MutexGuard<'static, ()> {
        GLOBALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn sample_table() -> Table {
        let cols = vec![
            Col::new("PID", Align::Right, Some(8)),
            Col::new("NAME", Align::Left, None).flexible(),
            Col::new("CPU", Align::Right, Some(7)),
        ];
        let mut table = Table::new(cols);
        table.push(vec![
            Cell::new("1234"),
            Cell::new("a-very-long-process-name"),
            Cell::new("12.3%"),
        ]);
        table.push(vec![Cell::new("7"), Cell::new("ls"), Cell::new("0.0%")]);
        table
    }

    #[test]
    fn renders_within_width() {
        for width in [20usize, 40, 80] {
            for line in sample_table().render(width, None) {
                assert!(
                    line.visible() <= width.max(20),
                    "line is wider than the terminal: {:?}",
                    line.plain_text()
                );
            }
        }
    }

    #[test]
    fn columns_are_aligned() {
        let _guard = globals();
        set_ascii(false);
        let lines = sample_table().render(80, None);
        let widths: Vec<usize> = lines.iter().map(Line::visible).collect();
        assert_eq!(widths[0], widths[1]);
        assert_eq!(widths[1], widths[2]);
        assert_eq!(widths[2], widths[3]);
    }

    #[test]
    fn long_cells_are_truncated() {
        let _guard = globals();
        set_ascii(false);
        let lines = sample_table().render(30, None);
        assert!(lines.iter().any(|l| l.plain_text().contains('…')));
        assert!(lines.iter().all(|l| l.visible() <= 30));
    }

    #[test]
    fn tight_render_has_no_trailing_spaces() {
        let _guard = globals();
        for line in sample_table().render_tight(80, None) {
            assert_eq!(line.plain_text(), line.plain_text().trim_end());
        }
    }

    #[test]
    fn ascii_mode_has_no_box_chars() {
        let _guard = globals();
        set_ascii(true);
        let lines = sample_table().render(60, None);
        assert!(lines.iter().all(|l| !l.text().contains('│')));
        set_ascii(false);
    }

    #[test]
    fn line_padding_and_styles() {
        let _guard = globals();
        ansi::set_enabled(false);
        let line = Line::plain("abc").pad_to(6);
        assert_eq!(line.visible(), 6);
        assert_eq!(line.text(), "abc   ");
        let styled = Line::colored(Code::Red, "abc").pad_to(5);
        assert_eq!(styled.visible(), 5);
        assert_eq!(styled.text(), "abc  ");
        let truncated = Line::plain("abcdefgh").truncated(5, false);
        assert_eq!(truncated.plain_text(), "abcd…");
        assert_eq!(truncated.visible(), 5);
    }
}
