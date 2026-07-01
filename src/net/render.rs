//! Rendering of the [`NetView`] network-connections explorer.

use super::{NetView, ProtoFilter, Socket};
use crate::ui::dialog::centered;
use crate::ui::graphics::{raster, Gfx, Slot};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::util::text::{ellipsize, pad_left, pad_right};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

/// Partial-block glyphs for cell sparklines (1/8 .. 8/8).
const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn render(f: &mut Frame, area: Rect, nv: &mut NetView, theme: &Theme, gfx: Option<&mut Gfx>) {
    let accent = theme.panel_border_active;
    let mode = if nv.root { "root — full visibility" } else { "user mode — limited visibility" };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(accent).bg(theme.panel_bg))
        .title(Span::styled(
            format!(" Network Connections — {mode} "),
            Style::default().fg(accent).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        ))
        .title(
            Line::from(Span::styled(
                format!(" refresh {}ms ", nv.interval_ms),
                Style::default().fg(accent).bg(theme.panel_bg),
            ))
            .right_aligned(),
        )
        .style(theme.panel_base());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 6 || inner.width < 12 {
        return;
    }

    if nv.scanning && nv.listening.is_empty() && nv.connections.is_empty() {
        center(f, inner, "Scanning… (running ss)", theme);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // summary
            Constraint::Percentage(40),
            Constraint::Min(3),
            Constraint::Length(1), // footer
        ])
        .split(inner);

    render_summary(f, rows[0], nv, theme);
    render_pane(f, rows[1], nv, 0, theme);
    render_pane(f, rows[2], nv, 1, theme);
    render_footer(f, rows[3], nv, theme);

    if nv.detail.is_some() {
        render_detail(f, inner, nv, theme, gfx);
    }
}

fn render_summary(f: &mut Frame, area: Rect, nv: &NetView, theme: &Theme) {
    let estab = nv.connections.iter().filter(|c| c.state == "ESTAB").count();
    let other = nv.connections.len() - estab;
    let mut toggles = String::new();
    if nv.proto_filter != ProtoFilter::All {
        toggles.push_str(&format!(" [{}]", nv.proto_filter.label()));
    }
    if nv.established_only {
        toggles.push_str(" [estab]");
    }
    if nv.hide_loopback {
        toggles.push_str(" [no-loopback]");
    }
    let text = format!(
        " {} listening · {} estab · {} other    ↓ {}/s  ↑ {}/s{}",
        nv.listening.len(),
        estab,
        other,
        human_size(nv.rate_in),
        human_size(nv.rate_out),
        toggles,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&ellipsize(&text, area.width as usize), area.width as usize),
            Style::default().fg(theme.header_fg).bg(theme.panel_bg),
        ))),
        area,
    );
}

fn render_pane(f: &mut Frame, area: Rect, nv: &mut NetView, pane: usize, theme: &Theme) {
    let focused = nv.focus.idx() == pane;
    let total = if pane == 0 { nv.listening.len() } else { nv.connections.len() };
    let name = if pane == 0 { "Listening ports" } else { "Connections" };
    let title = format!("{name}  [{}/{}]  sort:{}", nv.view[pane].len(), total, nv.sort_desc(pane));

    let border = if focused { theme.panel_border_active } else { theme.panel_border };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(border).bg(theme.panel_bg))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(border).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let ib = block.inner(area);
    f.render_widget(block, area);
    if ib.height < 2 || ib.width < 8 {
        return;
    }
    let w = ib.width as usize;

    // Build header + row strings for this pane.
    let (header, rows) = if pane == 0 {
        listening_rows(nv, w)
    } else {
        connection_rows(nv, w)
    };

    // Header row.
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&header, w),
            Style::default().fg(theme.header_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        ))),
        Rect { height: 1, ..ib },
    );
    let list = Rect { y: ib.y + 1, height: ib.height - 1, ..ib };
    let vr = list.height as usize;
    nv.view_rows[pane] = vr;

    if rows.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  (none)",
                Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
            ))),
            Rect { height: 1, ..list },
        );
        return;
    }

    let cursor = nv.cursor[pane];
    if cursor < nv.offset[pane] {
        nv.offset[pane] = cursor;
    } else if vr > 0 && cursor >= nv.offset[pane] + vr {
        nv.offset[pane] = cursor + 1 - vr;
    }
    let off = nv.offset[pane];
    for (i, text) in rows.iter().enumerate().skip(off).take(vr) {
        let y = list.y + (i - off) as u16;
        let style = if i == cursor {
            if focused { theme.cursor } else { theme.cursor_inactive }
        } else {
            Style::default().fg(theme.panel_fg).bg(theme.panel_bg)
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(pad_right(&ellipsize(text, w), w), style))),
            Rect { x: ib.x, y, width: ib.width, height: 1 },
        );
    }
}

/// Column layout for the listening pane: proto | program | service | local.
fn listening_rows(nv: &NetView, w: usize) -> (String, Vec<String>) {
    let (proto_w, prog_w, svc_w) = (6usize, 22usize, 12usize);
    let local_w = w.saturating_sub(proto_w + prog_w + svc_w + 3);
    let header = format!(
        "{} {} {} {}",
        cell("Proto", proto_w, false),
        cell("Program", prog_w, false),
        cell("Service", svc_w, false),
        cell("Local address", local_w, false),
    );
    let rows = nv.view[0]
        .iter()
        .map(|&i| {
            let s = &nv.listening[i];
            format!(
                "{} {} {} {}",
                cell(&s.proto, proto_w, false),
                cell(&program_label(s), prog_w, false),
                cell(&s.service, svc_w, false),
                cell(&s.local, local_w, false),
            )
        })
        .collect();
    (header, rows)
}

/// Column layout for the connections pane: proto | state | local | peer(+svc) |
/// program | cumulative In/Out | live In/s / Out/s | a per-connection rate
/// sparkline (its own recent throughput history).
fn connection_rows(nv: &NetView, w: usize) -> (String, Vec<String>) {
    let (proto_w, state_w, prog_w, io_w, spark_w) = (6usize, 8usize, 12usize, 7usize, 12usize);
    let fixed = proto_w + state_w + prog_w + io_w * 4 + spark_w + 9; // + single-space gaps
    let flex = w.saturating_sub(fixed);
    let local_w = flex / 2;
    let peer_w = flex - local_w;
    let header = format!(
        "{} {} {} {} {} {} {} {} {} {}",
        cell("Proto", proto_w, false),
        cell("State", state_w, false),
        cell("Local", local_w, false),
        cell("Peer", peer_w, false),
        cell("Program", prog_w, false),
        cell("In", io_w, true),
        cell("Out", io_w, true),
        cell("In/s", io_w, true),
        cell("Out/s", io_w, true),
        cell("Rate trend", spark_w, false),
    );
    let rows = nv.view[1]
        .iter()
        .map(|&i| {
            let s = &nv.connections[i];
            let peer = if s.service.is_empty() {
                s.peer.clone()
            } else {
                format!("{} ({})", s.peer, s.service)
            };
            // The connection's own recent combined-rate trend (self-scaled).
            let trend = nv
                .rate_history
                .get(&super::socket_key(s))
                .map(|h| {
                    let v: Vec<u64> = h.iter().map(|&(i, o)| i + o).collect();
                    spark_cells(&v, spark_w)
                })
                .unwrap_or_else(|| " ".repeat(spark_w));
            format!(
                "{} {} {} {} {} {} {} {} {} {}",
                cell(&s.proto, proto_w, false),
                cell(&s.state, state_w, false),
                cell(&s.local, local_w, false),
                cell(&peer, peer_w, false),
                cell(&program_label(s), prog_w, false),
                cell(&bytes(s.rx), io_w, true),
                cell(&bytes(s.tx), io_w, true),
                cell(&rate(s.rx_rate), io_w, true),
                cell(&rate(s.tx_rate), io_w, true),
                trend,
            )
        })
        .collect();
    (header, rows)
}

fn render_footer(f: &mut Frame, area: Rect, nv: &NetView, theme: &Theme) {
    if nv.filtering {
        // Show the filter input with a caret marker.
        let mut shown: String = nv.filter.chars().collect();
        let caret = nv.filter_cursor.min(shown.chars().count());
        let bytepos = shown.char_indices().nth(caret).map(|(b, _)| b).unwrap_or(shown.len());
        shown.insert(bytepos, '▏');
        let line = pad_right(&format!(" Filter: {shown}"), area.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                line,
                Style::default().fg(theme.dialog_fg).bg(theme.input_bg),
            ))),
            area,
        );
        return;
    }
    if let Some(err) = &nv.error {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&format!(" ⚠ {err}"), area.width as usize),
                Style::default().fg(theme.error_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    let hint = " / filter  s/S sort  p proto  e estab  h loopback  k/K kill  ⏎ details  Tab pane  r refresh  Esc close";
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(pad_right(hint, area.width as usize), theme.fkey_label)))
            .style(theme.fkey_label),
        area,
    );
}

/// The details popup for the selected socket.
fn render_detail(f: &mut Frame, area: Rect, nv: &NetView, theme: &Theme, gfx: Option<&mut Gfx>) {
    let Some(d) = &nv.detail else {
        return;
    };
    let w = 78u16.min(area.width.saturating_sub(4));
    let h = 18u16.min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.panel_border_active).bg(theme.dialog_bg))
        .title(Span::styled(
            " Connection details ",
            Style::default().fg(theme.dialog_title).bg(theme.dialog_bg).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
    let ib = block.inner(rect);
    f.render_widget(block, rect);
    if ib.height < 8 || ib.width < 20 {
        return;
    }

    let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
    let dim = Style::default().fg(theme.panel_fg).bg(theme.dialog_bg);
    let s = &d.sock;
    // Prefer the live socket (fresh rates) if it's still open, else the snapshot.
    let live = nv
        .connections
        .iter()
        .chain(nv.listening.iter())
        .find(|c| super::socket_key(c) == d.key)
        .unwrap_or(s);

    let mut y = ib.y;
    let put = |f: &mut Frame, y: &mut u16, label: &str, val: &str, style: Style| {
        if *y < ib.y + ib.height {
            let text = format!("{label:<10}{val}");
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(ellipsize(&text, ib.width as usize), style)))
                    .style(base),
                Rect { x: ib.x, y: *y, width: ib.width, height: 1 },
            );
            *y += 1;
        }
    };
    put(f, &mut y, "Type", &format!("{}   {}", s.proto, s.state), base);
    put(f, &mut y, "Local", &s.local, base);
    put(f, &mut y, "Peer", &format!("{}{}", s.peer, svc_suffix(&s.service)), base);
    put(f, &mut y, "Program", &program_label(s), base);
    if !d.info.user.is_empty() {
        put(f, &mut y, "User", &d.info.user, base);
    }
    if !d.info.cmdline.is_empty() {
        put(f, &mut y, "Command", &d.info.cmdline, base);
    }
    put(
        f,
        &mut y,
        "Traffic",
        &format!(
            "in {}  out {}   (live in {}  out {})",
            bytes(live.rx),
            bytes(live.tx),
            rate(live.rx_rate),
            rate(live.tx_rate),
        ),
        base,
    );

    // Rate graph (out grows up, in grows down) from the connection's history.
    if let Some(hist) = nv.rate_history.get(&d.key)
        && y + 4 <= ib.y + ib.height
    {
        put(f, &mut y, "Rate", "out ▲ / in ▼", dim);
        let graph = Rect { x: ib.x, y, width: ib.width, height: 4 };
        draw_rate_graph(f, graph, hist, theme, gfx);
        y += 4;
    }

    // Raw ss info (rtt/cwnd/retransmits…), wrapped.
    if !s.info.is_empty() && y + 1 < ib.y + ib.height {
        let avail = (ib.y + ib.height - y) as usize;
        f.render_widget(
            Paragraph::new(s.info.clone()).wrap(Wrap { trim: true }).style(dim),
            Rect { x: ib.x, y, width: ib.width, height: avail as u16 },
        );
    }
}

/// A mirrored rate graph (out up, in down) — graphics when available, else two
/// cell sparklines.
fn draw_rate_graph(
    f: &mut Frame,
    area: Rect,
    hist: &std::collections::VecDeque<(u64, u64)>,
    theme: &Theme,
    gfx: Option<&mut Gfx>,
) {
    let ins: Vec<f64> = hist.iter().map(|&(i, _)| i as f64).collect();
    let outs: Vec<f64> = hist.iter().map(|&(_, o)| o as f64).collect();
    let max = ins.iter().chain(outs.iter()).copied().fold(1.0f64, f64::max);

    if let Some(g) = gfx
        && g.available()
        && area.width > 0
        && area.height > 0
    {
        let (pw, ph) = g.px_size(area);
        let img = raster::mirror_bars(
            pw,
            ph,
            &outs,
            &ins,
            max,
            raster::rgb(theme.panel_border_active),
            raster::rgb(theme.exec_fg),
            raster::rgb(theme.panel_border),
            raster::rgb(theme.dialog_bg),
            0.5,
        );
        g.draw(f, area, Slot::NetRate, img);
        return;
    }

    // Cell fallback: out sparkline on the top half, in on the bottom half.
    let out_u: Vec<u64> = hist.iter().map(|&(_, o)| o).collect();
    let in_u: Vec<u64> = hist.iter().map(|&(i, _)| i).collect();
    let w = area.width as usize;
    let mid = area.height / 2;
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            spark_cells(&out_u, w),
            Style::default().fg(theme.panel_border_active).bg(theme.dialog_bg),
        ))),
        Rect { x: area.x, y: area.y, width: area.width, height: 1 },
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            spark_cells(&in_u, w),
            Style::default().fg(theme.exec_fg).bg(theme.dialog_bg),
        ))),
        Rect { x: area.x, y: area.y + mid.max(1), width: area.width, height: 1 },
    );
}

// --- small formatting helpers ------------------------------------------------

fn cell(s: &str, width: usize, right: bool) -> String {
    if width == 0 {
        return String::new();
    }
    let e = ellipsize(s, width);
    if right { pad_left(&e, width) } else { pad_right(&e, width) }
}

fn program_label(s: &Socket) -> String {
    if s.program.is_empty() {
        "-".to_string()
    } else if let Some(pid) = s.pid {
        format!("{} ({pid})", s.program)
    } else {
        s.program.clone()
    }
}

fn bytes(v: Option<u64>) -> String {
    v.map(human_size).unwrap_or_else(|| "-".to_string())
}

fn rate(v: Option<u64>) -> String {
    match v {
        Some(0) | None => "-".to_string(),
        Some(b) => format!("{}/s", human_size(b)),
    }
}

fn svc_suffix(service: &str) -> String {
    if service.is_empty() { String::new() } else { format!(" ({service})") }
}

/// A right-aligned cell sparkline of the last `width` values.
fn spark_cells(vals: &[u64], width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let max = vals.iter().copied().max().unwrap_or(0).max(1);
    let start = vals.len().saturating_sub(width);
    let slice = &vals[start..];
    let mut s = String::with_capacity(width);
    for _ in 0..width.saturating_sub(slice.len()) {
        s.push(' ');
    }
    for &v in slice {
        let lvl = ((v as f64 / max as f64) * 7.0).round() as usize;
        s.push(SPARK[lvl.min(7)]);
    }
    s
}

fn center(f: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    let y = area.y + area.height / 2;
    f.render_widget(
        Paragraph::new(Line::from(text)).alignment(Alignment::Center).style(theme.panel_base()),
        Rect { y, height: 1, ..area },
    );
}
