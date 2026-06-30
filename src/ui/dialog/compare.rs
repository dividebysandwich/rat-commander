//! Compare-directories dialog.

use super::widgets::*;
use super::{CompareMode, DialogResult, Submit};

// ---------------------------------------------------------------------------
// Compare-directories dialog
// ---------------------------------------------------------------------------

const COMPARE_MODES: [(&str, CompareMode); 3] = [
    ("Quick (name)", CompareMode::Quick),
    ("Size only", CompareMode::Size),
    ("Content", CompareMode::Content),
];

/// Asks how to compare the two panels' directories.
pub struct CompareDialog {
    focus: usize,
    zones: Vec<(Rect, usize)>,
}

impl CompareDialog {
    pub fn new() -> Self {
        CompareDialog { focus: 0, zones: Vec::new() }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Left | KeyCode::BackTab => {
                self.focus = (self.focus + COMPARE_MODES.len() - 1) % COMPARE_MODES.len();
                DialogResult::None
            }
            KeyCode::Right | KeyCode::Tab => {
                self.focus = (self.focus + 1) % COMPARE_MODES.len();
                DialogResult::None
            }
            KeyCode::Enter => DialogResult::Submit(Submit::CompareDirs(COMPARE_MODES[self.focus].1)),
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                DialogResult::Submit(Submit::CompareDirs(CompareMode::Quick))
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                DialogResult::Submit(Submit::CompareDirs(CompareMode::Size))
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                DialogResult::Submit(Submit::CompareDirs(CompareMode::Content))
            }
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, col: u16, row: u16) -> DialogResult {
        if let Some(&(_, i)) = self.zones.iter().find(|(r, _)| {
            col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
        }) {
            return DialogResult::Submit(Submit::CompareDirs(COMPARE_MODES[i].1));
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.zones.clear();
        let w = 52u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 7);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("Compare directories", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        f.render_widget(
            Paragraph::new("Compare the two panels by:")
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            rows[0],
        );

        // Centered row of bracketed buttons; record click zones.
        let labels: Vec<String> = COMPARE_MODES.iter().map(|(l, _)| format!("[ {l} ]")).collect();
        let total: usize =
            labels.iter().map(|l| l.chars().count()).sum::<usize>() + labels.len().saturating_sub(1);
        let mut x = rows[1].x + (rows[1].width.saturating_sub(total as u16)) / 2;
        for (i, label) in labels.iter().enumerate() {
            let style = if i == self.focus { theme.button_focused } else { theme.button };
            f.render_widget(
                Paragraph::new(Span::styled(label.clone(), style)),
                Rect { x, y: rows[1].y, width: label.chars().count() as u16, height: 1 },
            );
            self.zones.push((
                Rect { x, y: rows[1].y, width: label.chars().count() as u16, height: 1 },
                i,
            ));
            x += label.chars().count() as u16 + 1;
        }
    }
}

impl Default for CompareDialog {
    fn default() -> Self {
        Self::new()
    }
}

