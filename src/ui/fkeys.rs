//! The F1..F10 shortcut hint row at the bottom of the screen.

use crate::ui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Labels for the function-key row in panel mode (Midnight Commander order).
pub const PANEL_LABELS: [&str; 10] = [
    "Help", "Menu", "View", "Edit", "Copy", "RenMov", "Mkdir", "Delete", "PullDn", "Quit",
];

/// Render a function-key hint row using the supplied labels.
pub fn render(f: &mut Frame, area: Rect, labels: &[&str; 10], theme: &Theme) {
    let mut spans: Vec<Span> = Vec::with_capacity(20);
    for (i, label) in labels.iter().enumerate() {
        let num = format!("{}", i + 1);
        spans.push(Span::styled(num, theme.fkey_num));
        // Pad each label to a fixed cell so the row is evenly spaced.
        let cell_w = (area.width as usize / 10).saturating_sub(2).max(4);
        let mut text = label.to_string();
        text.truncate(cell_w);
        while text.len() < cell_w {
            text.push(' ');
        }
        spans.push(Span::styled(text, theme.fkey_label));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
