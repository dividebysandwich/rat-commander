//! Rendering of the [`ProcView`] full-screen process explorer.

use super::{ProcSort, ProcView};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::util::text::{ellipsize, pad_left, pad_right};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
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
            Constraint::Percentage(46),
            Constraint::Percentage(32),
            Constraint::Percentage(22),
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
    if pv.cores.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from("  n/a")).style(theme.panel_base()),
            inner,
        );
        return;
    }
    draw_vbars(f, inner, &pv.cores, theme);
}

fn render_memory(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme) {
    let pct = if pv.mem_total > 0 {
        100.0 * pv.mem_used as f32 / pv.mem_total as f32
    } else {
        0.0
    };
    let inner = titled(
        f,
        area,
        format!(" Mem {:>3.0}% ", pct),
        theme,
    );
    if inner.height < 1 {
        return;
    }
    // Reserve the bottom row for the used/total label, bar fills the rest.
    let bar = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    draw_vbars(f, bar, &[pct], theme);
    let label = format!("{}/{}", human_size(pv.mem_used), human_size(pv.mem_total));
    let label_row = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    f.render_widget(
        Paragraph::new(Line::from(ellipsize(&label, inner.width as usize)))
            .alignment(Alignment::Center)
            .style(theme.panel_base()),
        label_row,
    );
}

/// Draw a row of vertical bars (one per value, 0..=100), with truecolor
/// gradient coloring across the bars.
fn draw_vbars(f: &mut Frame, area: Rect, values: &[f32], theme: &Theme) {
    if area.width == 0 || area.height == 0 || values.is_empty() {
        return;
    }
    let n = values.len().min(area.width as usize).max(1);
    let cw = (area.width as usize / n).max(1);
    let h = area.height as usize;
    let buf = f.buffer_mut();
    for (ci, &val) in values.iter().take(n).enumerate() {
        let filled = ((val.clamp(0.0, 100.0) / 100.0) * (h as f32 * 8.0)).round() as usize;
        let color = theme.gradient_at(ci, n);
        let style = Style::default().fg(color).bg(theme.panel_bg);
        let x0 = area.x + (ci * cw) as u16;
        for row in 0..h {
            let from_bottom = h - 1 - row;
            let eighths = filled.saturating_sub(from_bottom * 8).min(8);
            let cell: String = std::iter::repeat_n(LEVELS[eighths], cw).collect();
            buf.set_string(x0, area.y + row as u16, cell, style);
        }
    }
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
    let pid_h = format!("PID{}", arrow(ProcSort::Pid));
    let cpu_h = format!("CPU%{}", arrow(ProcSort::Cpu));
    let mem_h = format!("MEM{}", arrow(ProcSort::Mem));
    let name_h = pad_right("NAME", name_w);
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
