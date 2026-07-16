//! User menu (F2).

use super::widgets::*;
use super::{DialogResult, Submit};
use crate::usermenu::UserMenuEntry;

// ---------------------------------------------------------------------------
// User menu (F2)
// ---------------------------------------------------------------------------

pub struct UserMenuDialog {
    entries: Vec<UserMenuEntry>,
    cursor: usize,
}

impl UserMenuDialog {
    pub fn new(entries: Vec<UserMenuEntry>) -> Self {
        UserMenuDialog { entries, cursor: 0 }
    }

    fn submit_current(&self) -> DialogResult {
        match self.entries.get(self.cursor) {
            Some(e) => DialogResult::Submit(Submit::UserCommand(e.command.clone())),
            None => DialogResult::Cancel,
        }
    }

    /// The centered outer box and its scrolled first-visible index (kept in sync
    /// with `render`), for click hit-testing.
    fn geometry(&self, area: Rect) -> (Rect, usize) {
        let width = 64u16.min(area.width.saturating_sub(2));
        let max_h = area.height.saturating_sub(2);
        let height = (self.entries.len() as u16 + 2).min(max_h.max(3));
        let rect = centered(area, width, height);
        let rows = rect.height.saturating_sub(2) as usize;
        let first = crate::util::scroll::scroll_to_visible(0, self.cursor, rows);
        (rect, first)
    }

    /// A left click on an entry activates it (runs its command), like the app's
    /// pulldown menus. Clicks off the list do nothing.
    pub(crate) fn handle_click(&mut self, area: Rect, col: u16, row: u16) -> DialogResult {
        let (rect, first) = self.geometry(area);
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        if col < inner.x || col >= inner.x + inner.width || row < inner.y || row >= inner.y + inner.height {
            return DialogResult::None;
        }
        let idx = first + (row - inner.y) as usize;
        if idx < self.entries.len() {
            self.cursor = idx;
            return self.submit_current();
        }
        DialogResult::None
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let max = self.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::F(2) | KeyCode::F(10) => DialogResult::Cancel,
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(max);
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
            KeyCode::Enter => self.submit_current(),
            KeyCode::Char(c) => {
                // Activate the entry whose hotkey matches (exact, then loose).
                if let Some(i) = self
                    .entries
                    .iter()
                    .position(|e| e.hotkey == c)
                    .or_else(|| {
                        self.entries
                            .iter()
                            .position(|e| e.hotkey.eq_ignore_ascii_case(&c))
                    })
                {
                    self.cursor = i;
                    return self.submit_current();
                }
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let width = 64u16.min(area.width.saturating_sub(2));
        let max_h = area.height.saturating_sub(2);
        let height = (self.entries.len() as u16 + 2).min(max_h.max(3));
        let rect = centered(area, width, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("User menu", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = inner.height as usize;
        // Window the list so the cursor stays visible.
        let first = if self.cursor < rows {
            0
        } else {
            self.cursor + 1 - rows
        };

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let hotkey_style = Style::default()
            .fg(theme.dialog_title)
            .bg(theme.dialog_bg)
            .add_modifier(Modifier::BOLD);

        let mut lines: Vec<Line> = Vec::with_capacity(rows);
        for (idx, e) in self.entries.iter().enumerate().skip(first).take(rows) {
            let title = crate::util::text::ellipsize(&e.title, inner.width.saturating_sub(6) as usize);
            if idx == self.cursor {
                let text = format!(" {}  {}", e.hotkey, title);
                let mut padded = text;
                while (padded.chars().count() as u16) < inner.width {
                    padded.push(' ');
                }
                lines.push(Line::from(Span::styled(padded, theme.button_focused)));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {} ", e.hotkey), hotkey_style),
                    Span::styled(format!(" {title}"), base),
                ]));
            }
        }
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)),
            inner,
        );
    }
}

