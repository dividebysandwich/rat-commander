//! Rendering of the [`ProcView`] full-screen process explorer.

use super::{ProcMode, ProcSort, ProcView};
use crate::ui::graphics::{raster, Gfx, Slot};
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

pub fn render(f: &mut Frame, area: Rect, pv: &mut ProcView, theme: &Theme, mut gfx: Option<&mut Gfx>) {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.panel_border_active).bg(theme.panel_bg))
        .title(Span::styled(
            format!(" {} — {} {} ", crate::l10n::trd("Process Explorer"), pv.procs.len(), crate::l10n::trd("processes")),
            Style::default()
                .fg(theme.panel_border_active)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ))
        // Update interval on the top-right border.
        .title(
            Line::from(Span::styled(
                format!(" {}ms ", pv.interval_ms),
                Style::default()
                    .fg(theme.panel_border_active)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ))
            .right_aligned(),
        )
        .style(theme.panel_base());
    // Battery (percentage + mini bar) centered on the top border, if present.
    if let Some((pct, charging)) = pv.battery {
        block = block.title(battery_title(pct, charging, theme));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 10 || inner.height < 6 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // CPU graph + per-core meters
            Constraint::Min(3),     // sys panels (left) + process table (right)
            Constraint::Length(1),  // footer
        ])
        .split(inner);

    render_graphs(f, rows[0], pv, theme, gfx.as_deref_mut());
    render_body(f, rows[1], pv, theme, gfx);
    render_footer(f, rows[2], pv, theme);
}

fn render_graphs(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme, mut gfx: Option<&mut Gfx>) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    render_cpu_chart(f, cols[0], pv, theme, gfx.as_deref_mut());
    render_cores(f, cols[1], pv, theme, gfx);
}

/// The lower region: stacked memory/disk/network sparkline panels on the left,
/// the process table on the right (btop-style).
fn render_body(f: &mut Frame, area: Rect, pv: &mut ProcView, theme: &Theme, gfx: Option<&mut Gfx>) {
    // Reserve a sensible fixed width for the sys panels when there's room.
    let left_w = (area.width / 3).clamp(18, 40);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_w), Constraint::Min(24)])
        .split(area);

    render_sys_panels(f, cols[0], pv, theme, gfx);
    render_table(f, cols[1], pv, theme);
}

/// Stack the memory, disk and network sparkline panels in the left column.
fn render_sys_panels(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme, mut gfx: Option<&mut Gfx>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(area);
    render_mem_panel(f, rows[0], pv, theme, gfx.as_deref_mut());
    render_disk_panel(f, rows[1], pv, theme, gfx.as_deref_mut());
    render_net_panel(f, rows[2], pv, theme, gfx);
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

fn render_cpu_chart(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme, gfx: Option<&mut Gfx>) {
    let inner = titled(f, area, format!(" CPU  {:>3.0}% ", pv.cpu_last()), theme);
    if inner.width < 2 || inner.height < 2 {
        return;
    }
    if pv.cpu_history.len() < 2 {
        f.render_widget(
            Paragraph::new(Line::from(format!("  {}", crate::l10n::trd("measuring…")))).style(theme.panel_base()),
            inner,
        );
        return;
    }

    // Graphics path: a smooth filled line graph with the animated theme gradient.
    if let Some(g) = gfx
        && g.available() {
            let samples: Vec<f64> = pv.cpu_history.iter().map(|&v| v as f64).collect();
            let (pw, ph) = g.px_size(inner);
            let img = raster::line_graph(
                pw,
                ph,
                &samples,
                100.0,
                |t| theme.gradient_rgb(t),
                raster::rgb(theme.panel_bg),
            );
            g.draw(f, inner, Slot::ProcCpu, img);
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

fn render_cores(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme, mut gfx: Option<&mut Gfx>) {
    // Show the CPU model name on the panel border (falling back to "Cores").
    let title = if pv.cpu_name.is_empty() {
        format!(" {} ({}) ", crate::l10n::trd("Cores"), pv.ncores)
    } else {
        format!(" {} ({} {}) ", pv.cpu_name, pv.ncores, crate::l10n::trd("cores"))
    };
    let inner = titled(f, area, title, theme);
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
        draw_core_cell(f, cell, c, pv.cores[c], label_w, hist, theme, gfx.as_deref_mut());
    }
}

/// One "C{n} <sparkline> NN%" core meter, btop-style.
#[allow(clippy::too_many_arguments)]
fn draw_core_cell(
    f: &mut Frame,
    cell: Rect,
    idx: usize,
    value: f32,
    label_w: usize,
    history: Option<&std::collections::VecDeque<f32>>,
    theme: &Theme,
    gfx: Option<&mut Gfx>,
) {
    let w = cell.width as usize;
    let label = format!("C{idx}");
    let pct = format!("{:>3.0}%", value);
    // Layout: label + space + graph + space + pct.
    let fixed = label_w + 1 + 1 + pct.chars().count();
    let graph_w = w.saturating_sub(fixed);
    let gx = cell.x + (label_w + 1) as u16;
    let px = cell.x + (w.saturating_sub(pct.chars().count())) as u16;

    // Label and percentage are text in columns disjoint from the graph.
    {
        let buf = f.buffer_mut();
        buf.set_string(
            cell.x,
            cell.y,
            format!("{label:<label_w$}"),
            Style::default().fg(theme.header_fg).bg(theme.panel_bg),
        );
        buf.set_string(
            px,
            cell.y,
            pct,
            Style::default().fg(load_color(value, theme)).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        );
    }

    if graph_w == 0 {
        return;
    }
    let graph_rect = Rect { x: gx, y: cell.y, width: graph_w as u16, height: 1 };

    // Graphics path: a filled load-colored sparkline over the graph columns.
    if let Some(g) = gfx
        && g.available() {
            let hist: Vec<f64> =
                history.map(|h| h.iter().map(|&v| v as f64).collect()).unwrap_or_default();
            let (pw, ph) = g.px_size(graph_rect);
            let img = raster::area_spark(
                pw,
                ph,
                &hist,
                100.0,
                |v| raster::load_rgb(v * 100.0),
                raster::rgb(theme.panel_bg),
            );
            g.draw(f, graph_rect, Slot::ProcCore(idx as u16), img);
            return;
        }

    // Cell fallback: recent load as block glyphs, right-aligned (latest right).
    let mut samples = vec![0.0f32; graph_w];
    if let Some(h) = history {
        let take = h.len().min(graph_w);
        for k in 0..take {
            samples[graph_w - take + k] = h[h.len() - take + k];
        }
    }
    let buf = f.buffer_mut();
    for (gi, &s) in samples.iter().enumerate() {
        let level = ((s.clamp(0.0, 100.0) / 100.0) * 8.0).round() as usize;
        // Idle samples render as a faint dotted baseline (btop style); any
        // load shows as a colored block scaled to the value.
        let (ch, color) = if level == 0 {
            ('·', theme.panel_border)
        } else {
            (LEVELS[level.min(8)], load_color(s, theme))
        };
        buf.set_string(gx + gi as u16, cell.y, ch.to_string(), Style::default().fg(color).bg(theme.panel_bg));
    }
}

fn render_mem_panel(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme, gfx: Option<&mut Gfx>) {
    let pct = if pv.mem_total > 0 {
        100.0 * pv.mem_used as f32 / pv.mem_total as f32
    } else {
        0.0
    };
    let title = format!(
        " Mem {pct:.0}%  {}/{} ",
        human_size(pv.mem_used),
        human_size(pv.mem_total)
    );
    let inner = titled(f, area, title, theme);
    // Memory sparkline is load-colored (green→red) by each sample's value.
    let samples: Vec<f64> = pv.mem_history.iter().copied().collect();
    draw_sparkline(f, inner, &samples, 100.0, &|v| load_color(v as f32, theme), Slot::ProcMem, theme, gfx);
}

fn render_disk_panel(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme, gfx: Option<&mut Gfx>) {
    // ▲ writes (grow upward), ▼ reads (grow downward) from the centre line.
    let title = format!(
        " Disk ▼{}/s ▲{}/s ",
        human_size(pv.disk_read as u64),
        human_size(pv.disk_write as u64)
    );
    let inner = titled(f, area, title, theme);
    let read: Vec<f64> = pv.disk_read_history.iter().copied().collect();
    let write: Vec<f64> = pv.disk_write_history.iter().copied().collect();
    // Shared scale so reads and writes are directly comparable.
    let max = peak(&read).max(peak(&write));
    draw_mirror_bars(f, inner, (&write, theme.header_fg), (&read, theme.exec_fg), max, Slot::ProcDisk, theme, gfx);
}

fn render_net_panel(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme, gfx: Option<&mut Gfx>) {
    // ▲ uploads (grow upward), ▼ downloads (grow downward) from the centre line.
    let title = format!(
        " Net ▼{}/s ▲{}/s ",
        human_size(pv.net_down as u64),
        human_size(pv.net_up as u64)
    );
    let inner = titled(f, area, title, theme);
    let down: Vec<f64> = pv.net_down_history.iter().copied().collect();
    let up: Vec<f64> = pv.net_up_history.iter().copied().collect();
    // Shared scale so the upload/download halves are directly comparable.
    let max = peak(&down).max(peak(&up));
    draw_mirror_bars(f, inner, (&up, theme.header_fg), (&down, theme.panel_border_active), max, Slot::ProcNet, theme, gfx);
}

/// The peak of `samples`, floored at 1.0 so a flat/empty series doesn't divide
/// by zero and stays drawn near the baseline.
fn peak(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(1.0, f64::max)
}

/// Draw a multi-row block-glyph sparkline filling `area`, newest sample at the
/// right edge, each bar scaled to `max` and colored by `colorer(value)`.
#[allow(clippy::too_many_arguments)]
fn draw_sparkline(
    f: &mut Frame,
    area: Rect,
    samples: &[f64],
    max: f64,
    colorer: &dyn Fn(f64) -> ratatui::style::Color,
    slot: Slot,
    theme: &Theme,
    gfx: Option<&mut Gfx>,
) {
    let (w, h) = (area.width as usize, area.height as usize);
    if w == 0 || h == 0 {
        return;
    }

    // Graphics path: a filled area sparkline colored per sample by `colorer`.
    if let Some(g) = gfx
        && g.available() {
            let (pw, ph) = g.px_size(area);
            let img = raster::area_spark(
                pw,
                ph,
                samples,
                max,
                |v| raster::rgb(colorer(v * max)),
                raster::rgb(theme.panel_bg),
            );
            g.draw(f, area, slot, img);
            return;
        }

    let levels = h * 8;
    let n = samples.len();
    let buf = f.buffer_mut();
    for col in 0..w {
        // Right-align: the rightmost column is the newest sample.
        let from_right = w - 1 - col;
        let v = if from_right < n { samples[n - 1 - from_right] } else { 0.0 };
        let frac = if max > 0.0 { (v / max).clamp(0.0, 1.0) } else { 0.0 };
        let filled = (frac * levels as f64).round() as usize;
        for row in 0..h {
            let from_bottom = h - 1 - row;
            let cell = filled.saturating_sub(from_bottom * 8).min(8);
            let (ch, color) = if cell == 0 {
                (' ', theme.panel_border)
            } else {
                (LEVELS[cell], colorer(v))
            };
            buf.set_string(
                area.x + col as u16,
                area.y + row as u16,
                ch.to_string(),
                Style::default().fg(color).bg(theme.panel_bg),
            );
        }
    }
}

/// Draw a centre-line mirrored bar graph in `area`: `up` samples grow upward
/// from the horizontal mid-line, `down` samples grow downward. Newest sample is
/// at the right edge; both are scaled to the shared `max`. Used by the disk
/// (write ▲ / read ▼) and network (upload ▲ / download ▼) panels.
#[allow(clippy::too_many_arguments)]
fn draw_mirror_bars(
    f: &mut Frame,
    area: Rect,
    up: (&[f64], ratatui::style::Color),
    down: (&[f64], ratatui::style::Color),
    max: f64,
    slot: Slot,
    theme: &Theme,
    gfx: Option<&mut Gfx>,
) {
    let (up, up_color) = up;
    let (down, down_color) = down;
    let (w, h) = (area.width as usize, area.height as usize);
    if w == 0 || h == 0 {
        return;
    }

    // Graphics path: a smooth center-axis mirrored bar graph.
    if let Some(g) = gfx
        && g.available() {
            let (pw, ph) = g.px_size(area);
            let img = raster::mirror_bars(
                pw,
                ph,
                up,
                down,
                max,
                raster::rgb(up_color),
                raster::rgb(down_color),
                raster::rgb(theme.panel_border),
                raster::rgb(theme.panel_bg),
                0.5,
            );
            g.draw(f, area, slot, img);
            return;
        }

    // One row is reserved for the horizontal centre axis; the remaining rows
    // split into an upper band (grows up) and a lower band (grows down). With an
    // odd leftover the spare row goes to the lower band.
    let up_h = (h - 1) / 2;
    let down_h = h - 1 - up_h;
    let axis_y = area.y + up_h as u16;
    let up_levels = up_h * 8;
    let down_levels = down_h * 8;
    let (nu, nd) = (up.len(), down.len());
    let bg = theme.panel_bg;
    let axis_style = Style::default().fg(theme.panel_border).bg(bg);
    let frac = |v: f64, levels: usize| -> usize {
        if max > 0.0 {
            ((v / max).clamp(0.0, 1.0) * levels as f64).round() as usize
        } else {
            0
        }
    };
    let buf = f.buffer_mut();
    for col in 0..w {
        // Right-align: the rightmost column is the newest sample.
        let from_right = w - 1 - col;
        let uv = if from_right < nu { up[nu - 1 - from_right] } else { 0.0 };
        let dv = if from_right < nd { down[nd - 1 - from_right] } else { 0.0 };
        let u_filled = frac(uv, up_levels);
        let d_filled = frac(dv, down_levels);
        let x = area.x + col as u16;

        // Upper band: bottom-anchored cells (fill from the centre upward), using
        // the lower-block glyphs in the bar colour.
        for r in 0..up_h {
            let from_centre = up_h - 1 - r; // 0 = the row just above the axis
            let cell = u_filled.saturating_sub(from_centre * 8).min(8);
            let (ch, style) = if cell == 0 {
                (' ', Style::default().fg(theme.panel_border).bg(bg))
            } else {
                (LEVELS[cell], Style::default().fg(up_color).bg(bg))
            };
            buf.set_string(x, area.y + r as u16, ch.to_string(), style);
        }
        // The centre axis line.
        buf.set_string(x, axis_y, "─", axis_style);
        // Lower band: top-anchored cells (fill from the centre downward). A cell's
        // top `cell`/8 is painted in the bar colour by using it as the cell
        // background and "erasing" the unfilled lower part with a lower-block
        // glyph drawn in the panel background colour.
        for r in 0..down_h {
            let from_centre = r; // 0 = the row just below the axis
            let cell = d_filled.saturating_sub(from_centre * 8).min(8);
            let (ch, style) = if cell == 0 {
                (' ', Style::default().fg(theme.panel_border).bg(bg))
            } else {
                (LEVELS[8 - cell], Style::default().fg(bg).bg(down_color))
            };
            buf.set_string(x, axis_y + 1 + r as u16, ch.to_string(), style);
        }
    }
}

/// A centered top-border title showing battery charge: "BAT[+] 86% ▆▆▆▆░░".
fn battery_title(pct: u8, charging: bool, theme: &Theme) -> Line<'static> {
    let header = Style::default()
        .fg(theme.header_fg)
        .bg(theme.panel_bg)
        .add_modifier(Modifier::BOLD);
    // Full at 100% = green, empty = red.
    let color = load_color(100.0 - pct as f32, theme);
    const CELLS: usize = 6;
    let filled = ((pct as usize * CELLS) + 50) / 100;
    let bar: String = (0..CELLS).map(|i| if i < filled { '█' } else { '░' }).collect();
    Line::from(vec![
        Span::styled(if charging { " BAT+ " } else { " BAT " }, header),
        Span::styled(format!("{pct}% "), header),
        Span::styled(bar, Style::default().fg(color).bg(theme.panel_bg)),
        Span::styled(" ", header),
    ])
    .centered()
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

// Fixed column widths shared by both modes. The "left" region (PID+Program+
// Command in flat mode, or the whole tree column in tree mode) takes the rest.
const SPARK_W: usize = 8;
const THR_W: usize = 9;
const USER_W: usize = 12;
const MEM_W: usize = 6;
const CPU_W: usize = 6;
const PID_W: usize = 7;
const PROG_W: usize = 16;

fn render_table(f: &mut Frame, area: Rect, pv: &mut ProcView, theme: &Theme) {
    if area.height < 2 {
        return;
    }
    let width = area.width as usize;
    let tree = pv.mode == ProcMode::Tree;
    // Everything to the right of the left region: Threads, User, MemB, spark, Cpu%
    // with a single-space separator before each (5 separators total).
    let fixed = THR_W + USER_W + MEM_W + SPARK_W + CPU_W + 5;
    let left_w = width.saturating_sub(fixed).max(12);
    let cmd_w = left_w.saturating_sub(PID_W + PROG_W + 2).max(4);

    let arrow = |k: ProcSort| -> &'static str {
        if pv.sort == k {
            if pv.reverse { " ▼" } else { " ▲" }
        } else {
            ""
        }
    };

    // --- header ---
    let thr_h = format!("{}{}", crate::l10n::trd("Threads"), arrow(ProcSort::Threads));
    let user_h = format!("{}{}", crate::l10n::trd("User"), arrow(ProcSort::User));
    let mem_h = format!("MemB{}", arrow(ProcSort::Mem));
    let cpu_h = format!("Cpu%{}", arrow(ProcSort::Cpu));
    let left_h = if tree {
        pad_right(&crate::l10n::trd("Tree"), left_w)
    } else {
        format!(
            "{} {} {}",
            pad_left(&format!("[P]id{}", arrow(ProcSort::Pid)), PID_W),
            pad_right(&format!("{}{}", crate::l10n::trd("Program"), arrow(ProcSort::Name)), PROG_W),
            pad_right(&crate::l10n::trd("Command"), cmd_w),
        )
    };
    let header = format!(
        "{left_h} {} {} {} {:^sw$} {}",
        pad_left(&thr_h, THR_W),
        pad_right(&user_h, USER_W),
        pad_left(&mem_h, MEM_W),
        "cpu",
        pad_left(&cpu_h, CPU_W),
        sw = SPARK_W,
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
        let Some(row) = pv.rows.get(idx) else {
            lines.push(Line::from(""));
            continue;
        };
        let p = &pv.procs[row.proc_idx];
        let cur = idx == pv.cursor;
        let row_style = if cur { theme.cursor } else { normal };
        let row_bg = if cur {
            theme.cursor.bg.unwrap_or(theme.panel_bg)
        } else {
            theme.panel_bg
        };
        // Left region (mode-dependent) then the shared Threads/User/MemB columns
        // and the trailing separator before the sparkline.
        let left_field = if tree {
            tree_cell(&row.tree_prefix, p, left_w)
        } else {
            format!(
                "{} {} {}",
                pad_left(&p.pid.to_string(), PID_W),
                pad_right(&ellipsize(&p.name, PROG_W), PROG_W),
                pad_right(&ellipsize(&p.cmd, cmd_w), cmd_w),
            )
        };
        let pre_spark = format!(
            "{left_field} {} {} {} ",
            pad_left(&p.threads.to_string(), THR_W),
            pad_right(&ellipsize(&p.user, USER_W), USER_W),
            pad_left(&human_size(p.rss), MEM_W),
        );
        let post_spark = format!(" {}", pad_left(&format!("{:.1}", p.cpu), CPU_W));
        let mut spans = vec![Span::styled(pre_spark, row_style)];
        spans.extend(cpu_spark_spans(pv.proc_cpu_history.get(&p.pid), SPARK_W, row_bg, theme));
        spans.push(Span::styled(post_spark, row_style));
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), body);
}

/// Build the tree-mode "Tree" column for one row: the branch-glyph prefix, the
/// PID, the program name and — when it adds information — the command in
/// parentheses, truncated/padded to exactly `width` display columns.
fn tree_cell(prefix: &str, p: &crate::proc::ProcInfo, width: usize) -> String {
    // Show the executable (argv0) in parens when it differs from the short name,
    // e.g. "systemd-resolve (systemd-resolved)" — like the btop tree view.
    let arg0 = p.cmd.split_whitespace().next().unwrap_or("");
    let base = arg0.rsplit('/').next().unwrap_or(arg0);
    let paren = if !arg0.is_empty() && base != p.name && arg0 != p.name {
        format!(" ({arg0})")
    } else {
        String::new()
    };
    let text = format!("{prefix}{} {}{paren}", p.pid, p.name);
    pad_right(&ellipsize(&text, width), width)
}

/// Build a `width`-cell colored CPU sparkline for one process row (newest at the
/// right), each cell load-colored; idle cells show a faint baseline dot.
fn cpu_spark_spans(
    history: Option<&std::collections::VecDeque<f32>>,
    width: usize,
    bg: ratatui::style::Color,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut samples = vec![0.0f32; width];
    if let Some(h) = history {
        let take = h.len().min(width);
        for k in 0..take {
            samples[width - take + k] = h[h.len() - take + k];
        }
    }
    samples
        .iter()
        .map(|&s| {
            let level = ((s.clamp(0.0, 100.0) / 100.0) * 8.0).round() as usize;
            let (ch, fg) = if level == 0 {
                ('·', theme.panel_border)
            } else {
                (LEVELS[level.min(8)], load_color(s, theme))
            };
            Span::styled(ch.to_string(), Style::default().fg(fg).bg(bg))
        })
        .collect()
}

fn render_footer(f: &mut Frame, area: Rect, pv: &ProcView, theme: &Theme) {
    // Tree-specific fold keys are only shown while in tree mode.
    let hint = match pv.mode {
        ProcMode::Flat => "↑↓ move   ⇥ tree   c CPU  m Mem  t Thr  n Prog  u User  p PID   r reverse   +/- rate   k kill  K force   Esc close",
        ProcMode::Tree => "↑↓ move   ⇥ flat   →←⏎ fold  * all   c CPU  m Mem  t Thr  n Prog  u User  p PID   r rev   +/- rate   k kill  Esc close",
    };
    // Highlighted bar (matching the F-key row) so the hints are clearly visible.
    let line = pad_right(&format!(" {hint}"), area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.fkey_label))).style(theme.fkey_label),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// `tree_cell` renders "<branch><pid> <name> (<cmd>)", shows argv0 in parens
    /// only when it differs from the name, and fills exactly `width` columns.
    #[test]
    fn tree_cell_formats_branch_and_command() {
        use crate::proc::ProcInfo;
        use unicode_width::UnicodeWidthStr;
        let mut p = ProcInfo {
            pid: 1,
            ppid: None,
            name: "systemd".into(),
            cmd: "/sbin/init".into(),
            user: "root".into(),
            cpu: 0.0,
            rss: 0,
            mem_pct: 0.0,
            threads: 1,
        };
        let cell = tree_cell("[-]─", &p, 30);
        assert!(cell.starts_with("[-]─1 systemd (/sbin/init)"), "branch+pid+name+cmd: {cell:?}");
        assert_eq!(cell.width(), 30, "padded to the column width");

        // No parens when argv0's basename matches the name.
        p.name = "niri".into();
        p.cmd = "niri --session".into();
        let cell = tree_cell("└─", &p, 30);
        assert!(cell.starts_with("└─1 niri "), "no redundant command shown: {cell:?}");
        assert!(!cell.contains('('), "argv0 == name ⇒ no parens: {cell:?}");
    }

    /// `draw_mirror_bars` must grow the `up` series upward (top band, bar-colored
    /// foreground) and the `down` series downward (bottom band, bar-colored
    /// background) from the centre line, with the newest sample at the right.
    #[test]
    fn mirror_bars_split_up_and_down() {
        let theme = crate::ui::theme::Theme::mc();
        let area = Rect { x: 0, y: 0, width: 3, height: 4 };
        let (up_c, down_c) = (Color::Red, Color::Blue);
        let cx = 2u16; // rightmost column = newest sample

        // Upload-only: a full bar fills the TOP band; the bottom band stays clear.
        let mut t = Terminal::new(TestBackend::new(3, 4)).unwrap();
        t.draw(|f| draw_mirror_bars(f, area, (&[1.0], up_c), (&[], down_c), 1.0, Slot::ProcDisk, &theme, None))
            .unwrap();
        let b = t.backend().buffer();
        assert_eq!(b[(cx, 0)].fg, up_c, "top band carries the up colour");
        assert_ne!(b[(cx, 0)].symbol(), " ", "top band draws a bar glyph");
        assert_eq!(b[(cx, 1)].symbol(), "─", "the centre axis line is drawn");
        assert_ne!(b[(cx, 3)].bg, down_c, "bottom band is clear with no downloads");

        // Download-only: a full bar fills the BOTTOM band; the top band stays clear.
        let mut t = Terminal::new(TestBackend::new(3, 4)).unwrap();
        t.draw(|f| draw_mirror_bars(f, area, (&[], up_c), (&[1.0], down_c), 1.0, Slot::ProcDisk, &theme, None))
            .unwrap();
        let b = t.backend().buffer();
        assert_eq!(b[(cx, 3)].bg, down_c, "bottom band carries the down colour");
        assert_ne!(b[(cx, 0)].fg, up_c, "top band is clear with no uploads");
    }
}
