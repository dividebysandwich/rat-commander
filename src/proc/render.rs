//! Rendering of the [`ProcView`] full-screen process explorer.

use super::{ProcSort, ProcView};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::util::text::{ellipsize, pad_left, pad_right};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, BorderType, Borders, Chart, Dataset, GraphType, Paragraph};

/// Block glyphs for fractional vertical-bar cells (0/8 .. 8/8 filled).
const LEVELS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn render(f: &mut Frame, area: Rect, pv: &mut ProcView, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.panel_border_active).bg(theme.panel_bg))
        .title(Span::styled(
            format!(" Process Explorer — {} processes ", pv.procs.len()),
            Style::default()
                .fg(theme.panel_border_active)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 10 || inner.height < 6 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // graphs
            Constraint::Min(3),     // process table
            Constraint::Length(1),  // footer
        ])
        .split(inner);

    render_graphs(f, rows[0], pv, theme);
    render_table(f, rows[1], pv, theme);
    render_footer(f, rows[2], pv, theme);
}

fn render_graphs(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Percentage(42),
            Constraint::Percentage(20),
        ])
        .split(area);

    render_cpu_chart(f, cols[0], pv, theme);
    render_cores(f, cols[1], pv, theme);
    render_memory(f, cols[2], pv, theme);
}

/// Sub-block with a title; returns its interior rect.
fn titled(f: &mut Frame, area: Rect, title: String, theme: &Theme) -> Rect {
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.panel_border).bg(theme.panel_bg))
        .title(Span::styled(
            title,
            Style::default().fg(theme.header_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let inner = b.inner(area);
    f.render_widget(b, area);
    inner
}

fn render_cpu_chart(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme) {
    let inner = titled(f, area, format!(" CPU  {:>3.0}% ", pv.cpu_last()), theme);
    if inner.width < 2 || inner.height < 2 {
        return;
    }
    if pv.cpu_history.len() < 2 {
        f.render_widget(
            Paragraph::new(Line::from("  measuring…")).style(theme.panel_base()),
            inner,
        );
        return;
    }
    let data: Vec<(f64, f64)> = pv
        .cpu_history
        .iter()
        .enumerate()
        .map(|(i, &v)| (i as f64, v as f64))
        .collect();
    let x_max = (data.len() - 1).max(1) as f64;
    // Animated truecolor line color (shifts over time), else a solid accent.
    let line_color = theme.gradient_at(0, 1);
    let base = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let datasets = vec![Dataset::default()
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(line_color).bg(theme.panel_bg))
        .data(&data)];
    let chart = Chart::new(datasets)
        .style(theme.panel_base())
        .x_axis(Axis::default().style(base).bounds([0.0, x_max]))
        .y_axis(
            Axis::default()
                .style(base)
                .bounds([0.0, 100.0])
                .labels([Span::raw("0"), Span::raw("100")]),
        );
    f.render_widget(chart, inner);
}

fn render_cores(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme) {
    let inner = titled(f, area, format!(" Cores ({}) ", pv.ncores), theme);
    if pv.cores.is_empty() || inner.height == 0 || inner.width < 8 {
        f.render_widget(
            Paragraph::new(Line::from("  n/a")).style(theme.panel_base()),
            inner,
        );
        return;
    }

    // btop-style grid: one core per row, filling each column top-to-bottom.
    let n = pv.cores.len();
    let rows = inner.height as usize;
    let cols = n.div_ceil(rows).max(1);
    let cell_w = (inner.width as usize / cols).max(1);
    let label_w = format!("C{}", n.saturating_sub(1)).len();

    for c in 0..n {
        let col = c / rows;
        let row = c % rows;
        if col >= cols {
            break;
        }
        let cell = Rect {
            x: inner.x + (col * cell_w) as u16,
            y: inner.y + row as u16,
            width: cell_w as u16,
            height: 1,
        };
        let hist = pv.core_history.get(c);
        draw_core_cell(f, cell, c, pv.cores[c], label_w, hist, theme);
    }
}

/// One "C{n} <sparkline> NN%" core meter, btop-style.
fn draw_core_cell(
    f: &mut Frame,
    cell: Rect,
    idx: usize,
    value: f32,
    label_w: usize,
    history: Option<&std::collections::VecDeque<f32>>,
    theme: &Theme,
) {
    let w = cell.width as usize;
    let label = format!("C{idx}");
    let pct = format!("{:>3.0}%", value);
    // Layout: label + space + graph + space + pct.
    let fixed = label_w + 1 + 1 + pct.chars().count();
    let graph_w = w.saturating_sub(fixed);

    let buf = f.buffer_mut();
    let label_style = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    buf.set_string(
        cell.x,
        cell.y,
        format!("{label:<label_w$}"),
        label_style,
    );

    // Sparkline of recent load, right-aligned (latest at the right).
    let gx = cell.x + (label_w + 1) as u16;
    if graph_w > 0 {
        let mut samples = vec![0.0f32; graph_w];
        if let Some(h) = history {
            let take = h.len().min(graph_w);
            for k in 0..take {
                samples[graph_w - take + k] = h[h.len() - take + k];
            }
        }
        for (gi, &s) in samples.iter().enumerate() {
            let level = ((s.clamp(0.0, 100.0) / 100.0) * 8.0).round() as usize;
            // Idle samples render as a faint dotted baseline (btop style); any
            // load shows as a colored block scaled to the value.
            let (ch, color) = if level == 0 {
                ('·', theme.panel_border)
            } else {
                (LEVELS[level.min(8)], load_color(s, theme))
            };
            let style = Style::default().fg(color).bg(theme.panel_bg);
            buf.set_string(gx + gi as u16, cell.y, ch.to_string(), style);
        }
    }

    // Percentage, load-colored.
    let px = cell.x + (w.saturating_sub(pct.chars().count())) as u16;
    buf.set_string(
        px,
        cell.y,
        pct,
        Style::default().fg(load_color(value, theme)).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
    );
}

fn render_memory(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme) {
    let pct = if pv.mem_total > 0 {
        100.0 * pv.mem_used as f32 / pv.mem_total as f32
    } else {
        0.0
    };
    let inner = titled(f, area, format!(" Mem {pct:>3.0}% "), theme);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let label = format!("{} / {}", human_size(pv.mem_used), human_size(pv.mem_total));
    draw_hbar(f, inner, pct, &label, theme);
}

/// A horizontal bar-graph meter filling `area` to `pct`, with `label` overlaid
/// on the middle row. The filled portion uses a truecolor load color.
fn draw_hbar(f: &mut Frame, area: Rect, pct: f32, label: &str, theme: &Theme) {
    let w = area.width as usize;
    if w == 0 || area.height == 0 {
        return;
    }
    let filled = ((pct.clamp(0.0, 100.0) / 100.0) * w as f32).round() as usize;
    let fill = Style::default().fg(load_color(pct, theme)).bg(theme.panel_bg);
    let empty = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let mid = area.height / 2;
    let label = ellipsize(label, w);
    let label_start = (w.saturating_sub(label.chars().count())) / 2;
    let label_chars: Vec<char> = label.chars().collect();

    let buf = f.buffer_mut();
    for row in 0..area.height {
        for col in 0..w {
            // Overlay the label on the middle row.
            let lbl_idx = col.checked_sub(label_start).filter(|&i| i < label_chars.len());
            let (ch, style) = if row == mid && let Some(i) = lbl_idx {
                let st = if col < filled {
                    Style::default()
                        .fg(theme.panel_bg)
                        .bg(load_color(pct, theme))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.panel_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD)
                };
                (label_chars[i], st)
            } else if col < filled {
                ('█', fill)
            } else {
                ('░', empty)
            };
            buf.set_string(area.x + col as u16, area.y + row, ch.to_string(), style);
        }
    }
}

/// A green→yellow→red color reflecting a 0..=100 load value (truecolor only).
fn load_color(pct: f32, theme: &Theme) -> ratatui::style::Color {
    use ratatui::style::Color;
    if !theme.truecolor {
        return theme.bar_fg;
    }
    let t = (pct / 100.0).clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.5 {
        lerp3((40, 200, 140), (220, 200, 40), t / 0.5)
    } else {
        lerp3((220, 200, 40), (230, 60, 50), (t - 0.5) / 0.5)
    };
    Color::Rgb(r, g, b)
}

fn lerp3(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

fn render_table(f: &mut Frame, area: Rect, pv: &mut ProcView, theme: &Theme) {
    if area.height < 2 {
        return;
    }
    let width = area.width as usize;
    // Column widths: PID, CPU%, MEM, MEM%, then the name fills the rest.
    let pid_w = 7;
    let cpu_w = 7;
    let mem_w = 9;
    let memp_w = 6;
    let name_w = width.saturating_sub(pid_w + cpu_w + mem_w + memp_w + 4).max(4);

    let arrow = |k: ProcSort| -> &'static str {
        if pv.sort == k {
            if pv.reverse { " ▼" } else { " ▲" }
        } else {
            ""
        }
    };
    // Bracketed letters show the sort hotkey, e.g. [C]PU%; the active column
    // also gets a ▼/▲ direction arrow.
    let pid_h = format!("[P]ID{}", arrow(ProcSort::Pid));
    let cpu_h = format!("[C]PU%{}", arrow(ProcSort::Cpu));
    let mem_h = format!("[M]EM{}", arrow(ProcSort::Mem));
    let name_h = pad_right("[N]AME", name_w);
    let header = format!(
        "{}{}{}{}  {}",
        pad_left(&pid_h, pid_w),
        pad_left(&cpu_h, cpu_w + 1),
        pad_left(&mem_h, mem_w + 1),
        pad_left("MEM%", memp_w + 1),
        name_h,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            header,
            Style::default()
                .fg(theme.header_fg)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ))),
        Rect { height: 1, ..area },
    );

    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let rows = body.height as usize;
    pv.view_rows = rows;
    // Keep the cursor visible.
    if pv.cursor < pv.offset {
        pv.offset = pv.cursor;
    } else if pv.cursor >= pv.offset + rows {
        pv.offset = pv.cursor + 1 - rows;
    }

    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for i in 0..rows {
        let idx = pv.offset + i;
        let Some(p) = pv.procs.get(idx) else {
            lines.push(Line::from(""));
            continue;
        };
        let text = format!(
            "{}{}{}{}  {}",
            pad_left(&p.pid.to_string(), pid_w),
            pad_left(&format!("{:.1}", p.cpu), cpu_w + 1),
            pad_left(&human_size(p.rss), mem_w + 1),
            pad_left(&format!("{:.1}", p.mem_pct), memp_w + 1),
            pad_right(&ellipsize(&p.name, name_w), name_w),
        );
        if idx == pv.cursor {
            lines.push(Line::from(Span::styled(text, theme.cursor)));
        } else {
            lines.push(Line::from(Span::styled(text, normal)));
        }
    }
    f.render_widget(Paragraph::new(lines), body);
}

fn render_footer(f: &mut Frame, area: Rect, _pv: &ProcView, theme: &Theme) {
    let hint = "↑↓ PgUp/Dn move   c CPU  m Mem  n Name  p PID   r reverse   k kill  K force   Esc close";
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {hint}"),
            Style::default().fg(theme.fkey_label.fg.unwrap_or(theme.panel_fg)).bg(theme.panel_bg),
        ))),
        area,
    );
}
