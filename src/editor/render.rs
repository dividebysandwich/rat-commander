//! Rendering for the internal editor.

use super::EditorState;
use crate::ui::theme::Theme;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub fn render(f: &mut Frame, area: Rect, ed: &mut EditorState, theme: &Theme) {
    if area.height < 3 {
        return;
    }
    let status = Rect { height: 1, ..area };
    let text_area = Rect {
        y: area.y + 1,
        height: area.height - 2,
        ..area
    };
    let footer = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };

    ed.view_rows = text_area.height as usize;
    ed.view_cols = text_area.width as usize;
    ensure_visible(ed);

    render_status(f, status, ed, theme);
    let cursor_pos = render_text(f, text_area, ed, theme);
    render_footer(f, footer, ed, theme);

    if let Some(p) = cursor_pos {
        f.set_cursor_position(p);
    }
}

fn ensure_visible(ed: &mut EditorState) {
    let line = ed.buf.char_to_line(ed.cursor);
    let col = ed.cursor - ed.buf.line_to_char(line);
    if line < ed.top_line {
        ed.top_line = line;
    } else if ed.view_rows > 0 && line >= ed.top_line + ed.view_rows {
        ed.top_line = line + 1 - ed.view_rows;
    }
    if col < ed.left_col {
        ed.left_col = col;
    } else if ed.view_cols > 0 && col >= ed.left_col + ed.view_cols {
        ed.left_col = col + 1 - ed.view_cols;
    }
}

fn render_status(f: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) {
    let line = ed.buf.char_to_line(ed.cursor);
    let col = ed.cursor - ed.buf.line_to_char(line);
    let total = ed.buf.len_lines();
    let code = match ed.buf.char_at(ed.cursor) {
        Some('\n') => "C= 10 0x0A".to_string(),
        Some(c) => format!("C={:>3} 0x{:02X}", c as u32, c as u32),
        None => "C= <EOF>".to_string(),
    };
    let dirty = if ed.dirty { "[+]" } else { "   " };
    let name = ellipsize(&ed.name, area.width.saturating_sub(48) as usize);
    let text = format!(
        " {name} {dirty}  Ln {}/{}  Col {}  {code}  Ofs {} ",
        line + 1,
        total,
        col + 1,
        ed.cursor,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&text, area.width as usize),
            theme.menubar.add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

/// Render the text body. Returns the hardware cursor position, if on screen.
fn render_text(f: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) -> Option<Position> {
    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let block_style = Style::default()
        .fg(ratatui::style::Color::Black)
        .bg(ratatui::style::Color::Cyan);
    let block = ed.block_range();
    let cols = area.width as usize;

    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    for row in 0..area.height as usize {
        let li = ed.top_line + row;
        if li >= ed.buf.len_lines() {
            lines.push(Line::from(Span::styled(" ".repeat(cols), normal)));
            continue;
        }
        let line_start = ed.buf.line_to_char(li);
        let chars: Vec<char> = ed.buf.line_text(li).chars().collect();

        // Build styled runs across the visible columns.
        let mut spans: Vec<Span> = Vec::new();
        let mut run = String::new();
        let mut run_hl = false;
        for vc in 0..cols {
            let ci = ed.left_col + vc;
            let (ch, hl) = if ci < chars.len() {
                let abs = line_start + ci;
                let highlighted = block.map(|(s, e)| abs >= s && abs < e).unwrap_or(false);
                (chars[ci], highlighted)
            } else {
                (' ', false)
            };
            if hl != run_hl && !run.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut run),
                    if run_hl { block_style } else { normal },
                ));
            }
            run_hl = hl;
            run.push(ch);
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, if run_hl { block_style } else { normal }));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)),
        area,
    );

    // Cursor position (only when not prompting; caller decides).
    let cline = ed.buf.char_to_line(ed.cursor);
    let ccol = ed.cursor - ed.buf.line_to_char(cline);
    if cline >= ed.top_line && ccol >= ed.left_col {
        let y = area.y + (cline - ed.top_line) as u16;
        let x = area.x + (ccol - ed.left_col) as u16;
        if y < area.y + area.height && x < area.x + area.width {
            return Some(Position::new(x, y));
        }
    }
    None
}

fn render_footer(f: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) {
    let width = area.width as usize;
    if ed.status.is_empty() {
        // Same full-width, number+label styling as the main program.
        crate::ui::fkeys::render(f, area, &crate::ui::fkeys::EDITOR_LABELS, theme);
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&ed.status, width),
                theme.fkey_label,
            ))),
            area,
        );
    }
}
