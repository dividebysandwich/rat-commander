//! The top menu bar. Phase 1 renders it statically; Phase 2 makes the
//! pulldowns interactive (F9).

use crate::ui::theme::Theme;
use crate::util::sysinfo::SysSampler;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, RenderDirection, Sparkline};

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
    // In RTL the reshaped title reads right-to-left, so the first-letter hotkey
    // accent no longer lines up — skip it (the accelerator key still works).
    let rtl = crate::l10n::active_is_rtl();
    let mut text = String::from(" ");
    // Char position of each title's first letter — its hotkey.
    let mut hotkeys: Vec<usize> = Vec::new();
    for title in titles() {
        if !rtl {
            hotkeys.push(text.chars().count() + 1); // +1 for the segment's leading space
        }
        text.push_str(&format!(" {} ", crate::l10n::display(&title)));
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

/// Render the mini background-transfer progress bar into `area` (one row on the
/// menu bar): a compact gauge over an opaque panel background, labelled with the
/// number of running operations and the aggregate percentage. `done`/`total` are
/// bytes across all background transfers; `count` is how many are running.
pub fn render_mini_progress(f: &mut Frame, area: Rect, done: u64, total: u64, count: usize, theme: &Theme) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    // Opaque background so the bar reads over the gradient menu bar.
    f.render_widget(Block::default().style(Style::default().bg(theme.panel_bg)), area);

    let ratio = if total > 0 { (done as f64 / total as f64).clamp(0.0, 1.0) } else { 0.0 };
    let label = format!("{count} op{}  {:.0}%", if count == 1 { "" } else { "s" }, ratio * 100.0);
    let w = area.width as usize;
    let filled = (ratio * w as f64).round() as usize;
    let label_chars: Vec<char> = label.chars().take(w).collect();
    let lstart = (w - label_chars.len()) / 2;
    let fill_color = theme.panel_border_active;
    let buf = f.buffer_mut();
    for x in 0..w {
        let in_label = x >= lstart && x < lstart + label_chars.len();
        let lc = if in_label { Some(label_chars[x - lstart]) } else { None };
        let (ch, fg, bg) = if x < filled {
            match lc {
                Some(c) => (c, theme.panel_bg, fill_color),
                None => ('█', fill_color, theme.panel_bg),
            }
        } else {
            match lc {
                Some(c) => (c, theme.panel_fg, theme.panel_bg),
                None => ('░', theme.panel_border, theme.panel_bg),
            }
        };
        buf.set_string(area.x + x as u16, area.y, ch.to_string(), Style::default().fg(fg).bg(bg));
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
    // Feed the samples newest-first and draw right-to-left, so the most recent
    // load sits at the right edge and older samples scroll off to the left. (The
    // widget only draws `width` bars from the front of the data; feeding it
    // oldest-first drew the *oldest* bars and hid the newest until they aged in.)
    let data: Vec<u64> = s.cpu_history.iter().rev().copied().collect();
    let spark = Sparkline::default()
        .data(data)
        .direction(RenderDirection::RightToLeft)
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

    /// The CPU sparkline anchors the *newest* sample at its right edge — older
    /// (leftmost) samples must not eclipse a fresh spike, which was the bug.
    #[test]
    fn cpu_sparkline_shows_newest_at_the_right() {
        let theme = crate::ui::theme::Theme::mc();
        let mut s = crate::util::sysinfo::SysSampler::new();
        // Low history with a fresh 100% spike as the newest (back) sample.
        for _ in 0..crate::util::sysinfo::HISTORY - 1 {
            s.cpu_history.push_back(0);
        }
        s.cpu_history.push_back(100);

        // Width 34 = STATUS_WIDTH: sparkline occupies x = 5 .. 25 (label 5, mem 9).
        let mut t = Terminal::new(TestBackend::new(34, 1)).unwrap();
        t.draw(|f| render_status(f, f.area(), &s, &theme)).unwrap();
        let b = t.backend().buffer();
        let rightmost = b[(24u16, 0u16)].symbol().to_string(); // last sparkline cell
        let leftmost = b[(5u16, 0u16)].symbol().to_string(); // oldest visible cell
        assert_eq!(rightmost, "█", "the 100% spike renders as a full bar at the right edge");
        assert_ne!(leftmost, "█", "older (0%) samples stay low on the left");
    }
}
