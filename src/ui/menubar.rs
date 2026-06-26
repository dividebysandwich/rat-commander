//! The top menu bar. Phase 1 renders it statically; Phase 2 makes the
//! pulldowns interactive (F9).

use crate::ui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub const TITLES: [&str; 5] = ["Left", "File", "Command", "Options", "Right"];

pub fn render(f: &mut Frame, area: Rect, theme: &Theme) {
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ", theme.menubar));
    for title in TITLES {
        spans.push(Span::styled(format!(" {title} "), theme.menubar));
    }
    // Fill the rest of the bar.
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if (used as u16) < area.width {
        spans.push(Span::styled(
            " ".repeat(area.width as usize - used),
            theme.menubar,
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
