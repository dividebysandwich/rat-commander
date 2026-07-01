//! Rendering of the [`NetView`] network-connections explorer.

use super::{NetView, Pane, Socket};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::util::text::{ellipsize, pad_left, pad_right};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

pub fn render(f: &mut Frame, area: Rect, nv: &mut NetView, theme: &Theme) {
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
    if inner.height < 5 || inner.width < 10 {
        return;
    }

    // First scan still running with nothing to show yet.
    if nv.scanning && nv.listening.is_empty() && nv.connections.is_empty() {
        center(f, inner, "Scanning… (running ss)", theme);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    render_listening(f, rows[0], nv, theme);
    render_connections(f, rows[1], nv, theme);
    render_footer(f, rows[2], nv, theme);
}

fn render_listening(f: &mut Frame, area: Rect, nv: &mut NetView, theme: &Theme) {
    let focused = nv.focus == Pane::Listening;
    let title = format!("Listening ports ({})", nv.listening.len());
    let ib = pane_block(f, area, &title, focused, theme);
    if ib.height < 2 {
        return;
    }
    let w = ib.width as usize;
    // Columns: proto | program | local address.
    let proto_w = 6usize;
    let prog_w = (w / 3).clamp(8, 30);
    let local_w = w.saturating_sub(proto_w + prog_w + 2);
    let header = format!(
        "{} {} {}",
        cell("Proto", proto_w, false),
        cell("Program", prog_w, false),
        cell("Local address", local_w, false),
    );
    let rows: Vec<String> = nv
        .listening
        .iter()
        .map(|s| {
            format!(
                "{} {} {}",
                cell(&s.proto, proto_w, false),
                cell(&program_label(s), prog_w, false),
                cell(&s.local, local_w, false),
            )
        })
        .collect();
    draw_rows(f, ib, &header, &rows, nv.cursor[0], &mut nv.offset[0], &mut nv.view_rows[0], focused, theme);
}

fn render_connections(f: &mut Frame, area: Rect, nv: &mut NetView, theme: &Theme) {
    let focused = nv.focus == Pane::Connections;
    let title = format!("Connections ({})", nv.connections.len());
    let ib = pane_block(f, area, &title, focused, theme);
    if ib.height < 2 {
        return;
    }
    let w = ib.width as usize;
    // Columns: proto | state | local | peer | program | In | Out.
    let (proto_w, state_w, prog_w, io_w) = (6usize, 9usize, 16usize, 9usize);
    let fixed = proto_w + state_w + prog_w + io_w * 2 + 6; // + 6 single-space gaps
    let flex = w.saturating_sub(fixed);
    let local_w = flex / 2;
    let peer_w = flex - local_w;
    let header = format!(
        "{} {} {} {} {} {} {}",
        cell("Proto", proto_w, false),
        cell("State", state_w, false),
        cell("Local", local_w, false),
        cell("Peer", peer_w, false),
        cell("Program", prog_w, false),
        cell("In", io_w, true),
        cell("Out", io_w, true),
    );
    let rows: Vec<String> = nv
        .connections
        .iter()
        .map(|s| {
            format!(
                "{} {} {} {} {} {} {}",
                cell(&s.proto, proto_w, false),
                cell(&s.state, state_w, false),
                cell(&s.local, local_w, false),
                cell(&s.peer, peer_w, false),
                cell(&program_label(s), prog_w, false),
                cell(&traffic(s.rx), io_w, true),
                cell(&traffic(s.tx), io_w, true),
            )
        })
        .collect();
    draw_rows(f, ib, &header, &rows, nv.cursor[1], &mut nv.offset[1], &mut nv.view_rows[1], focused, theme);
}

/// Draw a pane's outer block and return its interior rect.
fn pane_block(f: &mut Frame, area: Rect, title: &str, focused: bool, theme: &Theme) -> Rect {
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
    ib
}

/// Render a pane's header row and its scrollable data rows into interior `ib`.
#[allow(clippy::too_many_arguments)]
fn draw_rows(
    f: &mut Frame,
    ib: Rect,
    header: &str,
    rows: &[String],
    cursor: usize,
    offset: &mut usize,
    view_rows: &mut usize,
    focused: bool,
    theme: &Theme,
) {
    let w = ib.width as usize;
    // The pane block is drawn by the caller path via `pane_block`; render its
    // interior here (header + rows).
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(header, w),
            Style::default().fg(theme.header_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
        ))),
        Rect { height: 1, ..ib },
    );
    let list = Rect { y: ib.y + 1, height: ib.height - 1, ..ib };
    let vr = list.height as usize;
    *view_rows = vr;
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
    // Keep the cursor visible.
    if cursor < *offset {
        *offset = cursor;
    } else if vr > 0 && cursor >= *offset + vr {
        *offset = cursor + 1 - vr;
    }
    let off = *offset;
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

fn render_footer(f: &mut Frame, area: Rect, nv: &NetView, theme: &Theme) {
    if let Some(err) = &nv.error {
        let line = pad_right(&format!(" ⚠ {err}"), area.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                line,
                Style::default().fg(theme.error_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    let hint = " Tab switch pane   ↑↓ scroll   r refresh   +/- interval   Esc close";
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(pad_right(hint, area.width as usize), theme.fkey_label)))
            .style(theme.fkey_label),
        area,
    );
}

/// Fixed-width, left- or right-aligned cell (ellipsized to fit).
fn cell(s: &str, width: usize, right: bool) -> String {
    if width == 0 {
        return String::new();
    }
    let e = ellipsize(s, width);
    if right { pad_left(&e, width) } else { pad_right(&e, width) }
}

/// `name (pid)`, or `-` when the owning program isn't visible.
fn program_label(s: &Socket) -> String {
    if s.program.is_empty() {
        "-".to_string()
    } else if let Some(pid) = s.pid {
        format!("{} ({pid})", s.program)
    } else {
        s.program.clone()
    }
}

/// Human byte count, or `-` when the kernel didn't report it.
fn traffic(bytes: Option<u64>) -> String {
    match bytes {
        Some(b) => human_size(b),
        None => "-".to_string(),
    }
}

fn center(f: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    let y = area.y + area.height / 2;
    f.render_widget(
        Paragraph::new(Line::from(text))
            .alignment(Alignment::Center)
            .style(theme.panel_base()),
        Rect { y, height: 1, ..area },
    );
}
