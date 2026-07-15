//! The Directory History window (Alt-H): the active panel's visited directories
//! as a scrollable, selectable list — the counterpart to the Shell History window
//! for `Alt-Y` / `Alt-U` (back / forward), letting you jump straight to any of
//! them instead of stepping one at a time.

use super::widgets::*;
use super::{DialogResult, Submit};

pub struct DirHistoryDialog {
    /// The panel's history in visit order (oldest first): the back stack, the
    /// current directory, then the forward stack. Kept as [`VfsPath`]s rather
    /// than display strings so a remote entry survives the round trip — parsing
    /// `sftp://host/path` back into a path would be lossy.
    entries: Vec<VfsPath>,
    /// Index of the panel's current directory, marked in the list.
    current: usize,
    cursor: usize,
    /// First visible row (scroll offset), maintained by the renderer.
    offset: usize,
    /// Interior height from the last render, for paging and click hit-testing.
    view_h: usize,
}

impl DirHistoryDialog {
    /// Build from a panel's `back` / `cwd` / `forward`, oldest → newest. The
    /// cursor starts on the current directory, so the list opens where you are.
    pub fn new(back: &[VfsPath], cwd: &VfsPath, forward: &[VfsPath]) -> Self {
        let mut entries: Vec<VfsPath> = back.to_vec();
        let current = entries.len();
        entries.push(cwd.clone());
        // The forward stack is a stack: its last element is the *next* directory,
        // so reverse it to keep the list in visit order.
        entries.extend(forward.iter().rev().cloned());
        DirHistoryDialog { entries, current, cursor: current, offset: 0, view_h: 1 }
    }

    fn submit_current(&self) -> DialogResult {
        match self.entries.get(self.cursor) {
            // Picking the directory you are already in is just a close.
            Some(_) if self.cursor == self.current => DialogResult::Cancel,
            Some(path) => DialogResult::Submit(Submit::GotoDir(Box::new(path.clone()))),
            None => DialogResult::Cancel,
        }
    }

    fn box_rect(&self, area: Rect) -> Rect {
        let w = 76u16.min(area.width.saturating_sub(4));
        let h = (self.entries.len() as u16 + 2).min(area.height.saturating_sub(4)).max(3);
        centered(area, w, h)
    }

    pub(crate) fn handle_click(&mut self, area: Rect, col: u16, row: u16) -> DialogResult {
        let rect = self.box_rect(area);
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        if col < inner.x || col >= inner.x + inner.width || row < inner.y || row >= inner.y + inner.height
        {
            return DialogResult::None;
        }
        let idx = self.offset + (row - inner.y) as usize;
        if idx < self.entries.len() {
            self.cursor = idx;
            return self.submit_current();
        }
        DialogResult::None
    }

    pub(crate) fn handle_scroll(&mut self, delta: isize) -> DialogResult {
        let max = self.entries.len().saturating_sub(1);
        self.cursor = (self.cursor as isize + delta).clamp(0, max as isize) as usize;
        DialogResult::None
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let max = self.entries.len().saturating_sub(1);
        let page = self.view_h.max(1);
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(max);
                DialogResult::None
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(page);
                DialogResult::None
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + page).min(max);
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
            _ => DialogResult::None,
        }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let rect = self.box_rect(area);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Directory History"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        self.view_h = inner.height as usize;
        // Scroll only when the cursor would leave the window, so it moves freely.
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + self.view_h {
            self.offset = self.cursor + 1 - self.view_h;
        }
        self.offset = self.offset.min(self.entries.len().saturating_sub(self.view_h));

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let lines: Vec<Line> = self
            .entries
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(self.view_h)
            .map(|(i, path)| {
                // A marker for where the panel is now, so the list reads as a
                // position within the history rather than a flat list.
                let mark = if i == self.current { "▶ " } else { "  " };
                let shown = path.display();
                let text = format!("{mark}{}", ellipsize(&shown, inner.width.saturating_sub(2) as usize));
                let style = if i == self.cursor {
                    theme.dialog_selection
                } else if i == self.current {
                    base.fg(theme.dialog_title)
                } else {
                    base
                };
                Line::from(Span::styled(pad_right(&text, inner.width as usize), style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines).style(base), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, ratatui::crossterm::event::KeyModifiers::NONE)
    }

    /// back = [/one, /two], cwd = /now, forward stack = [/later, /next]
    /// (a stack, so /next is the immediate next directory).
    fn dialog() -> DirHistoryDialog {
        let p = |s: &str| VfsPath::local(s);
        DirHistoryDialog::new(&[p("/one"), p("/two")], &p("/now"), &[p("/later"), p("/next")])
    }

    fn screen(d: &mut DirHistoryDialog, w: u16, h: u16) -> String {
        let theme = crate::ui::theme::Theme::default();
        let area = Rect::new(0, 0, w, h);
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| d.render(f, area, &theme)).unwrap();
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn lists_history_in_visit_order_around_the_current_directory() {
        let d = dialog();
        // Oldest → newest: the forward stack is reversed so the immediate next
        // directory sits right after the current one.
        let shown: Vec<String> = d.entries.iter().map(|p| p.display()).collect();
        assert_eq!(shown, vec!["/one", "/two", "/now", "/next", "/later"]);
        assert_eq!(d.current, 2, "the current directory sits after the back stack");
        assert_eq!(d.cursor, d.current, "the list opens where the panel is");
    }

    #[test]
    fn enter_jumps_to_the_picked_directory() {
        let mut d = dialog();
        d.handle_key(key(KeyCode::Up)); // → /two
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::GotoDir(p)) => assert_eq!(p.display(), "/two"),
            _ => panic!("expected a GotoDir submit"),
        }
        // Picking the current directory is a no-op close, not a pointless move.
        let mut d = dialog();
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
    }

    #[test]
    fn navigation_is_clamped_to_the_list() {
        let mut d = dialog();
        d.handle_key(key(KeyCode::Home));
        assert_eq!(d.cursor, 0);
        d.handle_key(key(KeyCode::Up));
        assert_eq!(d.cursor, 0, "clamped at the top");
        d.handle_key(key(KeyCode::End));
        assert_eq!(d.cursor, 4);
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.cursor, 4, "clamped at the bottom");
        // The wheel moves the selection the same way.
        d.handle_scroll(-2);
        assert_eq!(d.cursor, 2);
        assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
    }

    #[test]
    fn shows_every_entry_and_marks_the_current_one() {
        let mut d = dialog();
        let s = screen(&mut d, 60, 16);
        for p in ["/one", "/two", "/now", "/next", "/later"] {
            assert!(s.contains(p), "{p} is listed: {s}");
        }
        assert!(s.contains("▶ /now"), "the current directory is marked: {s}");
        assert!(s.contains("Directory History"), "titled");
    }

    #[test]
    fn a_click_picks_the_row_under_the_pointer() {
        let mut d = dialog();
        let area = Rect::new(0, 0, 60, 16);
        let _ = screen(&mut d, 60, 16);
        let rect = d.box_rect(area);
        // The first interior row is the oldest entry.
        match d.handle_click(area, rect.x + 2, rect.y + 1) {
            DialogResult::Submit(Submit::GotoDir(p)) => assert_eq!(p.display(), "/one"),
            other => panic!("expected /one, got {}", matches!(other, DialogResult::None)),
        }
        // A click outside the list does nothing.
        assert!(matches!(d.handle_click(area, 0, 0), DialogResult::None));
    }

    #[test]
    fn a_long_history_scrolls_and_keeps_the_cursor_visible() {
        let back: Vec<VfsPath> = (0..100).map(|i| VfsPath::local(format!("/dir{i}"))).collect();
        let mut d = DirHistoryDialog::new(&back, &VfsPath::local("/now"), &[]);
        let _ = screen(&mut d, 60, 12); // establishes view_h
        // Opening on the current directory (the last entry) must have scrolled to it.
        assert!(d.offset > 0, "the window scrolled to show the current directory");
        assert!(d.cursor >= d.offset && d.cursor < d.offset + d.view_h, "cursor is visible");
        d.handle_key(key(KeyCode::Home));
        let _ = screen(&mut d, 60, 12);
        assert_eq!(d.offset, 0, "Home scrolls back to the start");
    }
}
