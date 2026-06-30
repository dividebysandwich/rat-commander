//! Rendering for the internal viewer (text / hex).

use super::{ViewMode, ViewerState};
use crate::ui::theme::Theme;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
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
    v.content_area = content;
    v.footer_area = footer;

    // Make sure the lines about to be drawn (plus one past, so the last line's
    // extent is known) are indexed — the rest of the file stays unscanned.
    if v.mode == ViewMode::Text {
        v.extend_to_line(v.top + v.view_rows);
    }

    render_header(f, header, v, theme);
    match v.mode {
        ViewMode::Text => render_text(f, content, v, theme),
        ViewMode::Hex => render_hex(f, content, v, theme),
    }
    render_footer(f, footer, v, theme);
}

/// Build a styled line from `chars`, coloring each by `fg[base + j]` (falling
/// back to `default`), merging adjacent same-color runs.
fn build_spans(chars: &[char], base: usize, fg: &[Color], default: Color, bg: Color) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let mut run = String::new();
    let mut cur = default;
    for (j, &ch) in chars.iter().enumerate() {
        let color = fg.get(base + j).copied().unwrap_or(default);
        if color != cur && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), Style::default().fg(cur).bg(bg)));
        }
        cur = color;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, Style::default().fg(cur).bg(bg)));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), Style::default().fg(default).bg(bg)));
    }
    Line::from(spans)
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
    // While the line index is still being built, the total is a lower bound, so
    // flag it with a trailing '+'.
    let more = if v.mode == ViewMode::Text && !v.fully_indexed() { "+" } else { "" };
    let text = format!(
        " View: {}  [{mode}/{wrap}]  {}/{}{more} {}{trunc}",
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

fn render_text(f: &mut Frame, area: Rect, v: &mut ViewerState, theme: &Theme) {
    let default = theme.text_fg;
    let bg = theme.panel_bg;
    let width = area.width as usize;
    let rows = area.height as usize;
    let highlighted = v.has_syntax();
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    let mut line_idx = v.top;

    while lines.len() < rows && line_idx < v.line_count() {
        let raw = v.line_str(line_idx);
        let chars: Vec<char> = raw.chars().collect();
        // Per-character foreground colors (empty ⇒ everything uses `default`).
        let mut fg: Vec<Color> = if highlighted {
            let runs = v.line_runs(line_idx);
            let mut out = Vec::with_capacity(chars.len());
            for (n, color) in runs {
                for _ in 0..n {
                    if out.len() >= chars.len() {
                        break;
                    }
                    out.push(color);
                }
            }
            out
        } else {
            Vec::new()
        };

        // Tint the `#` of any hex-color token with its own color, regardless of
        // syntax highlighting.
        let hashes = crate::ui::hexcolor::hex_color_hashes(&chars);
        if !hashes.is_empty() {
            if fg.len() < chars.len() {
                fg.resize(chars.len(), default);
            }
            for (i, color) in hashes {
                fg[i] = color;
            }
        }

        if v.wrap {
            if chars.is_empty() {
                lines.push(build_spans(&[], 0, &fg, default, bg));
            } else {
                let mut start = 0;
                while start < chars.len() && lines.len() < rows {
                    let end = (start + width.max(1)).min(chars.len());
                    lines.push(build_spans(&chars[start..end], start, &fg, default, bg));
                    start = end;
                }
            }
        } else {
            let from = v.h_offset.min(chars.len());
            lines.push(build_spans(&chars[from..], from, &fg, default, bg));
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
    let total_rows = v.hex_rows();
    for r in 0..area.height as usize {
        let row = v.top + r;
        if row >= total_rows {
            break;
        }
        let off = row * 16;
        let bytes = v.hex_row(off);

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
    if let Some(q) = v.search_input.as_ref() {
        let line = Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.header_fg)),
            Span::raw(q.clone()),
        ]);
        f.render_widget(Paragraph::new(line), area);
        let cx = area.x + 8 + q.chars().count() as u16;
        f.set_cursor_position(ratatui::layout::Position::new(cx, area.y));
        return;
    }

    // Same full-width, number+label styling as the main program.
    let labels = v.footer_labels();
    crate::ui::fkeys::render(f, area, &labels, theme);
}
