//! Rendering of the [`NetView`] network-connections explorer.

use super::{Dir, NetView, Pane, Proto3, ProtoFilter, ServiceCard, Socket};
use crate::ui::dialog::centered;
use crate::ui::graphics::{raster, Gfx, Slot};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::util::text::{ellipsize, pad_left, pad_right};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

/// Partial-block glyphs for cell sparklines (1/8 .. 8/8).
const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn render(f: &mut Frame, area: Rect, nv: &mut NetView, theme: &Theme, gfx: Option<&mut Gfx>) {
    let accent = theme.panel_border_active;
    let mode = if nv.root {
        crate::l10n::trd("root — full visibility")
    } else {
        crate::l10n::trd("user mode — limited visibility")
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(accent).bg(theme.panel_bg))
        .title(Span::styled(
            format!(" {} — {mode} ", crate::l10n::trd("Network Connections")),
            Style::default().fg(accent).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        ))
        .title(
            Line::from(Span::styled(
                format!(" {} {}ms ", crate::l10n::trd("refresh"), nv.interval_ms),
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
        center(f, inner, &crate::l10n::trd("Scanning… (running ss)"), theme);
        return;
    }

    let mut gfx = gfx;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // summary
            Constraint::Min(3),    // body (two panes, or the overview diagram)
            Constraint::Length(1), // footer
        ])
        .split(inner);

    render_summary(f, rows[0], nv, theme);
    if nv.focus == Pane::Overview {
        // The IP-details popup is drawn as cells on top of the diagram. A
        // terminal-graphics diagram packs each row's kitty placeholders into the
        // grid's left column, so when the diagram data changes it repaints the
        // whole row width — over the popup, whose (unchanged) cells the frame diff
        // won't re-emit. While the popup is open, fall back to the ASCII card grid
        // (plain cells) so the popup always composites correctly on top.
        let og = if nv.ip_detail.is_some() { None } else { gfx.as_deref_mut() };
        render_overview(f, rows[1], nv, theme, og);
    } else {
        nv.overview_nodes.clear();
        let panes = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(40), Constraint::Min(3)])
            .split(rows[1]);
        render_pane(f, panes[0], nv, 0, theme);
        render_pane(f, panes[1], nv, 1, theme);
    }
    render_footer(f, rows[2], nv, theme);

    if nv.detail.is_some() {
        render_detail(f, inner, nv, theme, gfx);
    }
    if nv.ip_detail.is_some() {
        render_ip_detail(f, inner, nv, theme);
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
        " {} {} · {} estab · {} {}    ↓ {}/s  ↑ {}/s{}",
        nv.listening.len(),
        crate::l10n::trd("listening"),
        estab,
        other,
        crate::l10n::trd("other"),
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
    let name = if pane == 0 {
        crate::l10n::trd("Listening ports")
    } else {
        crate::l10n::trd("Connections")
    };
    let title = format!(
        "{name}  [{}/{}]  {}:{}",
        nv.view[pane].len(),
        total,
        crate::l10n::trd("sort"),
        nv.sort_desc(pane)
    );

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
                format!("  {}", crate::l10n::trd("(none)")),
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
        cell(&crate::l10n::trd("Proto"), proto_w, false),
        cell(&crate::l10n::trd("Program"), prog_w, false),
        cell(&crate::l10n::trd("Service"), svc_w, false),
        cell(&crate::l10n::trd("Local address"), local_w, false),
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
        cell(&crate::l10n::trd("Proto"), proto_w, false),
        cell(&crate::l10n::trd("State"), state_w, false),
        cell(&crate::l10n::trd("Local"), local_w, false),
        cell(&crate::l10n::trd("Peer"), peer_w, false),
        cell(&crate::l10n::trd("Program"), prog_w, false),
        // Traffic-direction column headers stay literal: "In"/"Out" are terse
        // networking abbreviations, and the bare "In" key already denotes the
        // unrelated location-field label elsewhere (localizing it here would
        // mistranslate one of the two).
        cell("In", io_w, true),
        cell("Out", io_w, true),
        cell("In/s", io_w, true),
        cell("Out/s", io_w, true),
        cell(&crate::l10n::trd("Rate trend"), spark_w, false),
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
        let line = pad_right(&format!(" {}: {shown}", crate::l10n::trd("Filter")), area.width as usize);
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
    let hint = if nv.focus == Pane::Overview {
        "arrows move  ⏎ IP details  k/K kill  / filter  p proto  e estab  h loopback  Tab view  r refresh  Esc close"
    } else {
        "/ filter  s/S sort  p proto  e estab  h loopback  k/K kill  ⏎ details  Tab view  r refresh  Esc close"
    };
    let line = pad_right(&format!(" {}", crate::l10n::trd(hint)), area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.fkey_label)))
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
            format!(" {} ", crate::l10n::trd("Connection details")),
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
    put(f, &mut y, &crate::l10n::trd("Type"), &format!("{}   {}", s.proto, s.state), base);
    put(f, &mut y, &crate::l10n::trd("Local"), &s.local, base);
    put(f, &mut y, &crate::l10n::trd("Peer"), &format!("{}{}", s.peer, svc_suffix(&s.service)), base);
    put(f, &mut y, &crate::l10n::trd("Program"), &program_label(s), base);
    if !d.info.user.is_empty() {
        put(f, &mut y, &crate::l10n::trd("User"), &d.info.user, base);
    }
    if !d.info.cmdline.is_empty() {
        put(f, &mut y, &crate::l10n::trd("Command"), &d.info.cmdline, base);
    }
    put(
        f,
        &mut y,
        &crate::l10n::trd("Traffic"),
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
        put(f, &mut y, &crate::l10n::trd("Rate"), "out ▲ / in ▼", dim);
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

// ---------------------------------------------------------------------------
// Overview diagram (service-card grid)
// ---------------------------------------------------------------------------

/// Max IP rows shown per card before an overflow line.
const MAX_IPS: usize = 8;
/// Card width in cells.
const CARD_W: u16 = 34;

/// Color for a protocol mix (TCP=cyan, UDP=green, both=yellow).
fn proto_color(theme: &Theme, p: Proto3) -> Color {
    match p {
        Proto3::Tcp => theme.symlink_fg,
        Proto3::Udp => theme.exec_fg,
        Proto3::Both => theme.marked_fg,
    }
}

fn dir_glyph(dir: Dir) -> &'static str {
    match dir {
        Dir::In => "◀",
        Dir::Out => "▶",
    }
}

fn render_overview(f: &mut Frame, area: Rect, nv: &mut NetView, theme: &Theme, gfx: Option<&mut Gfx>) {
    nv.overview_cards = nv.build_cards();

    // Legend line.
    let leg = |t: &str, c: Color| Span::styled(t.to_string(), Style::default().fg(c).bg(theme.panel_bg));
    let base = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let legend = Line::from(vec![
        Span::styled(" ", base),
        leg("TCP", theme.symlink_fg),
        Span::styled("  ", base),
        leg("UDP", theme.exec_fg),
        Span::styled("  ", base),
        leg("TCP+UDP", theme.marked_fg),
        Span::styled(
            format!("     ◀ {}   ▶ {}", crate::l10n::trd("inbound"), crate::l10n::trd("outbound")),
            base,
        ),
    ]);
    f.render_widget(Paragraph::new(legend).style(theme.panel_base()), Rect { height: 1, ..area });

    let grid = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    nv.overview_grid = grid;
    if nv.overview_cards.is_empty() {
        nv.overview_nodes.clear();
        center(f, grid, &crate::l10n::trd("(no active connections)"), theme);
        return;
    }

    let (boxes, nodes, total_h) = layout_cards(grid, &nv.overview_cards);
    nv.overview_nodes = nodes;
    if nv.overview_cursor >= nv.overview_nodes.len() {
        nv.overview_cursor = nv.overview_nodes.len().saturating_sub(1);
    }
    // Keep the selected node visible.
    let visible = grid.height as usize;
    if let Some((_, _, r)) = nv.overview_nodes.get(nv.overview_cursor) {
        let sy = r.y as usize;
        if sy < nv.overview_scroll {
            nv.overview_scroll = sy;
        } else if sy >= nv.overview_scroll + visible {
            nv.overview_scroll = sy + 1 - visible;
        }
    }
    nv.overview_scroll = nv.overview_scroll.min(total_h.saturating_sub(visible));
    let scroll = nv.overview_scroll;

    if let Some(g) = gfx.filter(|g| g.available()) {
        draw_overview_graphics(f, grid, nv, &boxes, scroll, theme, g);
    } else {
        draw_overview_ascii(f, grid, nv, &boxes, scroll, theme);
    }
}

/// Flow cards into a responsive grid; returns card boxes and IP-node rects in
/// *virtual* coordinates (absolute x, y = row from the grid top) plus total rows.
#[allow(clippy::type_complexity)]
fn layout_cards(
    grid: Rect,
    cards: &[ServiceCard],
) -> (Vec<(usize, Rect)>, Vec<(usize, usize, Rect)>, usize) {
    let card_w = CARD_W.min(grid.width.max(1));
    let gap = 1u16;
    let cols = ((grid.width + gap) / (card_w + gap)).max(1) as usize;
    let mut boxes = Vec::new();
    let mut nodes = Vec::new();
    let (mut vy, mut col, mut row_h) = (0u16, 0usize, 0u16);
    for (ci, card) in cards.iter().enumerate() {
        let shown = card.ips.len().min(MAX_IPS);
        let overflow = card.ips.len() > MAX_IPS;
        let ch = 2 + shown as u16 + overflow as u16; // top+bottom border + ip rows + overflow
        if col >= cols {
            col = 0;
            vy += row_h + 1;
            row_h = 0;
        }
        let x = grid.x + col as u16 * (card_w + gap);
        boxes.push((ci, Rect { x, y: vy, width: card_w, height: ch }));
        for r in 0..shown {
            nodes.push((ci, r, Rect { x: x + 1, y: vy + 1 + r as u16, width: card_w - 2, height: 1 }));
        }
        row_h = row_h.max(ch);
        col += 1;
    }
    (boxes, nodes, (vy + row_h) as usize)
}

fn selected_node(nv: &NetView) -> Option<(usize, usize)> {
    nv.overview_nodes.get(nv.overview_cursor).map(|(c, r, _)| (*c, *r))
}

/// Reverse-DNS suffix for an IP row, once resolved.
fn dns_suffix(nv: &NetView, ip: &str) -> String {
    match nv.dns.get(ip) {
        Some(Some(h)) => format!("  {h}"),
        _ => String::new(),
    }
}

fn draw_overview_ascii(f: &mut Frame, grid: Rect, nv: &NetView, boxes: &[(usize, Rect)], scroll: usize, theme: &Theme) {
    let sel = selected_node(nv);
    let top = grid.y as i32;
    let bottom = (grid.y + grid.height) as i32;
    for (ci, vr) in boxes {
        let card = &nv.overview_cards[*ci];
        let color = proto_color(theme, card.proto);
        let border = Style::default().fg(color).bg(theme.panel_bg);
        let inner_w = vr.width.saturating_sub(2) as usize;
        let shown = card.ips.len().min(MAX_IPS);
        let overflow = card.ips.len() > MAX_IPS;
        for rr in 0..vr.height {
            let sy_i = top + vr.y as i32 - scroll as i32 + rr as i32;
            if sy_i < top || sy_i >= bottom {
                continue;
            }
            let sy = sy_i as u16;
            let rowrect = Rect { x: vr.x, y: sy, width: vr.width, height: 1 };
            let line = if rr == 0 {
                // Top border with the service title.
                let title = format!(" {} :{} {} ", card.proto.label(), card.port, card.name);
                let title = ellipsize(&title, inner_w);
                let dashes = inner_w.saturating_sub(title.chars().count());
                Line::from(Span::styled(format!("┌{title}{}┐", "─".repeat(dashes)), border.add_modifier(Modifier::BOLD)))
            } else if rr as usize == vr.height as usize - 1 {
                Line::from(Span::styled(format!("└{}┘", "─".repeat(inner_w)), border))
            } else if overflow && rr as usize == 1 + shown {
                let extra = card.ips.len() - shown;
                let txt = pad_right(&format!(" … +{extra} {}", crate::l10n::trd("more")), inner_w);
                Line::from(vec![
                    Span::styled("│", border),
                    Span::styled(txt, Style::default().fg(theme.panel_fg).bg(theme.panel_bg)),
                    Span::styled("│", border),
                ])
            } else {
                let r = rr as usize - 1;
                let ip = &card.ips[r];
                let content = format!("{} {}{}", dir_glyph(ip.dir), ip.ip, dns_suffix(nv, &ip.ip));
                let content = pad_right(&ellipsize(&content, inner_w), inner_w);
                let is_sel = sel == Some((*ci, r));
                let style = if is_sel {
                    theme.dialog_selection
                } else {
                    Style::default().fg(theme.panel_fg).bg(theme.panel_bg)
                };
                Line::from(vec![
                    Span::styled("│", border),
                    Span::styled(content, style),
                    Span::styled("│", border),
                ])
            };
            f.render_widget(Paragraph::new(line), rowrect);
        }
    }
}

fn draw_overview_graphics(
    f: &mut Frame,
    grid: Rect,
    nv: &NetView,
    boxes: &[(usize, Rect)],
    scroll: usize,
    theme: &Theme,
    g: &mut Gfx,
) {
    use std::hash::{Hash, Hasher};
    let (cw, ch) = g.cell();
    let total_rows = boxes.iter().map(|(_, r)| r.y + r.height).max().unwrap_or(0) as u32;
    let img_w = (grid.width as u32 * cw).max(1);
    let full_h = (total_rows * ch).max(1);
    let win_h = (grid.height as u32 * ch).max(1);
    let sel = selected_node(nv);

    // Signature so the image is only rebuilt when something visible changes.
    let mut hh = std::collections::hash_map::DefaultHasher::new();
    (grid.width, grid.height, scroll as u64).hash(&mut hh);
    (sel.map(|(a, b)| (a as u64, b as u64))).hash(&mut hh);
    for (ci, _) in boxes {
        let card = &nv.overview_cards[*ci];
        (card.dir as u8, card.port, card.name.as_str(), card.proto.label()).hash(&mut hh);
        for ip in card.ips.iter().take(MAX_IPS) {
            (ip.ip.as_str(), matches!(ip.dir, Dir::In), nv.dns.get(&ip.ip).cloned().flatten()).hash(&mut hh);
        }
    }
    let sig = hh.finish();

    let bg = raster::rgb(theme.panel_bg);
    g.draw_cached(f, grid, Slot::NetDiagram, sig, || {
        let mut full = raster::canvas(img_w, full_h, bg);
        for (ci, vr) in boxes {
            let card = &nv.overview_cards[*ci];
            let color = raster::rgb(proto_color(theme, card.proto));
            let ox = (vr.x - grid.x) as u32 * cw;
            let oy = vr.y as u32 * ch;
            let bw = vr.width as u32 * cw;
            let bh = vr.height as u32 * ch;
            raster::pillow_into(&mut full, ox, oy, bw, bh, raster::over(bg, color, 0.18), &[], Some(color));
            let title = format!("{} :{} {}", card.proto.label(), card.port, card.name);
            let tpx = (ch as f32 * 0.62).clamp(11.0, 20.0);
            raster::draw_text(&mut full, ox as i32 + 6, oy as i32 + 2, &title, color, None, tpx);
            let shown = card.ips.len().min(MAX_IPS);
            let px = (ch as f32 * 0.58).clamp(10.0, 18.0);
            for r in 0..shown {
                let ip = &card.ips[r];
                let ny = oy + (1 + r as u32) * ch;
                let is_sel = sel == Some((*ci, r));
                if is_sel {
                    let selc = theme.dialog_selection.bg.unwrap_or(theme.panel_border_active);
                    raster::fill_rect(&mut full, ox + 2, ny, bw.saturating_sub(4), ch, raster::rgb(selc));
                }
                let glyph = if matches!(ip.dir, Dir::In) { "<" } else { ">" };
                let text = format!("{glyph} {}{}", ip.ip, dns_suffix(nv, &ip.ip));
                let fg = if is_sel {
                    raster::rgb(theme.dialog_selection.fg.unwrap_or(theme.panel_bg))
                } else {
                    raster::rgb(theme.panel_fg)
                };
                raster::draw_text(&mut full, ox as i32 + 8, ny as i32 + 2, &text, fg, None, px);
            }
        }
        // Crop to the visible window (whole-image scroll).
        let y0 = (scroll as u32 * ch).min(full_h.saturating_sub(1));
        let hcrop = win_h.min(full_h - y0).max(1);
        let cropped = image::imageops::crop_imm(&full, 0, y0, img_w, hcrop).to_image();
        let mut out = raster::canvas(img_w, win_h, bg);
        image::imageops::overlay(&mut out, &cropped, 0, 0);
        out
    });
}

/// The IP-details popup opened from the overview (with reverse-DNS).
fn render_ip_detail(f: &mut Frame, area: Rect, nv: &NetView, theme: &Theme) {
    let Some(d) = &nv.ip_detail else {
        return;
    };
    let w = 66u16.min(area.width.saturating_sub(4));
    let h = 12u16.min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.panel_border_active).bg(theme.dialog_bg))
        .title(Span::styled(
            format!(" {} ", crate::l10n::trd("IP details")),
            Style::default().fg(theme.dialog_title).bg(theme.dialog_bg).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
    let ib = block.inner(rect);
    f.render_widget(block, rect);
    if ib.height < 6 {
        return;
    }
    let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
    let host = match nv.dns.get(&d.ip) {
        Some(Some(h)) => h.clone(),
        Some(None) => crate::l10n::trd("(no PTR record)"),
        None => crate::l10n::trd("resolving…"),
    };
    let dir = if matches!(d.dir, Dir::In) {
        crate::l10n::trd("inbound (remote → us)")
    } else {
        crate::l10n::trd("outbound (us → remote)")
    };
    let progs = if d.programs.is_empty() { "-".to_string() } else { d.programs.join(", ") };
    let rows = [
        format!("{:<10}{}", "IP", d.ip),
        format!("{:<10}{host}", crate::l10n::trd("Host")),
        format!("{:<10}:{} {} ({})", crate::l10n::trd("Service"), d.port, d.service, d.proto.label()),
        format!("{:<10}{dir}", crate::l10n::trd("Direction")),
        format!("{:<10}{progs}", crate::l10n::trd("Program")),
        format!("{:<10}{}", crate::l10n::trd("Sockets"), d.count),
        format!(
            "{:<10}in {}  out {}   ({}/s)",
            crate::l10n::trd("Traffic"),
            human_size(d.rx),
            human_size(d.tx),
            human_size(d.rate)
        ),
    ];
    for (i, line) in rows.iter().enumerate() {
        let y = ib.y + i as u16;
        if y >= ib.y + ib.height {
            break;
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(ellipsize(line, ib.width as usize), base))).style(base),
            Rect { x: ib.x, y, width: ib.width, height: 1 },
        );
    }
    let by = ib.y + ib.height - 1;
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(crate::l10n::trd("Esc / Enter to close"), base)))
            .alignment(Alignment::Center)
            .style(base),
        Rect { x: ib.x, y: by, width: ib.width, height: 1 },
    );
}
