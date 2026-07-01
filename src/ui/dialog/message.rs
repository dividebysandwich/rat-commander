//! Message dialog (errors / info).

use super::widgets::*;

// ---------------------------------------------------------------------------
// Message dialog (errors / info)
// ---------------------------------------------------------------------------

pub struct MessageDialog {
    pub title: String,
    pub message: String,
    pub is_error: bool,
}

impl MessageDialog {
    pub fn error(message: impl Into<String>) -> Self {
        MessageDialog {
            title: "Error".to_string(),
            message: message.into(),
            is_error: true,
        }
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 8);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let fg = if self.is_error {
            theme.error_fg
        } else {
            theme.dialog_fg
        };
        f.render_widget(
            Paragraph::new(self.message.clone())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(fg).bg(theme.dialog_bg))
                .alignment(ratatui::layout::Alignment::Center),
            rows[0],
        );
        let ok = center_button_rect(rows[1], 10);
        if !gfx_button(f, gfx, Slot::Button(0), ok, "OK", true, theme) {
            f.render_widget(
                Paragraph::new(Line::from(button("[ OK ]", true, theme)))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().bg(theme.dialog_bg)),
                rows[1],
            );
        }
    }
}

