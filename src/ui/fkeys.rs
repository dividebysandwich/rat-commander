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

/// Labels for the internal editor's function-key row (mcedit order).
pub const EDITOR_LABELS: [&str; 10] = [
    "Help", "Save", "Mark", "Replac", "Copy", "Move", "Search", "Delete", "", "Quit",
];

/// Render a function-key hint row using the supplied labels. The segments are
/// distributed so the row spans the full width of `area`, with the same
/// number/label styling everywhere in the program.
pub fn render(f: &mut Frame, area: Rect, labels: &[&str], theme: &Theme) {
    let n = labels.len().max(1);
    let total = area.width as usize;
    let base = total / n;
    let extra = total % n; // spread the remainder over the first segments

    let mut spans: Vec<Span> = Vec::with_capacity(n * 2);
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
