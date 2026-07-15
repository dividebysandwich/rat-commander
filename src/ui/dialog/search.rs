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
    /// "Find all" was pressed rather than OK: highlight every matching line
    /// instead of jumping to the next match.
    pub find_all: bool,
}

pub struct SearchReplaceDialog {
    pub(crate) replace: bool,
    search: String,
    search_cursor: usize,
    /// The whole search field is marked (pre-filled): typing replaces it.
    search_selected: bool,
    replacement: String,
    repl_cursor: usize,
    /// The whole replacement field is marked (pre-filled): typing replaces it.
    repl_selected: bool,
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
    /// A bottom-row button, indexed into [`SearchReplaceDialog::buttons`].
    Button(usize),
}

/// What a bottom-row button does when activated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SrButton {
    /// Find the next match (the plain Enter action).
    Ok,
    /// Highlight every line holding the term, and keep it highlighted.
    FindAll,
    Cancel,
}

impl SearchReplaceDialog {
    pub fn new(replace: bool, initial: String, initial_replacement: String) -> Self {
        let search_cursor = initial.chars().count();
        let repl_cursor = initial_replacement.chars().count();
        let search_selected = !initial.is_empty();
        let repl_selected = !initial_replacement.is_empty();
        SearchReplaceDialog {
            replace,
            search: initial,
            search_cursor,
            search_selected,
            replacement: initial_replacement,
            repl_cursor,
            repl_selected,
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
    pub fn new_hex(replace: bool, initial: String, initial_replacement: String) -> Self {
        let mut d = Self::new(replace, initial, initial_replacement);
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
        v.extend((0..self.buttons().len()).map(SrFocus::Button));
        v
    }

    /// The bottom-row buttons. "Find all" is offered on the plain Search dialog
    /// only: highlighting every match says nothing useful next to a Replace,
    /// which reports its own count.
    fn buttons(&self) -> Vec<(SrButton, &'static str)> {
        let mut v = vec![(SrButton::Ok, "OK")];
        if !self.replace {
            v.push((SrButton::FindAll, "Find all"));
        }
        v.push((SrButton::Cancel, "Cancel"));
        v
    }

    /// Run a button. An empty term has nothing to look for, so it just closes.
    fn activate(&self, b: SrButton) -> DialogResult {
        if b == SrButton::Cancel || self.search.trim().is_empty() {
            return DialogResult::Cancel;
        }
        let mut p = self.params();
        p.find_all = b == SrButton::FindAll;
        DialogResult::Submit(Submit::SearchReplace(p))
    }

    /// `(action, label, rect)` for each bottom-row button, centred as a group.
    /// Shared by `render` and `click_field` so the two can't disagree.
    fn buttons_at(&self, area: Rect) -> Vec<(SrButton, String, Rect)> {
        let height = if self.replace { 14 } else { 12 };
        let rect = centered(area, 64u16.min(area.width.saturating_sub(2)), height);
        let row = Rect {
            x: rect.x + 1,
            y: rect.y + rect.height - 2,
            width: rect.width.saturating_sub(2),
            height: 1,
        };
        let items = self.buttons();
        let labels: Vec<String> = items.iter().map(|(_, l)| crate::l10n::trd(l)).collect();
        let widths: Vec<u16> = labels.iter().map(|l| l.chars().count() as u16 + 4).collect();
        let gaps = 2 * (items.len() as u16 - 1);
        let total: u16 = widths.iter().sum::<u16>() + gaps;
        let mut x = row.x + row.width.saturating_sub(total) / 2;
        let mut out = Vec::with_capacity(items.len());
        for (i, (action, _)) in items.iter().enumerate() {
            let r = Rect { x, y: row.y, width: widths[i], height: 1 };
            out.push((*action, labels[i].clone(), r));
            x += widths[i] + 2;
        }
        out
    }

    /// Index of the focused bottom-row button, if focus is on one.
    fn focused_button(&self) -> Option<usize> {
        match self.cur() {
            SrFocus::Button(i) => Some(i),
            _ => None,
        }
    }

    fn cur(&self) -> SrFocus {
        let items = self.items();
        items[self.focus.min(items.len() - 1)]
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let len = self.items().len();
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            // Enter runs the focused button; from a field it means "find".
            KeyCode::Enter => {
                let action = match self.cur() {
                    SrFocus::Button(i) => self.buttons()[i].0,
                    _ => SrButton::Ok,
                };
                return self.activate(action);
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % len,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + len - 1) % len,
            KeyCode::Char(' ') if !matches!(self.cur(), SrFocus::Search | SrFocus::Repl) => {
                match self.cur() {
                    SrFocus::Mode(m) => self.mode = m,
                    SrFocus::Check(c) => self.toggle_check(c),
                    SrFocus::Button(i) => return self.activate(self.buttons()[i].0),
                    _ => {}
                }
            }
            _ => match self.cur() {
                SrFocus::Search => edit_text_marked(
                    &mut self.search,
                    &mut self.search_cursor,
                    &mut self.search_selected,
                    key,
                ),
                SrFocus::Repl => edit_text_marked(
                    &mut self.replacement,
                    &mut self.repl_cursor,
                    &mut self.repl_selected,
                    key,
                ),
                _ => {}
            },
        }
        DialogResult::None
    }

    /// Route a click onto a text field, mode radio, or option checkbox (the
    /// OK/Cancel row is left to the generic dialog button handler). The geometry
    /// mirrors `render`. Returns `Some` when a field/radio/check was hit.
    pub(crate) fn click_field(&mut self, area: Rect, col: u16, row: u16) -> Option<DialogResult> {
        let height = if self.replace { 14 } else { 12 };
        let rect = centered(area, 64u16.min(area.width.saturating_sub(2)), height);
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        if col < inner.x || col >= inner.x + inner.width {
            return None;
        }
        let caret_at = |value: &str| (col.saturating_sub(inner.x) as usize).min(value.chars().count());
        // Search field (one row below its label).
        if row == inner.y + 1 {
            self.focus = 0;
            self.search_selected = false;
            self.search_cursor = caret_at(&self.search);
            return Some(DialogResult::None);
        }
        // Replacement field (only when replacing).
        if self.replace && row == inner.y + 3 {
            self.focus = 1;
            self.repl_selected = false;
            self.repl_cursor = caret_at(&self.replacement);
            return Some(DialogResult::None);
        }
        // The bottom-row buttons (this dialog has three, so the generic
        // half-and-half OK/Cancel hit-test in `Dialog::handle_click` won't do).
        for (i, (action, _, r)) in self.buttons_at(area).into_iter().enumerate() {
            if row == r.y && col >= r.x && col < r.x + r.width {
                self.focus = self.items().len() - self.buttons().len() + i;
                return Some(self.activate(action));
            }
        }
        // Options block: radios (left half) + checkboxes (right half).
        let opt_y = inner.y + if self.replace { 5 } else { 3 };
        if row >= opt_y && row < opt_y + 5 {
            let r = (row - opt_y) as usize;
            let base = if self.replace { 2 } else { 1 };
            if col < inner.x + inner.width / 2 {
                if r < 4 {
                    self.mode = r;
                    self.focus = base + r;
                }
            } else {
                self.toggle_check(r);
                self.focus = base + 4 + r;
            }
            return Some(DialogResult::None);
        }
        None
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
            // Set by `activate` for the "Find all" button; a plain OK leaves it off.
            find_all: false,
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
        let search_focused = matches!(self.cur(), SrFocus::Search);
        if let Some(p) = draw_input_field_ex(
            f, line_at(y), &self.search, self.search_cursor,
            search_focused, false, search_focused && self.search_selected, theme,
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
            let repl_focused = matches!(self.cur(), SrFocus::Repl);
            if let Some(p) = draw_input_field_ex(
                f, line_at(y), &self.replacement, self.repl_cursor,
                repl_focused, false, repl_focused && self.repl_selected, theme,
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

        // The button row is drawn here rather than via the shared OK/Cancel
        // helper: the Search dialog carries a third button ("Find all").
        let mut gfx = gfx;
        let focused = self.focused_button();
        for (i, (_, label, r)) in self.buttons_at(area).into_iter().enumerate() {
            // Enter from a field means "find", so OK reads as focused until the
            // ring actually lands on a button.
            let hot = focused.map(|f| f == i).unwrap_or(i == 0);
            if !gfx_button(f, gfx.as_deref_mut(), Slot::Button(i as u16), r, &label, hot, theme) {
                let style = if hot { theme.button_focused } else { theme.button };
                let text = if hot { format!("[< {label} >]") } else { format!("[  {label}  ]") };
                f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))).style(base), r);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn prefilled_terms_are_marked_and_typing_replaces() {
        // Reopened with remembered search + replacement, both marked.
        let mut d = SearchReplaceDialog::new(true, "old".into(), "repl".into());
        assert!(d.search_selected && d.repl_selected);

        // Typing in the (focused) search field replaces the whole marked term,
        // leaving the remembered replacement intact.
        d.handle_key(key(KeyCode::Char('n')));
        d.handle_key(key(KeyCode::Char('e')));
        d.handle_key(key(KeyCode::Char('w')));
        assert!(!d.search_selected);
        let p = d.params();
        assert_eq!(p.search, "new");
        assert_eq!(p.replacement, "repl");
    }

    #[test]
    fn cursor_move_clears_mark_so_typing_appends() {
        let mut d = SearchReplaceDialog::new(false, "abc".into(), String::new());
        assert!(d.search_selected);
        d.handle_key(key(KeyCode::End)); // drops the mark, keeps the text
        assert!(!d.search_selected);
        d.handle_key(key(KeyCode::Char('d')));
        assert_eq!(d.params().search, "abcd");
    }
}
