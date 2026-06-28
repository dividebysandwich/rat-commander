//! The F1..F10 shortcut hint row at the bottom of the screen.

use crate::ui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Labels for the function-key row in panel mode (Midnight Commander order).
pub const PANEL_LABELS: [&str; 10] = [
    "Help", "Menu", "View", "Edit", "Copy", "RenMov", "Mkdir", "Delete", "PullDn", "Quit",
];

/// Labels for the internal editor's function-key row (mcedit order).
pub const EDITOR_LABELS: [&str; 10] = [
    "Help", "Save", "Mark", "Replac", "Copy", "Move", "Search", "Delete", "Hex", "Quit",
];

/// Labels for the editor's hex mode (only the supported functions are shown).
pub const HEX_LABELS: [&str; 10] = [
    "", "Save", "", "Replac", "", "", "Search", "", "Text", "Quit",
];

/// Render a function-key hint row using the supplied labels. The segments are
/// distributed so the row spans the full width of `area`. With truecolor, the
/// bar is drawn as a horizontal gradient; otherwise the classic two-tone look.
pub fn render(f: &mut Frame, area: Rect, labels: &[&str], theme: &Theme) {
    let n = labels.len().max(1);
    let total = area.width as usize;
    let base = total / n;
    let extra = total % n; // spread the remainder over the first segments

    // Build a per-cell list of (char, is_number).
    let mut cells: Vec<(char, bool)> = Vec::with_capacity(total);
    for (i, label) in labels.iter().enumerate() {
        let seg = base + usize::from(i < extra);
        let num = (i + 1).to_string();
        for ch in num.chars() {
            cells.push((ch, true));
        }
        let label_w = seg.saturating_sub(num.chars().count());
        let mut count = 0;
        for ch in label.chars().take(label_w) {
            cells.push((ch, false));
            count += 1;
        }
        while count < label_w {
            cells.push((' ', false));
            count += 1;
        }
    }

    let spans: Vec<Span> = cells
        .iter()
        .enumerate()
        .map(|(i, (ch, is_num))| {
            let style = if *is_num {
                // Numbers always sit on their solid, contrasting key-cap color.
                theme.fkey_num
            } else if theme.truecolor {
                Style::default().bg(theme.gradient_at(i, total)).fg(theme.bar_fg)
            } else {
                theme.fkey_label
            };
            Span::styled(ch.to_string(), style)
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
