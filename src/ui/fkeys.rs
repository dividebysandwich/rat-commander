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

/// The function-key index (0-based — `i` means F`i+1`) at screen column `col`
/// on the bar row `row`, or `None` if the click misses the row or lands on an
/// empty (disabled) segment. Mirrors the segment layout used by [`render`].
pub fn index_at(area: Rect, labels: &[&str], col: u16, row: u16) -> Option<usize> {
    if row != area.y || col < area.x || col >= area.x + area.width {
        return None;
    }
    let n = labels.len().max(1);
    let total = area.width as usize;
    let base = total / n;
    let extra = total % n;
    let mut x = area.x as usize;
    for (i, label) in labels.iter().enumerate() {
        let seg = base + usize::from(i < extra);
        if seg == 0 {
            continue;
        }
        if (col as usize) >= x && (col as usize) < x + seg {
            return (!label.is_empty()).then_some(i);
        }
        x += seg;
    }
    None
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_at_maps_columns_to_function_keys() {
        // 20 wide / 10 labels → 2 cells each: F1 at 0-1, F2 at 2-3, … F10 at 18-19.
        let area = Rect::new(0, 5, 20, 1);
        let labels = super::PANEL_LABELS;
        assert_eq!(index_at(area, &labels, 0, 5), Some(0));
        assert_eq!(index_at(area, &labels, 5, 5), Some(2)); // F3
        assert_eq!(index_at(area, &labels, 19, 5), Some(9)); // F10
        // Wrong row, or off the right edge → no hit.
        assert_eq!(index_at(area, &labels, 5, 4), None);
        assert_eq!(index_at(area, &labels, 20, 5), None);
        // Empty (disabled) segments report no hit.
        let hex = super::HEX_LABELS; // index 0 ("") and 2 ("") are blank
        assert_eq!(index_at(area, &hex, 0, 5), None);
        assert_eq!(index_at(area, &hex, 2, 5), Some(1)); // F2 "Save"
        assert_eq!(index_at(area, &hex, 4, 5), None); // F3 blank
    }
}
