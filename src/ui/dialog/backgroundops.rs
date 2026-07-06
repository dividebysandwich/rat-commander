//! The "Background operations" list: running transfers that were sent to the
//! background, each with a progress bar. Enter foregrounds the selected task,
//! Delete aborts it, Esc closes the list.

use super::widgets::*;
use super::{DialogResult, Submit};
use crate::ops::progress::TaskId;

/// One row in the background-operations list.
pub struct BgRow {
    pub id: TaskId,
    /// Display label, e.g. `"Copying  photos/img001.jpg"`.
    pub label: String,
    /// Completion fraction in `0.0..=1.0`.
    pub ratio: f64,
}

pub struct BackgroundOpsDialog {
    rows: Vec<BgRow>,
    cursor: usize,
    /// Clickable row rects → row index, recorded at render time.
    zones: Vec<(Rect, usize)>,
}

impl BackgroundOpsDialog {
    pub fn new(rows: Vec<BgRow>) -> Self {
        BackgroundOpsDialog { rows, cursor: 0, zones: Vec::new() }
    }

    fn selected_id(&self) -> Option<TaskId> {
        self.rows.get(self.cursor).map(|r| r.id)
    }

    /// Test accessor: `(id, ratio)` for each row, in order.
    #[cfg(test)]
    pub(crate) fn row_snapshot(&self) -> Vec<(TaskId, f64)> {
        self.rows.iter().map(|r| (r.id, r.ratio)).collect()
    }

    /// Replace the rows with a fresh snapshot, keeping the cursor on the same
    /// task when it's still present (so a live refresh doesn't move the selection).
    pub(crate) fn set_rows(&mut self, rows: Vec<BgRow>) {
        let sel = self.selected_id();
        self.rows = rows;
        if let Some(id) = sel
            && let Some(i) = self.rows.iter().position(|r| r.id == id)
        {
            self.cursor = i;
        }
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let last = self.rows.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(last);
                DialogResult::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.cursor = last;
                DialogResult::None
            }
            KeyCode::Enter => match self.selected_id() {
                Some(id) => DialogResult::Submit(Submit::ForegroundTask(id)),
                None => DialogResult::Cancel,
            },
            KeyCode::Delete => match self.selected_id() {
                Some(id) => DialogResult::Abort(id),
                None => DialogResult::None,
            },
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        for (rect, i) in &self.zones {
            if col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height {
                self.cursor = *i;
                if let Some(r) = self.rows.get(*i) {
                    return DialogResult::Submit(Submit::ForegroundTask(r.id));
                }
            }
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.zones.clear();
        let w = 64u16.min(area.width.saturating_sub(4));
        let n = self.rows.len().max(1) as u16;
        let height = (n + 4).min(area.height.saturating_sub(2)); // border + title + hint
        let rect = centered(area, w, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Background operations"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let rows_area = inner.height.saturating_sub(1) as usize; // reserve last row for the hint
        let bar_w: u16 = 22u16.min(inner.width.saturating_sub(4));
        let label_w = inner.width.saturating_sub(bar_w + 1);

        for (i, r) in self.rows.iter().enumerate().take(rows_area) {
            let y = inner.y + i as u16;
            let row_rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
            self.zones.push((row_rect, i));

            let selected = i == self.cursor;
            let label = crate::util::text::pad_right(
                &crate::util::text::ellipsize(&r.label, label_w as usize),
                label_w as usize,
            );
            let label_style = if selected { theme.dialog_selection } else { base };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(label, label_style))),
                Rect { x: inner.x, y, width: label_w, height: 1 },
            );
            let bar_rect = Rect { x: inner.x + label_w + 1, y, width: bar_w, height: 1 };
            pulse_gauge(
                f,
                bar_rect,
                r.ratio,
                &format!("{:.0}%", r.ratio * 100.0),
                theme.panel_border_active,
                theme,
            );
        }

        let hint_y = inner.y + inner.height.saturating_sub(1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter foreground   Del abort   Esc close",
                base,
            )))
            .alignment(ratatui::layout::Alignment::Center)
            .style(base),
            Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 },
        );
    }
}
