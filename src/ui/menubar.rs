//! The top menu bar. Phase 1 renders it statically; Phase 2 makes the
//! pulldowns interactive (F9).

use crate::ui::theme::Theme;
use crate::util::sysinfo::SysSampler;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Sparkline};

/// Minimum terminal width before the system-status widget is shown.
pub const STATUS_MIN_WIDTH: u16 = 100;
/// Width reserved for the system-status widget.
pub const STATUS_WIDTH: u16 = 34;

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

/// Render the CPU-histogram + memory status widget into `area` (one row).
pub fn render_status(f: &mut Frame, area: Rect, s: &SysSampler, theme: &Theme) {
    if area.width < 16 {
        return;
    }
    // Opaque background so the widget reads over the gradient bar.
    f.render_widget(
        Block::default().style(Style::default().bg(theme.panel_bg)),
        area,
    );

    let cpu_label_w: u16 = 5; // "CPU "
    let mem_w: u16 = 9; // " MEM nnn%"
    let spark_w = area.width.saturating_sub(cpu_label_w + mem_w);

    let label_style = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    f.render_widget(
        Paragraph::new(Span::styled("CPU ", label_style)),
        Rect { width: cpu_label_w, ..area },
    );

    // Color the histogram by current load: green → yellow → red.
    let load = s.cpu_last();
    let spark_color = if load >= 80 {
        theme.error_fg
    } else if load >= 50 {
        theme.header_fg
    } else {
        theme.exec_fg
    };
    let data: Vec<u64> = s.cpu_history.iter().copied().collect();
    let spark = Sparkline::default()
        .data(data)
        .max(100)
        .style(Style::default().fg(spark_color).bg(theme.panel_bg));
    f.render_widget(
        spark,
        Rect {
            x: area.x + cpu_label_w,
            width: spark_w,
            ..area
        },
    );

    let mem = format!(" MEM{:>3}%", s.mem_percent());
    let mem_style = Style::default()
        .fg(theme.panel_border_active)
        .bg(theme.panel_bg);
    f.render_widget(
        Paragraph::new(Span::styled(mem, mem_style)),
        Rect {
            x: area.x + cpu_label_w + spark_w,
            width: mem_w,
            ..area
        },
    );
}
