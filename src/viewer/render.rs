//! Rendering for the internal viewer (text / hex).

use super::{ViewMode, ViewerState};
use crate::ui::theme::Theme;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub fn render(f: &mut Frame, area: Rect, v: &mut ViewerState, theme: &Theme) {
    if area.height < 3 {
        return;
    }
    let header = Rect { height: 1, ..area };
    let content = Rect {
        y: area.y + 1,
        height: area.height - 2,
        ..area
    };
    let footer = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };

    v.view_rows = content.height as usize;
    v.view_cols = content.width as usize;

    render_header(f, header, v, theme);
    match v.mode {
        ViewMode::Text => render_text(f, content, v, theme),
        ViewMode::Hex => render_hex(f, content, v, theme),
    }
    render_footer(f, footer, v, theme);
}

fn render_header(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    let mode = match v.mode {
        ViewMode::Text => "Text",
        ViewMode::Hex => "Hex",
    };
    let wrap = if v.wrap { "Wrap" } else { "Unwrap" };
    let trunc = if v.truncated { " [TRUNCATED]" } else { "" };
    let total = match v.mode {
        ViewMode::Text => v.line_count(),
        ViewMode::Hex => v.hex_rows(),
    };
    let text = format!(
        " View: {}  [{mode}/{wrap}]  {}/{} {}{trunc}",
        ellipsize(&v.name, area.width.saturating_sub(40) as usize),
        v.top + 1,
        total.max(1),
        if v.mode == ViewMode::Hex { "rows" } else { "lines" },
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&text, area.width as usize),
            theme.menubar.add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

fn render_text(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    let style = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let width = area.width as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    let mut line_idx = v.top;

    while lines.len() < area.height as usize && line_idx < v.line_count() {
        let raw = v.line_str(line_idx);
        if v.wrap {
            for chunk in wrap_chunks(&raw, width.max(1)) {
                if lines.len() >= area.height as usize {
                    break;
                }
                lines.push(Line::from(Span::styled(chunk, style)));
            }
            if raw.is_empty() {
                lines.push(Line::from(Span::styled(String::new(), style)));
            }
        } else {
            let shown: String = raw.chars().skip(v.h_offset).collect();
            lines.push(Line::from(Span::styled(shown, style)));
        }
        line_idx += 1;
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)),
        area,
    );
}

fn render_hex(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    let style = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    for r in 0..area.height as usize {
        let row = v.top + r;
        let off = row * 16;
        if off >= v.data.len() {
            break;
        }
        let end = (off + 16).min(v.data.len());
        let bytes = &v.data[off..end];

        let mut hex = String::with_capacity(48);
        let mut ascii = String::with_capacity(16);
        for (i, b) in bytes.iter().enumerate() {
            if i == 8 {
                hex.push(' ');
            }
            hex.push_str(&format!("{b:02x} "));
            ascii.push(if b.is_ascii_graphic() || *b == b' ' {
                *b as char
            } else {
                '.'
            });
        }
        let line = format!("{off:08x}  {hex:<49} |{ascii}|");
        lines.push(Line::from(Span::styled(line, style)));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)),
        area,
    );
}

fn render_footer(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    let line = if let Some(q) = v.search_input.as_ref() {
        Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.header_fg)),
            Span::raw(q.clone()),
        ])
    } else {
        let hint = "F2 Wrap  F4 Hex/Text  F7 Search  n Next  F10 Quit";
        let found = if !v.query.is_empty() {
            format!("  [/{}]", v.query)
        } else {
            String::new()
        };
        Line::from(Span::styled(
            format!("{hint}{found}"),
            theme.fkey_label,
        ))
    };
    f.render_widget(Paragraph::new(line), area);
    if let Some(q) = v.search_input.as_ref() {
        let cx = area.x + 8 + q.chars().count() as u16;
        f.set_cursor_position(ratatui::layout::Position::new(cx, area.y));
    }
}

/// Split a string into display-width chunks of at most `width` characters.
fn wrap_chunks(s: &str, width: usize) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(width).map(|c| c.iter().collect()).collect()
}
