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

    if ed.is_hex() {
        let cursor_pos = render_hex(f, text_area, ed, theme);
        render_hex_status(f, status, ed, theme);
        render_hex_footer(f, footer, ed, theme);
        if let Some(p) = cursor_pos {
            f.set_cursor_position(p);
        }
        return;
    }

    ensure_visible(ed);
    // Highlight up to the bottom of the visible window before drawing.
    let bottom = ed.top_line + ed.view_rows + 1;
    ed.ensure_hl(bottom);
    render_status(f, status, ed, theme);
    let cursor_pos = render_text(f, text_area, ed, theme);
    render_footer(f, footer, ed, theme);

    if let Some(p) = cursor_pos {
        f.set_cursor_position(p);
    }
}

/// Render the hex view (offset | bytes | ascii). Returns the hardware cursor
/// position on the active nibble/char, if visible.
fn render_hex(f: &mut Frame, area: Rect, ed: &mut EditorState, theme: &Theme) -> Option<Position> {
    let rows = area.height as usize;
    let bpr = super::hex::BYTES_PER_ROW;
    let h = ed.hex.as_mut().unwrap();
    h.view_rows = rows;

    // Scroll so the cursor's row is visible.
    let cur_row = h.cursor / bpr;
    let top_row = h.top / bpr;
    let new_top_row = if cur_row < top_row {
        cur_row
    } else if rows > 0 && cur_row >= top_row + rows as u64 {
        cur_row + 1 - rows as u64
    } else {
        top_row
    };
    h.top = new_top_row * bpr;
    let window = h.window(h.top, rows * bpr as usize);

    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let offset_style = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    let sep = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let active = theme.cursor; // highlighted cell in the focused pane
    let inactive = Style::default()
        .fg(theme.panel_bg)
        .bg(theme.panel_border)
        .add_modifier(Modifier::BOLD);

    let cell = |off: u64| -> Option<u8> {
        if off < h.len {
            Some(window[(off - h.top) as usize])
        } else {
            None
        }
    };

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for r in 0..rows {
        let base = h.top + r as u64 * bpr;
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled(format!("{base:08X}"), offset_style));
        spans.push(Span::styled("  ", sep));
        for j in 0..bpr {
            let off = base + j;
            let txt = match cell(off) {
                Some(b) => format!("{b:02X}"),
                None => "  ".to_string(),
            };
            let st = if off == h.cursor {
                if h.ascii_pane { inactive } else { active }
            } else {
                normal
            };
            spans.push(Span::styled(txt, st));
            spans.push(Span::styled(if j == 7 { "  " } else { " " }, sep));
        }
        spans.push(Span::styled("|", sep));
        for j in 0..bpr {
            let off = base + j;
            let (ch, present) = match cell(off) {
                Some(b) if (0x20..0x7f).contains(&b) => (b as char, true),
                Some(_) => ('.', true),
                None => (' ', false),
            };
            let st = if present && off == h.cursor {
                if h.ascii_pane { active } else { inactive }
            } else {
                normal
            };
            spans.push(Span::styled(ch.to_string(), st));
        }
        spans.push(Span::styled("|", sep));
        lines.push(Line::from(spans));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.panel_fg).bg(theme.panel_bg)),
        area,
    );

    // Hardware cursor on the active nibble / ascii cell.
    if cur_row >= new_top_row {
        let rrow = (cur_row - new_top_row) as u16;
        let j = (h.cursor % bpr) as u16;
        let x = if h.ascii_pane {
            60 + j
        } else {
            10 + 3 * j + u16::from(j >= 8) + u16::from(h.nibble_low)
        };
        if (rrow as usize) < rows && x < area.width {
            return Some(Position::new(area.x + x, area.y + rrow));
        }
    }
    None
}

fn render_hex_status(f: &mut Frame, area: Rect, ed: &mut EditorState, theme: &Theme) {
    let name = ellipsize(&ed.name, area.width.saturating_sub(56).max(4) as usize);
    let h = ed.hex.as_mut().unwrap();
    let cur = h.cursor;
    let byte = match h.byte_at(cur) {
        Some(b) => format!("0x{b:02X} {b:>3}"),
        None => "--".to_string(),
    };
    let pane = if h.ascii_pane { "ASCII" } else { "HEX" };
    let flags = format!(
        "{}{}",
        if h.dirty { "[+]" } else { "   " },
        if h.readonly { " [RO]" } else { "" }
    );
    let text = format!(
        " HEX {flags} {name}  Off 0x{cur:08X}/{:X}  Byte {byte}  pane:{pane} ",
        h.len
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&text, area.width as usize),
            theme.menubar.add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

fn render_hex_footer(f: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) {
    if ed.status.is_empty() {
        // Same F-key bar as text mode, showing only the supported functions.
        crate::ui::fkeys::render(f, area, &crate::ui::fkeys::HEX_LABELS, theme);
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&ed.status, area.width as usize),
                theme.fkey_label,
            ))),
            area,
        );
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
        // Syntax foreground per character (None ⇒ all `panel_fg`).
        let fg = ed.line_fg(li, chars.len(), theme.panel_fg);

        // Build styled runs across the visible columns, breaking on style change.
        let mut spans: Vec<Span> = Vec::new();
        let mut run = String::new();
        let mut run_style = normal;
        for vc in 0..cols {
            let ci = ed.left_col + vc;
            let (ch, style) = if ci < chars.len() {
                let abs = line_start + ci;
                if block.map(|(s, e)| abs >= s && abs < e).unwrap_or(false) {
                    (chars[ci], block_style)
                } else {
                    let color = fg.as_ref().map(|v| v[ci]).unwrap_or(theme.panel_fg);
                    (chars[ci], Style::default().fg(color).bg(theme.panel_bg))
                }
            } else {
                (' ', normal)
            };
            if style != run_style && !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), run_style));
            }
            run_style = style;
            run.push(ch);
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, run_style));
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
