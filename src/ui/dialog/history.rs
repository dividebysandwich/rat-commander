//! The Shell History window (Ctrl-H): a scrollable list of recently entered
//! commands shown just above the command line. Selecting one copies it into the
//! command line without running it (Ctrl-P / Ctrl-N cycle history inline).

use super::widgets::*;
use super::{DialogResult, Submit};
use ratatui::crossterm::event::KeyModifiers;

pub struct ShellHistoryDialog {
    /// Recent commands in chronological order (oldest first, newest last), shown
    /// like terminal scrollback: older entries scroll up, newest at the bottom.
    entries: Vec<String>,
    cursor: usize,
    /// First visible row (scroll offset), maintained by the renderer.
    offset: usize,
}

impl ShellHistoryDialog {
    /// Build from the command line's chronological history (oldest → newest),
    /// with the cursor starting on the most recent command.
    pub fn new(history: &[String]) -> Self {
        let entries = history.to_vec();
        let cursor = entries.len().saturating_sub(1);
        ShellHistoryDialog { entries, cursor, offset: 0 }
    }

    /// Nothing to show (the caller then simply doesn't open the window).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn submit_current(&self) -> DialogResult {
        match self.entries.get(self.cursor) {
            Some(cmd) => DialogResult::Submit(Submit::RecallCommand(cmd.clone())),
            None => DialogResult::Cancel,
        }
    }

    /// A left click on a listed command recalls it into the command line. The
    /// geometry mirrors `render` (bottom-anchored above the command line), using
    /// the scroll `offset` the last render recorded.
    pub(crate) fn handle_click(&mut self, area: Rect, col: u16, row: u16) -> DialogResult {
        let width = 76u16.min(area.width.saturating_sub(2));
        let avail = area.height.saturating_sub(2 + 2);
        let height = (self.entries.len() as u16).clamp(1, avail.max(1)) + 2;
        let rect = Rect {
            x: area.x + 1,
            y: area.y + area.height.saturating_sub(2 + height),
            width,
            height,
        };
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        if col < inner.x || col >= inner.x + inner.width || row < inner.y || row >= inner.y + inner.height {
            return DialogResult::None;
        }
        let idx = self.offset + (row - inner.y) as usize;
        if idx < self.entries.len() {
            self.cursor = idx;
            return self.submit_current();
        }
        DialogResult::None
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let max = self.entries.len().saturating_sub(1);
        match (key.code, ctrl) {
            (KeyCode::Esc, _) => DialogResult::Cancel,
            // Ctrl-H again toggles the window back off.
            (KeyCode::Char('h'), true) => DialogResult::Cancel,
            // Up / Ctrl-P move toward older entries (up the list).
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            // Down / Ctrl-N move toward the most recent entry.
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) => {
                self.cursor = (self.cursor + 1).min(max);
                DialogResult::None
            }
            (KeyCode::PageUp, _) => {
                self.cursor = self.cursor.saturating_sub(10);
                DialogResult::None
            }
            (KeyCode::PageDown, _) => {
                self.cursor = (self.cursor + 10).min(max);
                DialogResult::None
            }
            (KeyCode::Home, _) => {
                self.cursor = 0;
                DialogResult::None
            }
            (KeyCode::End, _) => {
                self.cursor = max;
                DialogResult::None
            }
            (KeyCode::Enter, _) => self.submit_current(),
            _ => DialogResult::None,
        }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let width = 76u16.min(area.width.saturating_sub(2));
        // Leave the command line (1 row) and the F-key bar (1 row) below the box.
        let avail = area.height.saturating_sub(2 + 2); // 2 borders + cmdline + fkeys
        let height = (self.entries.len() as u16).clamp(1, avail.max(1)) + 2;
        let x = area.x + 1;
        // Bottom-anchored: sit directly above the command line row.
        let y = area.y + area.height.saturating_sub(2 + height);
        let rect = Rect { x, y, width, height };
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Shell History"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = inner.height as usize;
        // Keep the cursor within the visible window.
        self.offset = crate::util::scroll::scroll_to_visible(self.offset, self.cursor, rows);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let mut lines: Vec<Line> = Vec::with_capacity(rows);
        for (idx, cmd) in self.entries.iter().enumerate().skip(self.offset).take(rows) {
            let text = ellipsize(cmd, inner.width.saturating_sub(2) as usize);
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
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn starts_on_newest_and_navigates() {
        // history is oldest→newest; cursor starts on the newest ("three").
        let mut d = ShellHistoryDialog::new(&["one".into(), "two".into(), "three".into()]);
        assert!(matches!(d.submit_current(), DialogResult::Submit(Submit::RecallCommand(ref c)) if c == "three"));
        // Up / Ctrl-P go to older entries.
        d.handle_key(key(KeyCode::Up));
        assert!(matches!(d.submit_current(), DialogResult::Submit(Submit::RecallCommand(ref c)) if c == "two"));
        d.handle_key(ctrl('p'));
        assert!(matches!(d.submit_current(), DialogResult::Submit(Submit::RecallCommand(ref c)) if c == "one"));
        d.handle_key(key(KeyCode::Up)); // clamped at oldest
        assert!(matches!(d.submit_current(), DialogResult::Submit(Submit::RecallCommand(ref c)) if c == "one"));
        // Down / Ctrl-N go back toward the newest.
        d.handle_key(ctrl('n'));
        assert!(matches!(d.submit_current(), DialogResult::Submit(Submit::RecallCommand(ref c)) if c == "two"));
    }

    #[test]
    fn enter_recalls_and_esc_or_ctrl_h_cancel() {
        let mut d = ShellHistoryDialog::new(&["ls".into(), "pwd".into()]);
        // Enter on the newest selects it.
        assert!(matches!(d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::RecallCommand(ref c)) if c == "pwd"));
        assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
        assert!(matches!(d.handle_key(ctrl('h')), DialogResult::Cancel));
    }

    #[test]
    fn empty_history_is_reported() {
        assert!(ShellHistoryDialog::new(&[]).is_empty());
        assert!(!ShellHistoryDialog::new(&["x".into()]).is_empty());
    }

    #[test]
    fn click_recalls_the_clicked_command() {
        let mut d = ShellHistoryDialog::new(&["ls".into(), "pwd".into(), "cd /".into()]);
        // 80x24: the box is bottom-anchored, 3 entries → height 5 at y=17, so the
        // interior starts at y=18. Row 19 is the middle entry ("pwd").
        let area = Rect::new(0, 0, 80, 24);
        match d.handle_click(area, 10, 19) {
            DialogResult::Submit(Submit::RecallCommand(c)) => assert_eq!(c, "pwd"),
            _ => panic!("clicking a command recalls it"),
        }
        // A click above the window does nothing.
        assert!(matches!(d.handle_click(area, 10, 2), DialogResult::None));
    }
}
