//! The viewer "Goto" dialog (line / percent / byte offset).

use super::widgets::*;
use super::{DialogResult, Submit};

// ---------------------------------------------------------------------------
// Goto dialog (viewer F5)
// ---------------------------------------------------------------------------

const GOTO_MODES: [(&str, crate::viewer::GotoMode); 4] = [
    ("Line number", crate::viewer::GotoMode::Line),
    ("Percents", crate::viewer::GotoMode::Percent),
    ("Decimal offset", crate::viewer::GotoMode::DecimalOffset),
    ("Hexadecimal offset", crate::viewer::GotoMode::HexOffset),
];

/// The viewer's "Goto" prompt: a value field plus a radio group choosing how to
/// interpret it (line / percent / decimal or hex byte offset).
pub struct GotoDialog {
    input: String,
    cursor: usize,
    /// Index into [`GOTO_MODES`].
    pub(crate) mode: usize,
}

impl GotoDialog {
    pub fn new() -> Self {
        GotoDialog { input: String::new(), cursor: 0, mode: 0 }
    }

    pub(crate) fn box_rect(&self, area: Rect) -> Rect {
        centered(area, 40u16.min(area.width.saturating_sub(2)), 9)
    }

    fn submit(&self) -> DialogResult {
        if self.input.trim().is_empty() {
            return DialogResult::Cancel;
        }
        DialogResult::Submit(Submit::ViewerGoto(self.input.clone(), GOTO_MODES[self.mode].1))
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => self.submit(),
            KeyCode::Up | KeyCode::BackTab => {
                self.mode = (self.mode + GOTO_MODES.len() - 1) % GOTO_MODES.len();
                DialogResult::None
            }
            KeyCode::Down | KeyCode::Tab => {
                self.mode = (self.mode + 1) % GOTO_MODES.len();
                DialogResult::None
            }
            _ => {
                edit_text(&mut self.input, &mut self.cursor, key);
                DialogResult::None
            }
        }
    }

    pub(crate) fn handle_click(&mut self, area: Rect, col: u16, row: u16) -> DialogResult {
        let rect = self.box_rect(area);
        if col < rect.x || col >= rect.x + rect.width || row < rect.y || row >= rect.y + rect.height {
            return DialogResult::None;
        }
        let (inner_x, inner_y) = (rect.x + 1, rect.y + 1);
        let (inner_w, inner_h) = (rect.width - 2, rect.height - 2);
        // Radio rows sit just below the input field.
        let ry = row as i32 - (inner_y + 1) as i32;
        if (0..GOTO_MODES.len() as i32).contains(&ry) {
            self.mode = ry as usize;
            return DialogResult::None;
        }
        // The button row is the last interior row (left half OK, right Cancel).
        if row == inner_y + inner_h - 1 {
            return if col < inner_x + inner_w / 2 { self.submit() } else { DialogResult::Cancel };
        }
        DialogResult::None
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let rect = self.box_rect(area);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("Goto", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let line_at = |yy: u16| Rect { x: inner.x, y: yy, width: inner.width, height: 1 };

        let caret =
            draw_input_field(f, line_at(inner.y), &self.input, self.cursor, true, false, theme);

        for (i, (label, _)) in GOTO_MODES.iter().enumerate() {
            f.render_widget(
                Paragraph::new(Line::from(radio_span(label, self.mode == i, false, theme)))
                    .style(base),
                line_at(inner.y + 1 + i as u16),
            );
        }

        f.render_widget(
            Paragraph::new(ok_cancel_line(true, theme))
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            line_at(inner.y + inner.height - 1),
        );

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

impl Default for GotoDialog {
    fn default() -> Self {
        Self::new()
    }
}

