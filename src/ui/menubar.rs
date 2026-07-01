//! The top menu bar. Phase 1 renders it statically; Phase 2 makes the
//! pulldowns interactive (F9).

use crate::ui::theme::Theme;
use crate::util::sysinfo::SysSampler;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Sparkline};

/// Minimum terminal width before the system-status widget is shown.
pub const STATUS_MIN_WIDTH: u16 = 100;
/// Width reserved for the system-status widget.
pub const STATUS_WIDTH: u16 = 34;

pub const TITLES: [&str; 5] = ["Left", "File", "Command", "Options", "Right"];

/// The menu-bar titles in the active language (the accelerator is each title's
/// first letter).
pub fn titles() -> [String; 5] {
    TITLES.map(crate::l10n::tr)
}

/// Render the top menu bar. `show_hotkeys` accents the accelerator letters
/// — shown only while the menu is active or Alt arms it.
pub fn render(f: &mut Frame, area: Rect, theme: &Theme, show_hotkeys: bool) {
    let width = area.width as usize;
    let mut text = String::from(" ");
    // Char position of each title's first letter — its hotkey.
    let mut hotkeys: Vec<usize> = Vec::new();
    for title in titles() {
        hotkeys.push(text.chars().count() + 1); // +1 for the segment's leading space
        text.push_str(&format!(" {title} "));
    }
    while text.chars().count() < width {
        text.push(' ');
    }
    let is_hot = |i: usize| show_hotkeys && hotkeys.contains(&i);

    // A gradient (truecolor) or two-tone bar, with hotkey letters accented.
    let spans: Vec<Span> = text
        .chars()
        .take(width)
        .enumerate()
        .map(|(i, ch)| {
            let style = if theme.truecolor {
                let base = Style::default().bg(theme.gradient_at(i, width));
                if is_hot(i) {
                    base.fg(theme.hotkey_fg).add_modifier(Modifier::BOLD)
                } else {
                    base.fg(theme.bar_fg)
                }
            } else if is_hot(i) {
                theme.menubar.fg(theme.hotkey_fg).add_modifier(Modifier::BOLD)
            } else {
                theme.menubar
            };
            Span::styled(ch.to_string(), style)
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(spans)), area);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn hotkey_cells(show: bool) -> usize {
        // Classic MC paints accelerators in a distinct yellow, easy to count.
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(60, 1)).unwrap();
        t.draw(|f| render(f, f.area(), &theme, show)).unwrap();
        let b = t.backend().buffer();
        (0..b.area.width).filter(|&x| b[(x, 0)].fg == theme.hotkey_fg).count()
    }

    #[test]
    fn hotkeys_show_only_when_requested() {
        assert_eq!(hotkey_cells(false), 0, "closed/idle bar has no accents");
        assert_eq!(hotkey_cells(true), 5, "armed bar accents L/F/C/O/R");
    }
}
