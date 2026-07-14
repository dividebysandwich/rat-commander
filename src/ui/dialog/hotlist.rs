//! The directory hotlist (Ctrl-\): a scrollable list of bookmarked directories.
//! `Enter` jumps the active panel to the highlighted one; `a`/`Insert` adds the
//! current directory and `d`/`Delete` removes the highlighted one. Edits are
//! carried back to the app on close so they persist to `config.toml`.

use super::widgets::*;
use super::{DialogResult, Submit};
use ratatui::crossterm::event::KeyModifiers;

/// What the hotlist reports back when it closes.
#[derive(Debug, Clone)]
pub enum HotlistOutcome {
    /// Jump the active panel to `path`, and persist the (possibly edited)
    /// bookmark list.
    Jump { path: String, bookmarks: Vec<String> },
    /// Closed without jumping; persist the edited bookmark list.
    Save(Vec<String>),
}

pub struct HotlistDialog {
    /// Editable copy of the bookmarks (added/removed live; persisted on close).
    entries: Vec<String>,
    /// The active panel's directory, when it is a local path that can be added.
    current: Option<String>,
    cursor: usize,
    offset: usize,
    /// Whether the list was changed, so a plain Esc still persists it.
    dirty: bool,
    /// Interior list rect from the last render, for click hit-testing.
    list_area: Rect,
}

impl HotlistDialog {
    pub fn new(bookmarks: Vec<String>, current: Option<String>) -> Self {
        HotlistDialog {
            entries: bookmarks,
            current,
            cursor: 0,
            offset: 0,
            dirty: false,
            list_area: Rect::new(0, 0, 0, 0),
        }
    }

    fn close(&self) -> DialogResult {
        if self.dirty {
            DialogResult::Submit(Submit::Hotlist(HotlistOutcome::Save(self.entries.clone())))
        } else {
            DialogResult::Cancel
        }
    }

    fn jump_current(&self) -> DialogResult {
        match self.entries.get(self.cursor) {
            Some(path) => DialogResult::Submit(Submit::Hotlist(HotlistOutcome::Jump {
                path: path.clone(),
                bookmarks: self.entries.clone(),
            })),
            None => self.close(),
        }
    }

    /// Add the current directory if it isn't bookmarked already, and select it.
    fn add_current(&mut self) {
        let Some(cur) = self.current.clone() else { return };
        if let Some(pos) = self.entries.iter().position(|b| b == &cur) {
            self.cursor = pos;
            return;
        }
        self.entries.push(cur);
        self.cursor = self.entries.len() - 1;
        self.dirty = true;
    }

    fn remove_selected(&mut self) {
        if self.cursor < self.entries.len() {
            self.entries.remove(self.cursor);
            if self.cursor >= self.entries.len() {
                self.cursor = self.entries.len().saturating_sub(1);
            }
            self.dirty = true;
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let max = self.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => self.close(),
            // Ctrl-\ again toggles the hotlist back off.
            KeyCode::Char('\\') if ctrl => self.close(),
            KeyCode::Enter => self.jump_current(),
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(max);
                DialogResult::None
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(10);
                DialogResult::None
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 10).min(max);
                DialogResult::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.cursor = max;
                DialogResult::None
            }
            KeyCode::Insert | KeyCode::Char('a') => {
                self.add_current();
                DialogResult::None
            }
            KeyCode::Delete | KeyCode::Char('d') => {
                self.remove_selected();
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        let a = self.list_area;
        if col < a.x || col >= a.x + a.width || row < a.y || row >= a.y + a.height {
            return DialogResult::None;
        }
        let idx = self.offset + (row - a.y) as usize;
        if idx < self.entries.len() {
            self.cursor = idx;
            return self.jump_current();
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let width = 72u16.min(area.width.saturating_sub(4)).max(24);
        let max_rows = area.height.saturating_sub(6).clamp(1, 16);
        let rows = (self.entries.len().max(1) as u16).clamp(1, max_rows);
        let height = rows + 4; // borders(2) + list + footer(1) + spacing
        let rect = centered(area, width, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Directory hotlist"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        if inner.height < 2 || inner.width < 4 {
            return;
        }

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let list = Rect {
            height: inner.height.saturating_sub(1),
            ..inner
        };
        self.list_area = list;
        let visible = list.height as usize;
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + visible {
            self.offset = self.cursor + 1 - visible;
        }

        let mut lines: Vec<Line> = Vec::with_capacity(visible);
        if self.entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no bookmarks — press 'a' to add this directory)",
                base.fg(theme.panel_border),
            )));
        } else {
            for (idx, path) in self.entries.iter().enumerate().skip(self.offset).take(visible) {
                let text = ellipsize(path, inner.width.saturating_sub(2) as usize);
                if idx == self.cursor {
                    let mut padded = format!(" {text}");
                    while (padded.chars().count() as u16) < inner.width {
                        padded.push(' ');
                    }
                    lines.push(Line::from(Span::styled(padded, theme.button_focused)));
                } else {
                    lines.push(Line::from(Span::styled(format!(" {text}"), base)));
                }
            }
        }
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)), list);

        // Footer key hints (compact; not part of the translated catalogs).
        let footer = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " Enter: go   a/Ins: add   d/Del: remove   Esc: close",
                base.fg(theme.dialog_title),
            ))),
            footer,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn add_remove_and_jump() {
        let mut d = HotlistDialog::new(vec!["/a".into(), "/b".into()], Some("/c".into()));
        // 'a' adds the current dir and selects it.
        d.handle_key(ch('a'));
        assert_eq!(d.entries, vec!["/a", "/b", "/c"]);
        assert_eq!(d.cursor, 2);
        // Enter jumps to it, carrying the edited list for persistence.
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::Hotlist(HotlistOutcome::Jump { path, bookmarks })) => {
                assert_eq!(path, "/c");
                assert_eq!(bookmarks, vec!["/a", "/b", "/c"]);
            }
            _ => panic!("Enter should jump"),
        }
    }

    #[test]
    fn adding_existing_selects_it_without_duplicating() {
        let mut d = HotlistDialog::new(vec!["/a".into(), "/b".into()], Some("/a".into()));
        d.handle_key(ch('a'));
        assert_eq!(d.entries, vec!["/a", "/b"], "no duplicate");
        assert_eq!(d.cursor, 0, "selects the existing entry");
    }

    #[test]
    fn delete_marks_dirty_and_esc_saves() {
        let mut d = HotlistDialog::new(vec!["/a".into(), "/b".into()], None);
        d.handle_key(key(KeyCode::Delete));
        assert_eq!(d.entries, vec!["/b"]);
        match d.handle_key(key(KeyCode::Esc)) {
            DialogResult::Submit(Submit::Hotlist(HotlistOutcome::Save(b))) => {
                assert_eq!(b, vec!["/b"]);
            }
            _ => panic!("Esc after an edit should save"),
        }
    }

    #[test]
    fn esc_without_edits_cancels() {
        let mut d = HotlistDialog::new(vec!["/a".into()], None);
        assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
    }
}
