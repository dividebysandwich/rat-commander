//! The raw `git` output viewer: a large scrollable box showing exactly what a
//! git command printed, with a Close button at the bottom.
//!
//! Git's own wording is the most useful thing we can show for status/log/push
//! results and for failures, so the text is presented verbatim — never wrapped
//! (that would mangle `log --graph`) but horizontally scrollable instead.

use super::widgets::*;
use super::DialogResult;

pub struct GitOutputDialog {
    /// The command this reports on, e.g. `"status"` — shown in the title.
    pub title: String,
    /// Whether git exited successfully (a failure is titled and coloured as one).
    pub ok: bool,
    lines: Vec<String>,
    /// First visible line (vertical scroll offset).
    top: usize,
    /// First visible column (horizontal scroll offset).
    left: usize,
    /// Interior size from the last render, so paging and clamping match what the
    /// user actually sees.
    view_h: usize,
    view_w: usize,
    /// The Close button's screen rect, recorded for click hit-testing.
    close_rect: Rect,
}

impl GitOutputDialog {
    pub fn new(title: impl Into<String>, ok: bool, text: &str) -> Self {
        // An empty body still deserves a line, so the box never renders blank.
        let lines: Vec<String> = if text.trim().is_empty() {
            vec!["(no output)".to_string()]
        } else {
            text.lines().map(str::to_string).collect()
        };
        GitOutputDialog {
            title: title.into(),
            ok,
            lines,
            top: 0,
            left: 0,
            view_h: 1,
            view_w: 1,
            close_rect: Rect::default(),
        }
    }

    /// The longest line, for clamping the horizontal scroll.
    fn max_len(&self) -> usize {
        self.lines.iter().map(|l| l.chars().count()).max().unwrap_or(0)
    }

    /// Largest first-visible line that still fills the view.
    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.view_h)
    }

    fn scroll_by(&mut self, delta: isize) {
        let t = (self.top as isize + delta).max(0) as usize;
        self.top = t.min(self.max_top());
    }

    pub(crate) fn handle_scroll(&mut self, delta: isize) -> DialogResult {
        self.scroll_by(delta);
        DialogResult::None
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let page = self.view_h.max(1) as isize;
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => return DialogResult::Cancel,
            KeyCode::Up => self.scroll_by(-1),
            KeyCode::Down => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::PageDown => self.scroll_by(page),
            KeyCode::Home => {
                self.top = 0;
                self.left = 0;
            }
            KeyCode::End => self.top = self.max_top(),
            KeyCode::Left => self.left = self.left.saturating_sub(4),
            KeyCode::Right => {
                // Stop once the longest line's tail is on screen.
                let max_left = self.max_len().saturating_sub(self.view_w);
                self.left = (self.left + 4).min(max_left);
            }
            _ => {}
        }
        DialogResult::None
    }

    /// Any click on the Close button dismisses; clicks elsewhere are ignored so
    /// a stray click can't lose the output.
    pub(crate) fn handle_click(&mut self, col: u16, row: u16) -> DialogResult {
        let r = self.close_rect;
        let hit = r.width > 0 && col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;
        if hit { DialogResult::Cancel } else { DialogResult::None }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        // A large box: git output is the point of this dialog, so give it room.
        let w = area.width.saturating_sub(6).clamp(1, 110);
        let h = area.height.saturating_sub(4).max(6);
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);

        let heading = if self.ok {
            format!("git {}", self.title)
        } else {
            format!("git {} — failed", self.title)
        };
        let block = dialog_block(&heading, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let body = rows[0];
        self.view_h = body.height as usize;
        self.view_w = body.width as usize;
        // A late resize (or a shorter list) can leave the offset past the end.
        self.top = self.top.min(self.max_top());

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        // A failed command's text reads as an error; successful output is plain.
        let text_style = if self.ok { base } else { base.fg(theme.error_fg) };
        let visible: Vec<Line> = self
            .lines
            .iter()
            .skip(self.top)
            .take(self.view_h)
            .map(|l| {
                let s: String = l.chars().skip(self.left).take(self.view_w).collect();
                Line::from(Span::styled(s, text_style))
            })
            .collect();
        f.render_widget(Paragraph::new(visible).style(base), body);

        // A scroll hint on the right of the button row when the text overflows.
        if self.lines.len() > self.view_h {
            let pos = format!(
                "{}–{}/{}",
                self.top + 1,
                (self.top + self.view_h).min(self.lines.len()),
                self.lines.len()
            );
            let px = rows[1].x + rows[1].width.saturating_sub(pos.chars().count() as u16 + 1);
            f.buffer_mut().set_string(px, rows[1].y, pos, base.fg(theme.panel_border));
        }

        let close = center_button_rect(rows[1], 13);
        self.close_rect = close;
        if !gfx_button(f, gfx, Slot::Button(0), close, "Close", true, theme) {
            f.render_widget(
                Paragraph::new(Line::from(button("[ Close ]", true, theme)))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(base),
                rows[1],
            );
        }
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

    fn screen(d: &mut GitOutputDialog, w: u16, h: u16) -> String {
        let theme = crate::ui::theme::Theme::default();
        let area = Rect::new(0, 0, w, h);
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| d.render(f, area, &theme, None)).unwrap();
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
    fn shows_output_and_a_close_button() {
        let mut d = GitOutputDialog::new("status", true, "On branch main\nnothing to commit");
        let s = screen(&mut d, 80, 24);
        assert!(s.contains("On branch main"));
        assert!(s.contains("git status"), "title names the command");
        assert!(s.contains("Close"), "a Close button is shown");
    }

    #[test]
    fn a_failure_is_marked_in_the_title() {
        let mut d = GitOutputDialog::new("push", false, "rejected: non-fast-forward");
        let s = screen(&mut d, 80, 24);
        assert!(s.contains("failed"), "a failed command says so");
        assert!(s.contains("rejected"));
    }

    #[test]
    fn empty_output_still_renders() {
        let mut d = GitOutputDialog::new("add", true, "   \n ");
        let s = screen(&mut d, 60, 12);
        assert!(s.contains("(no output)"));
    }

    #[test]
    fn scrolls_within_bounds_and_closes_on_dismiss_keys() {
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let mut d = GitOutputDialog::new("log", true, &text);
        let _ = screen(&mut d, 80, 24); // establish the view size

        // Down scrolls; Home returns to the top; End goes to the last page.
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.top, 1);
        d.handle_key(key(KeyCode::Home));
        assert_eq!(d.top, 0);
        // Scrolling up at the top is clamped, not negative.
        d.handle_key(key(KeyCode::Up));
        assert_eq!(d.top, 0);
        d.handle_key(key(KeyCode::End));
        assert_eq!(d.top, d.max_top(), "End lands on the final page");
        // Paging past the end clamps too.
        d.handle_key(key(KeyCode::PageDown));
        assert_eq!(d.top, d.max_top());
        // The mouse wheel scrolls the same way.
        d.handle_scroll(-3);
        assert!(d.top < d.max_top());

        assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
        // A plain letter neither scrolls nor closes.
        assert!(matches!(d.handle_key(key(KeyCode::Char('z'))), DialogResult::None));
    }

    #[test]
    fn horizontal_scroll_is_clamped_to_the_longest_line() {
        let mut d = GitOutputDialog::new("log", true, "short\nthis one is a good deal longer");
        let _ = screen(&mut d, 40, 10);
        d.handle_key(key(KeyCode::Left));
        assert_eq!(d.left, 0, "cannot scroll left of column 0");
        for _ in 0..50 {
            d.handle_key(key(KeyCode::Right));
        }
        assert_eq!(d.left, d.max_len().saturating_sub(d.view_w), "clamped at the longest line");
    }

    #[test]
    fn only_the_close_button_takes_a_click() {
        let mut d = GitOutputDialog::new("status", true, "x");
        let _ = screen(&mut d, 80, 24);
        let r = d.close_rect;
        assert!(matches!(d.handle_click(r.x, r.y), DialogResult::Cancel));
        // A click in the body is ignored, so the output isn't lost by accident.
        assert!(matches!(d.handle_click(r.x, r.y.saturating_sub(3)), DialogResult::None));
    }
}
