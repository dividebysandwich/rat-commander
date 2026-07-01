//! Search / replace dialog (editor).

use super::widgets::*;
use super::{DialogResult, Submit};

// ---------------------------------------------------------------------------
// Search / replace dialog (editor)
// ---------------------------------------------------------------------------

/// Result of the editor search/replace dialog.
#[derive(Debug, Clone)]
pub struct SearchReplaceParams {
    pub replace: bool,
    pub search: String,
    pub replacement: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_words: bool,
    pub backwards: bool,
    /// Hex mode was selected: search/replacement are hex byte strings.
    pub hex: bool,
}

pub struct SearchReplaceDialog {
    pub(crate) replace: bool,
    search: String,
    search_cursor: usize,
    replacement: String,
    repl_cursor: usize,
    mode: usize, // 0 Normal, 1 Regex, 2 Hex, 3 Wildcard
    case_sensitive: bool,
    backwards: bool,
    in_selection: bool,
    whole_words: bool,
    all_charsets: bool,
    focus: usize,
}

#[derive(Clone, Copy)]
enum SrFocus {
    Search,
    Repl,
    Mode(usize),
    Check(usize),
}

impl SearchReplaceDialog {
    pub fn new(replace: bool, initial: String) -> Self {
        let search_cursor = initial.chars().count();
        SearchReplaceDialog {
            replace,
            search: initial,
            search_cursor,
            replacement: String::new(),
            repl_cursor: 0,
            mode: 0,
            case_sensitive: false,
            backwards: false,
            in_selection: false,
            whole_words: false,
            all_charsets: false,
            focus: 0,
        }
    }

    /// Like `new`, but starting in Hex mode (for the editor's hex search).
    pub fn new_hex(replace: bool, initial: String) -> Self {
        let mut d = Self::new(replace, initial);
        d.mode = 2;
        d
    }

    fn items(&self) -> Vec<SrFocus> {
        let mut v = vec![SrFocus::Search];
        if self.replace {
            v.push(SrFocus::Repl);
        }
        v.extend([SrFocus::Mode(0), SrFocus::Mode(1), SrFocus::Mode(2), SrFocus::Mode(3)]);
        v.extend((0..5).map(SrFocus::Check));
        v
    }

    fn cur(&self) -> SrFocus {
        let items = self.items();
        items[self.focus.min(items.len() - 1)]
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let len = self.items().len();
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => {
                if self.search.trim().is_empty() {
                    return DialogResult::Cancel;
                }
                return DialogResult::Submit(Submit::SearchReplace(self.params()));
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % len,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + len - 1) % len,
            KeyCode::Char(' ') if !matches!(self.cur(), SrFocus::Search | SrFocus::Repl) => {
                match self.cur() {
                    SrFocus::Mode(m) => self.mode = m,
                    SrFocus::Check(c) => self.toggle_check(c),
                    _ => {}
                }
            }
            _ => match self.cur() {
                SrFocus::Search => edit_text(&mut self.search, &mut self.search_cursor, key),
                SrFocus::Repl => edit_text(&mut self.replacement, &mut self.repl_cursor, key),
                _ => {}
            },
        }
        DialogResult::None
    }

    fn toggle_check(&mut self, c: usize) {
        match c {
            0 => self.case_sensitive = !self.case_sensitive,
            1 => self.backwards = !self.backwards,
            2 => self.in_selection = !self.in_selection,
            3 => self.whole_words = !self.whole_words,
            4 => self.all_charsets = !self.all_charsets,
            _ => {}
        }
    }

    fn params(&self) -> SearchReplaceParams {
        // Map the search mode to a regex flag, converting wildcards.
        let (search, regex) = match self.mode {
            1 => (self.search.clone(), true),                // Regular expression
            3 => (wildcard_to_regex(&self.search), true),    // Wildcard search
            _ => (self.search.clone(), false),               // Normal / Hex (literal)
        };
        SearchReplaceParams {
            replace: self.replace,
            search,
            replacement: self.replacement.clone(),
            regex,
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            backwards: self.backwards,
            hex: self.mode == 2,
        }
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let title = if self.replace { "Replace" } else { "Search" };
        let height = if self.replace { 14 } else { 12 };
        let rect = centered(area, 64u16.min(area.width.saturating_sub(2)), height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let mut y = inner.y;
        let mut caret = None;
        let line_at = |yy: u16| Rect { x: inner.x, y: yy, width: inner.width, height: 1 };

        f.render_widget(Paragraph::new(Span::styled("Enter search string:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.search, self.search_cursor,
            matches!(self.cur(), SrFocus::Search), false, theme,
        ) {
            caret = Some(p);
        }
        y += 1;
        if self.replace {
            f.render_widget(
                Paragraph::new(Span::styled("Enter replacement string:", base)),
                line_at(y),
            );
            y += 1;
            if let Some(p) = draw_input_field(
                f, line_at(y), &self.replacement, self.repl_cursor,
                matches!(self.cur(), SrFocus::Repl), false, theme,
            ) {
                caret = Some(p);
            }
            y += 1;
        }
        y += 1; // spacer

        // Options: radios (left) + checkboxes (right).
        let radios = ["Normal", "Regular expression", "Hexadecimal", "Wildcard search"];
        let checks = ["Case sensitive", "Backwards", "In selection", "Whole words", "All charsets"];
        let check_vals = [
            self.case_sensitive, self.backwards, self.in_selection, self.whole_words, self.all_charsets,
        ];
        let half = inner.width / 2;
        for row in 0..5u16 {
            let ry = y + row;
            if ry >= inner.y + inner.height - 1 {
                break;
            }
            if (row as usize) < radios.len() {
                let focused = matches!(self.cur(), SrFocus::Mode(m) if m == row as usize);
                f.render_widget(
                    Paragraph::new(Line::from(radio_span(
                        radios[row as usize], self.mode == row as usize, focused, theme,
                    )))
                    .style(base),
                    Rect { x: inner.x, y: ry, width: half, height: 1 },
                );
            }
            let focused = matches!(self.cur(), SrFocus::Check(c) if c == row as usize);
            f.render_widget(
                Paragraph::new(Line::from(check_span(
                    checks[row as usize], check_vals[row as usize], focused, theme,
                )))
                .style(base),
                Rect { x: inner.x + half, y: ry, width: inner.width - half, height: 1 },
            );
        }

        let by = inner.y + inner.height - 1;
        if !draw_ok_cancel(f, gfx, line_at(by), theme) {
            f.render_widget(
                Paragraph::new(ok_cancel_line(true, theme))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(base),
                line_at(by),
            );
        }

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

/// Convert a shell wildcard to an (unanchored) regular expression.
fn wildcard_to_regex(pattern: &str) -> String {
    let mut out = String::new();
    for ch in pattern.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c if ".+()|[]{}^$\\".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}
