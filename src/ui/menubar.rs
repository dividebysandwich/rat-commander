//! The top menu bar row: the static backdrop the interactive pulldown
//! ([`crate::ui::pulldown`]) draws its open menu over, plus the two widgets
//! that share the row (background transfers and the system-status readout).

use crate::ui::theme::{GradRole, GradZone, Theme};
use crate::util::sysinfo::SysSampler;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

/// Minimum terminal width before the system-status widget is shown.
pub const STATUS_MIN_WIDTH: u16 = 100;
/// Width reserved for the system-status widget.
pub const STATUS_WIDTH: u16 = 36;

pub const TITLES: [&str; 5] = ["Left", "File", "Command", "Options", "Right"];

/// The menu-bar titles in the active language (the accelerator is each title's
/// first letter).
pub fn titles() -> [String; 5] {
    TITLES.map(crate::l10n::tr)
}

/// Render the top menu bar. `show_hotkeys` accents the accelerator letters
/// — shown only while the menu is active or Alt arms it.
pub fn render(f: &mut Frame, area: Rect, theme: &Theme, show_hotkeys: bool) {
    render_titles(f, area, theme, &titles(), show_hotkeys);
}

/// Render a full-width menu bar carrying `bar_titles` — the file manager's own
/// (via [`render`]) or the editor's. The bar is drawn as a gradient (its own, or
/// the theme's accent ramp on truecolor) or a two-tone row, with each title's
/// first letter accented when `show_hotkeys`. The interactive pulldown paints
/// its highlighted title over the result, so the gradient still shows through
/// either side of it.
pub fn render_titles(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    bar_titles: &[String],
    show_hotkeys: bool,
) {
    let width = area.width as usize;
    // Claim the row, so a body gradient can't repaint the bar (and the bar's own
    // can't reach past it) when the two share a color. The cells below carry the
    // bar's ramp already, so the screen pass leaves them be.
    crate::ui::gradient::mark_zone(GradZone::Menubar, area);
    crate::ui::gradient::mark_painted(area);
    // In RTL the reshaped title reads right-to-left, so the first-letter hotkey
    // accent no longer lines up — skip it (the accelerator key still works).
    let rtl = crate::l10n::active_is_rtl();
    let mut text = String::from(" ");
    // Char position of each title's first letter — its hotkey.
    let mut hotkeys: Vec<usize> = Vec::new();
    for title in bar_titles {
        if !rtl {
            hotkeys.push(text.chars().count() + 1); // +1 for the segment's leading space
        }
        text.push_str(&format!(" {} ", crate::l10n::display(title)));
    }
    while text.chars().count() < width {
        text.push(' ');
    }
    let is_hot = |i: usize| show_hotkeys && hotkeys.contains(&i);

    // A gradient or two-tone bar, with hotkey letters accented.
    let spans: Vec<Span> = text
        .chars()
        .take(width)
        .enumerate()
        .map(|(i, ch)| {
            let style = match theme.bar_bg(GradRole::MenubarBg, i, width) {
                Some(bg) => Style::default().bg(bg).fg(theme.bar_fg),
                None => theme.menubar,
            };
            let style = if is_hot(i) {
                style.fg(theme.hotkey_fg).add_modifier(Modifier::BOLD)
            } else {
                style
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
pub fn render_mini_progress(
    f: &mut Frame,
    area: Rect,
    done: u64,
    total: u64,
    count: usize,
    theme: &Theme,
) {
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

/// Render the CPU-histogram + memory status widget into `area` (one row):
/// `CPU ▁▂▁▃… nn%  MEM nn%`. Labels share one colour and figures another, so
/// the two read as a pair; the CPU figure and each bar take their load colour.
pub fn render_status(f: &mut Frame, area: Rect, s: &SysSampler, theme: &Theme) {
    if area.width < 20 {
        return;
    }
    // Opaque background so the widget reads over the gradient bar.
    f.render_widget(Block::default().style(Style::default().bg(theme.panel_bg)), area);

    let cpu_label_w: u16 = 4; // "CPU "
    let cpu_pct_w: u16 = 5; // " nnn%"
    let mem_w: u16 = 10; // "  MEM nnn%"
    let spark_w = area.width.saturating_sub(cpu_label_w + cpu_pct_w + mem_w);

    let label = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let figure = Style::default().fg(theme.panel_border_active).bg(theme.panel_bg);
    let load_color = |v: u64| {
        if v >= 80 {
            theme.error_fg
        } else if v >= 50 {
            theme.header_fg
        } else {
            theme.exec_fg
        }
    };
    let (x, y) = (area.x, area.y);
    let buf = f.buffer_mut();
    buf.set_string(x, y, "CPU ", label);

    // One column per sample, newest at the right edge. A sample too small for
    // even the lowest bar (and a column with no sample yet) draws a dim `▁`
    // track, so an idle machine reads as idle rather than as missing data.
    const BARS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let track = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let w = spark_w as usize;
    let skip = s.cpu_history.len().saturating_sub(w);
    let pad = w.saturating_sub(s.cpu_history.len());
    for col in 0..w {
        let cx = x + cpu_label_w + col as u16;
        let sample = col.checked_sub(pad).and_then(|i| s.cpu_history.get(skip + i)).copied();
        let level = sample.map_or(0, |v| (v.min(100) * 8).div_ceil(100) as usize);
        match (sample, level) {
            (Some(v), 1..) => {
                let style = Style::default().fg(load_color(v)).bg(theme.panel_bg);
                buf.set_string(cx, y, BARS[level - 1], style);
            }
            _ => {
                buf.set_string(cx, y, BARS[0], track);
            }
        }
    }

    let load = s.cpu_last();
    let pct_x = x + cpu_label_w + spark_w;
    buf.set_string(pct_x, y, format!("{load:>4}%"), figure.fg(load_color(load)));
    let mem_x = pct_x + cpu_pct_w;
    buf.set_string(mem_x, y, "  MEM", label);
    buf.set_string(mem_x + 5, y, format!("{:>4}%", s.mem_percent()), figure);
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

    fn status_buffer(s: &SysSampler) -> (ratatui::buffer::Buffer, crate::ui::theme::Theme) {
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(STATUS_WIDTH, 1)).unwrap();
        t.draw(|f| render_status(f, f.area(), s, &theme)).unwrap();
        (t.backend().buffer().clone(), theme)
    }

    fn row_text(b: &ratatui::buffer::Buffer) -> String {
        (0..b.area.width).map(|x| b[(x, 0u16)].symbol().to_string()).collect()
    }

    /// The CPU sparkline anchors the *newest* sample at its right edge — older
    /// (leftmost) samples must not eclipse a fresh spike, which was the bug.
    #[test]
    fn cpu_sparkline_shows_newest_at_the_right() {
        let mut s = SysSampler::new();
        // Low history with a fresh 100% spike as the newest (back) sample.
        for _ in 0..crate::util::sysinfo::HISTORY - 1 {
            s.cpu_history.push_back(0);
        }
        s.cpu_history.push_back(100);

        // Sparkline occupies x = 4 .. STATUS_WIDTH - 15 (label 4, pct 5, mem 10).
        let (b, _) = status_buffer(&s);
        let last = STATUS_WIDTH - 16;
        assert_eq!(b[(last, 0u16)].symbol(), "█", "the 100% spike is a full bar at the right");
        assert_ne!(b[(4u16, 0u16)].symbol(), "█", "older (0%) samples stay low on the left");
        assert!(row_text(&b).contains(" 100%"), "the current load is shown as a figure");
    }

    /// An idle machine (and one with no samples yet) still draws a visible
    /// track and a figure, rather than a blank gap after "CPU".
    #[test]
    fn idle_cpu_draws_a_track_and_a_percentage() {
        let mut s = SysSampler::new();
        for _ in 0..5 {
            s.cpu_history.push_back(2);
        }
        let (b, theme) = status_buffer(&s);
        let spark: Vec<_> = (4..STATUS_WIDTH - 15).map(|x| &b[(x, 0u16)]).collect();
        assert!(spark.iter().all(|c| c.symbol() == "▁"), "{}", row_text(&b));
        assert!(spark.iter().any(|c| c.fg == theme.exec_fg), "small loads still show");
        assert!(spark.iter().any(|c| c.fg == theme.panel_border), "empty columns are a dim track");
        assert!(row_text(&b).contains("   2%"), "{}", row_text(&b));

        let (b, _) = status_buffer(&SysSampler::new());
        assert!(row_text(&b).starts_with("CPU ▁▁▁"), "{}", row_text(&b));
    }

    /// "CPU" and "MEM" share the label colour, the two figures the figure colour.
    #[test]
    fn cpu_and_mem_labels_match() {
        let mut s = SysSampler::new();
        s.mem_used_kb = 14;
        s.mem_total_kb = 100;
        let (b, theme) = status_buffer(&s);
        let text = row_text(&b);
        let cpu = text.find("CPU").unwrap() as u16;
        let mem = text.chars().collect::<Vec<_>>().iter().position(|&c| c == 'M').unwrap() as u16;
        assert_eq!(b[(cpu, 0u16)].fg, theme.panel_fg);
        assert_eq!(b[(mem, 0u16)].fg, theme.panel_fg);
        let pct = STATUS_WIDTH - 3; // the "14" of " 14%"
        assert_eq!(b[(pct, 0u16)].fg, theme.panel_border_active);
        assert!(text.ends_with("MEM  14%"), "{text}");
    }
}
