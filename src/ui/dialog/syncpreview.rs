//! The directory-sync preview: exactly what a mirror is about to do, listed
//! before any of it happens.
//!
//! A sync is the one file operation whose *plan* is worth reading before it runs
//! — especially a mirror, which deletes. So the plan is shown in full (scrollable)
//! with a summary line, and nothing touches the disk until Execute.

use super::widgets::*;
use super::{DialogResult, Submit};
use crate::ops::sync::{SyncMode, SyncPlan, SyncStep};

pub struct SyncPreviewDialog {
    plan: SyncPlan,
    /// Pre-rendered `(label, is_delete)` rows, so painting doesn't re-format.
    rows: Vec<(String, bool)>,
    /// First visible row.
    top: usize,
    /// Interior height from the last render, for paging and clamping.
    view_h: usize,
    /// 0 = Execute, 1 = Cancel. Cancel is focused when the plan deletes, so the
    /// destructive option is never one stray Enter away.
    focus: u8,
    exec_rect: Rect,
    cancel_rect: Rect,
}

impl SyncPreviewDialog {
    pub fn new(plan: SyncPlan) -> Self {
        let rows = plan
            .steps
            .iter()
            .map(|s| (s.label(), matches!(s, SyncStep::Delete { .. })))
            .collect();
        // A plan that removes files opens on Cancel, so the destructive option is
        // never one stray Enter away; a purely additive one opens on Execute,
        // since there is nothing to lose. An empty plan has no Execute at all.
        let cancel_first = plan.counts().deletes > 0 || plan.is_empty();
        SyncPreviewDialog {
            plan,
            rows,
            top: 0,
            view_h: 1,
            focus: u8::from(cancel_first),
            exec_rect: Rect::default(),
            cancel_rect: Rect::default(),
        }
    }

    /// Whether the plan is a mirror that will delete files (drives the warning).
    fn destructive(&self) -> bool {
        self.plan.counts().deletes > 0
    }

    fn max_top(&self) -> usize {
        self.rows.len().saturating_sub(self.view_h)
    }

    fn scroll_by(&mut self, delta: isize) {
        self.top = ((self.top as isize + delta).max(0) as usize).min(self.max_top());
    }

    pub(crate) fn handle_scroll(&mut self, delta: isize) -> DialogResult {
        self.scroll_by(delta);
        DialogResult::None
    }

    fn submit(&self) -> DialogResult {
        // Nothing to run: close instead. Keeps the keyboard honest with the
        // render, which draws no Execute button for an empty plan.
        if self.plan.is_empty() {
            return DialogResult::Cancel;
        }
        DialogResult::Submit(Submit::SyncRun(Box::new(self.plan.clone())))
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let page = self.view_h.max(1) as isize;
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                if self.focus == 0 {
                    self.submit()
                } else {
                    DialogResult::Cancel
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                if !self.plan.is_empty() {
                    self.focus ^= 1;
                }
                DialogResult::None
            }
            KeyCode::Up => {
                self.scroll_by(-1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.scroll_by(1);
                DialogResult::None
            }
            KeyCode::PageUp => {
                self.scroll_by(-page);
                DialogResult::None
            }
            KeyCode::PageDown => {
                self.scroll_by(page);
                DialogResult::None
            }
            KeyCode::Home => {
                self.top = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.top = self.max_top();
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, col: u16, row: u16) -> DialogResult {
        let hit = |r: Rect| {
            r.width > 0 && col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
        };
        if hit(self.exec_rect) {
            return self.submit();
        }
        if hit(self.cancel_rect) {
            return DialogResult::Cancel;
        }
        DialogResult::None
    }

    /// The summary shown above the list, e.g. `"12 copies (4.1 MiB), 3 deletes"`.
    fn summary(&self) -> String {
        let c = self.plan.counts();
        let mut parts = Vec::new();
        if c.copies > 0 {
            parts.push(format!("{} to copy ({})", c.copies, human_size(c.bytes)));
        }
        if c.deletes > 0 {
            parts.push(format!("{} to delete", c.deletes));
        }
        if c.mkdirs > 0 {
            parts.push(format!("{} directories", c.mkdirs));
        }
        if parts.is_empty() {
            "Already in sync — nothing to do".to_string()
        } else {
            parts.join(",  ")
        }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, mut gfx: Option<&mut Gfx>) {
        let w = area.width.saturating_sub(6).clamp(1, 100);
        let h = area.height.saturating_sub(4).max(8);
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);

        // A mirror that deletes is flagged in red; an additive sync is routine.
        let block = if self.destructive() {
            danger_block(&crate::l10n::trd("Synchronize"), theme)
        } else {
            dialog_block(&crate::l10n::trd("Synchronize"), theme)
        };
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // direction
                Constraint::Length(1), // summary
                Constraint::Min(1),    // the plan
                Constraint::Length(1), // buttons
            ])
            .split(inner);

        let arrow = if matches!(self.plan.mode, SyncMode::TwoWay) { "↔" } else { "→" };
        f.render_widget(
            Paragraph::new(Line::from(ellipsize(
                &format!("{}  {arrow}  {}", self.plan.roots[0], self.plan.roots[1]),
                inner.width as usize,
            )))
            .style(base.fg(theme.dialog_title).add_modifier(Modifier::BOLD)),
            rows[0],
        );
        let sum_style = if self.destructive() { base.fg(theme.error_fg) } else { base };
        f.render_widget(Paragraph::new(Line::from(self.summary())).style(sum_style), rows[1]);

        // The plan itself.
        self.view_h = rows[2].height as usize;
        self.top = self.top.min(self.max_top());
        let lines: Vec<Line> = self
            .rows
            .iter()
            .skip(self.top)
            .take(self.view_h)
            .map(|(label, is_delete)| {
                let style = if *is_delete { base.fg(theme.error_fg) } else { base };
                Line::from(Span::styled(ellipsize(label, rows[2].width as usize), style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines).style(base), rows[2]);

        // Buttons: Execute / Cancel, with a scroll position between them.
        let brow = rows[3];
        let (exec_label, cancel_label) = (crate::l10n::trd("Execute"), crate::l10n::trd("Cancel"));
        let ew = 13u16.min(brow.width);
        let cw = 12u16.min(brow.width.saturating_sub(ew));
        let exec = Rect { x: brow.x, y: brow.y, width: ew, height: 1 };
        let cancel = Rect { x: brow.x + brow.width - cw, y: brow.y, width: cw, height: 1 };
        let can_exec = !self.plan.is_empty();
        // Only record a clickable Execute zone when one is actually drawn.
        self.exec_rect = if can_exec { exec } else { Rect::default() };
        self.cancel_rect = cancel;
        if self.rows.len() > self.view_h {
            let pos = format!("{}–{}/{}", self.top + 1, (self.top + self.view_h).min(self.rows.len()), self.rows.len());
            let mid_x = exec.x + exec.width + 1;
            let mid_w = cancel.x.saturating_sub(mid_x);
            if mid_w > 0 {
                f.render_widget(
                    Paragraph::new(Line::from(pos))
                        .alignment(ratatui::layout::Alignment::Center)
                        .style(base.fg(theme.panel_border)),
                    Rect { x: mid_x, y: brow.y, width: mid_w, height: 1 },
                );
            }
        }
        // An empty plan has nothing to execute, so only Cancel is live.
        if can_exec && !gfx_button(f, gfx.as_deref_mut(), Slot::Button(0), exec, &exec_label, self.focus == 0, theme) {
            f.render_widget(
                Paragraph::new(Line::from(button(&format!("[ {exec_label} ]"), self.focus == 0, theme)))
                    .style(base),
                exec,
            );
        }
        if !gfx_button(f, gfx, Slot::Button(1), cancel, &cancel_label, self.focus == 1 || !can_exec, theme) {
            f.render_widget(
                Paragraph::new(Line::from(button(
                    &format!("[ {cancel_label} ]"),
                    self.focus == 1 || !can_exec,
                    theme,
                )))
                .style(base),
                cancel,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::sync::SyncStep;
    use crate::vfs::VfsPath;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, ratatui::crossterm::event::KeyModifiers::NONE)
    }

    fn copy(rel: &str, size: u64) -> SyncStep {
        SyncStep::Copy {
            from: 0,
            src: VfsPath::local(format!("/a/{rel}")),
            dst: VfsPath::local(format!("/b/{rel}")),
            rel: rel.into(),
            size,
        }
    }

    fn del(rel: &str) -> SyncStep {
        SyncStep::Delete {
            side: 1,
            path: VfsPath::local(format!("/b/{rel}")),
            rel: rel.into(),
            files: 1,
            size: 1,
        }
    }

    fn plan_of(steps: Vec<SyncStep>) -> SyncPlan {
        SyncPlan {
            steps,
            mode: SyncMode::OneWay { delete_extraneous: true },
            roots: ["/a".into(), "/b".into()],
        }
    }

    fn screen(d: &mut SyncPreviewDialog, w: u16, h: u16) -> String {
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
    fn lists_the_plan_with_a_summary_and_direction() {
        let mut d = SyncPreviewDialog::new(plan_of(vec![copy("a.txt", 1024), del("old.txt")]));
        let s = screen(&mut d, 80, 24);
        assert!(s.contains("a.txt") && s.contains("old.txt"), "every step is listed");
        assert!(s.contains("1 to copy") && s.contains("1 to delete"), "summary counts: {s}");
        assert!(s.contains("/a") && s.contains("/b") && s.contains('→'), "shows the direction");
        assert!(s.contains("Execute") && s.contains("Cancel"));
    }

    #[test]
    fn a_deleting_plan_opens_on_cancel_so_enter_cannot_destroy() {
        let mut d = SyncPreviewDialog::new(plan_of(vec![copy("a", 1), del("b")]));
        assert_eq!(d.focus, 1, "a plan with deletes focuses Cancel");
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
        // Tabbing to Execute then Enter submits the plan.
        d.handle_key(key(KeyCode::Tab));
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::SyncRun(_))
        ));
    }

    #[test]
    fn an_additive_plan_opens_on_execute() {
        let mut d = SyncPreviewDialog::new(plan_of(vec![copy("a", 1)]));
        assert_eq!(d.focus, 0, "nothing is destroyed, so Execute is the default");
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::SyncRun(_))
        ));
    }

    #[test]
    fn an_empty_plan_says_so_and_offers_no_execute() {
        let mut d = SyncPreviewDialog::new(plan_of(vec![]));
        let s = screen(&mut d, 70, 16);
        assert!(s.contains("Already in sync"), "says there is nothing to do: {s}");
        assert!(!s.contains("Execute"), "and offers no Execute button");
        // Enter just closes it (focus sits on Cancel).
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
    }

    #[test]
    fn scrolls_a_long_plan_within_bounds() {
        let steps: Vec<SyncStep> = (0..200).map(|i| copy(&format!("f{i}"), 1)).collect();
        let mut d = SyncPreviewDialog::new(plan_of(steps));
        let _ = screen(&mut d, 80, 24); // establishes the view height
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.top, 1);
        d.handle_key(key(KeyCode::Up));
        d.handle_key(key(KeyCode::Up));
        assert_eq!(d.top, 0, "clamped at the top");
        d.handle_key(key(KeyCode::End));
        assert_eq!(d.top, d.max_top());
        d.handle_key(key(KeyCode::PageDown));
        assert_eq!(d.top, d.max_top(), "clamped at the end");
        d.handle_scroll(-5);
        assert!(d.top < d.max_top(), "the wheel scrolls too");
    }

    #[test]
    fn clicking_execute_runs_and_cancel_closes() {
        let mut d = SyncPreviewDialog::new(plan_of(vec![copy("a", 1)]));
        let _ = screen(&mut d, 80, 24);
        let (e, c) = (d.exec_rect, d.cancel_rect);
        assert!(matches!(d.handle_click(c.x, c.y), DialogResult::Cancel));
        assert!(matches!(d.handle_click(e.x, e.y), DialogResult::Submit(Submit::SyncRun(_))));
        // A click on the list does nothing (no accidental execution).
        assert!(matches!(d.handle_click(e.x, e.y.saturating_sub(3)), DialogResult::None));
    }
}
