//! The top menu bar. Phase 1 renders it statically; Phase 2 makes the
//! pulldowns interactive (F9).

use crate::ui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub const TITLES: [&str; 5] = ["Left", "File", "Command", "Options", "Right"];

pub fn render(f: &mut Frame, area: Rect, theme: &Theme) {
    let width = area.width as usize;
    let mut text = String::from(" ");
    for title in TITLES {
        text.push_str(&format!(" {title} "));
    }
    while text.chars().count() < width {
        text.push(' ');
    }

    if theme.truecolor {
        // A horizontal gradient bar with readable text on top.
        let spans: Vec<Span> = text
            .chars()
            .take(width)
            .enumerate()
            .map(|(i, ch)| {
                Span::styled(
                    ch.to_string(),
                    Style::default().bg(theme.gradient_at(i, width)).fg(theme.bar_fg),
                )
            })
            .collect();
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(text, theme.menubar))),
            area,
        );
    }
}
