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

/// Render a function-key hint row using the supplied labels. The 10 segments
/// are distributed so the row spans the full width of `area`.
pub fn render(f: &mut Frame, area: Rect, labels: &[&str; 10], theme: &Theme) {
    let total = area.width as usize;
    let base = total / 10;
    let extra = total % 10; // spread the remainder over the first segments

    let mut spans: Vec<Span> = Vec::with_capacity(20);
    for (i, label) in labels.iter().enumerate() {
        let seg = base + usize::from(i < extra);
        let num = (i + 1).to_string();
        let label_w = seg.saturating_sub(num.len());

        let mut text: String = label.chars().take(label_w).collect();
        while text.chars().count() < label_w {
            text.push(' ');
        }
        spans.push(Span::styled(num, theme.fkey_num));
        spans.push(Span::styled(text, theme.fkey_label));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
