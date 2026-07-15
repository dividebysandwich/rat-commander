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
    ed.text_area = text_area;
    ed.footer_area = footer;

    if ed.is_hex() {
        let cursor_pos = render_hex(f, text_area, ed, theme);
        render_hex_status(f, status, ed, theme);
        render_hex_footer(f, footer, ed, theme);
        if let Some(p) = cursor_pos {
            f.set_cursor_position(p);
        }
        return;
    }

    // A just-restored cursor (see `EditorState::restore_position`) is scrolled to
    // the vertical center of the view, once, before the usual visibility clamp.
    if ed.pending_center {
        ed.pending_center = false;
        let line = ed.buf.char_to_line(ed.cursor);
        ed.top_line = line.saturating_sub(ed.view_rows / 2);
        ed.top_sub = 0;
    }
    if ed.wrap() {
        ensure_visible_wrapped(ed);
    } else {
        ensure_visible(ed);
    }
    // Highlight up to the bottom of the visible window before drawing.
    let bottom = ed.top_line + ed.view_rows + 1;
    ed.ensure_hl(bottom);
    render_status(f, status, ed, theme);
    let cursor_pos = if ed.wrap() {
        render_text_wrapped(f, text_area, ed, theme)
    } else {
        render_text(f, text_area, ed, theme)
    };
    render_footer(f, footer, ed, theme);

    // The F1 help overlay sits above the text and hides the hardware cursor.
    if ed.help_open() {
        render_help(f, area, theme);
        return;
    }
    if let Some(p) = cursor_pos {
        f.set_cursor_position(p);
    }
}

/// A centered modal listing the editor's keyboard shortcuts (F1).
fn render_help(f: &mut Frame, area: Rect, theme: &Theme) {
    use ratatui::widgets::{Block, BorderType, Borders, Clear};
    let help = super::EDITOR_HELP;
    // Width = widest "keys  description" line (+ padding); height = rows + border.
    let key_w = help.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    let inner_w = help
        .iter()
        .map(|(_k, d)| key_w + 2 + d.chars().count())
        .max()
        .unwrap_or(20);
    let w = (inner_w as u16 + 4).min(area.width.saturating_sub(2));
    let h = (help.len() as u16 + 2).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect { x, y, width: w, height: h };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.dialog_border_fg).bg(theme.dialog_border_bg))
        .title(Span::styled(
            " Editor shortcuts ",
            Style::default().fg(theme.dialog_title).bg(theme.dialog_border_bg).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let key_style = Style::default().fg(theme.header_fg).bg(theme.dialog_bg).add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
    let mut lines: Vec<Line> = Vec::with_capacity(help.len());
    for (k, d) in help {
        let pad = " ".repeat(key_w + 2 - k.chars().count());
        lines.push(Line::from(vec![
            Span::styled(format!(" {k}"), key_style),
            Span::styled(pad, desc_style),
            Span::styled((*d).to_string(), desc_style),
        ]));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)), inner);
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

    let normal = Style::default().fg(theme.text_fg).bg(theme.panel_bg);
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
        Paragraph::new(lines).style(Style::default().fg(theme.text_fg).bg(theme.panel_bg)),
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
        let labels = ed.footer_labels();
        crate::ui::fkeys::render(f, area, &labels, theme);
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
    let wrap = if ed.wrap() { " WRAP" } else { "" };
    let name = ellipsize(&ed.name, area.width.saturating_sub(54) as usize);
    let text = format!(
        " {name} {dirty}{wrap}  Ln {}/{}  Col {}  {code}  Ofs {} ",
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
/// Background for a line matched by the search dialog's "Find all": the theme's
/// inactive-cursor bar, i.e. the shade the panels already use for "highlighted,
/// but not where you are". Reusing it means every theme — including a user's own
/// — gets a sensible colour without a new field to define.
fn found_line_bg(theme: &Theme) -> ratatui::style::Color {
    theme.cursor_inactive.bg.unwrap_or(theme.panel_bg)
}

fn render_text(f: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) -> Option<Position> {
    let normal = Style::default().fg(theme.text_fg).bg(theme.panel_bg);
    let found_bg = found_line_bg(theme);
    // The selected block follows the theme's selection bar (like the panel
    // cursor) rather than a hardcoded colour.
    let block_style = theme.cursor;
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
        // A "Find all" hit tints the whole line, using the theme's inactive-cursor
        // bar — the same "highlighted, but not where you are" shade the panels use.
        let line_bg = if ed.line_found(li) { found_bg } else { theme.panel_bg };
        let chars: Vec<char> = ed.buf.line_text(li).chars().collect();
        // Syntax foreground per character (None ⇒ all `text_fg`).
        let mut fg = ed.line_fg(li, chars.len(), theme.text_fg);
        // Tint the `#` of any hex-color token with its own color, regardless of
        // syntax highlighting.
        let hashes = crate::ui::hexcolor::hex_color_hashes(&chars);
        if !hashes.is_empty() {
            let v = fg.get_or_insert_with(|| vec![theme.text_fg; chars.len()]);
            if v.len() < chars.len() {
                v.resize(chars.len(), theme.text_fg);
            }
            for (i, color) in hashes {
                v[i] = color;
            }
        }

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
                    let color = fg.as_ref().map(|v| v[ci]).unwrap_or(theme.text_fg);
                    (chars[ci], Style::default().fg(color).bg(line_bg))
                }
            } else {
                // Pad to the full width in the line's own colour, so a highlighted
                // line reads as one bar rather than stopping at its last character.
                (' ', Style::default().fg(theme.text_fg).bg(line_bg))
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

/// Adjust the wrap scroll position (`top_line` / `top_sub`) so the cursor's
/// visual row is within the window. Walks at most `view_rows` visual rows.
fn ensure_visible_wrapped(ed: &mut EditorState) {
    ed.left_col = 0;
    let total = ed.buf.len_lines();
    if ed.top_line >= total {
        ed.top_line = total.saturating_sub(1);
        ed.top_sub = 0;
    }
    let tsubs = ed.line_breaks(ed.top_line).len();
    if ed.top_sub >= tsubs {
        ed.top_sub = tsubs - 1;
    }
    let (cl, cs, _) = ed.cursor_visual();
    // Above the window → scroll up to the cursor's row.
    if (cl, cs) < (ed.top_line, ed.top_sub) {
        ed.top_line = cl;
        ed.top_sub = cs;
        return;
    }
    // Count visual rows from the top down to the cursor (capped).
    let mut steps = 0usize;
    let mut pos = (ed.top_line, ed.top_sub);
    while pos != (cl, cs) {
        match ed.vis_next(pos.0, pos.1) {
            Some(p) => {
                pos = p;
                steps += 1;
                if steps >= ed.view_rows {
                    break;
                }
            }
            None => break,
        }
    }
    // At/below the bottom → put the cursor's row on the last visible line.
    if steps >= ed.view_rows {
        let mut top = (cl, cs);
        for _ in 0..ed.view_rows.saturating_sub(1) {
            match ed.vis_prev(top.0, top.1) {
                Some(p) => top = p,
                None => break,
            }
        }
        ed.top_line = top.0;
        ed.top_sub = top.1;
    }
}

/// Render the body with virtual word wrap: each logical line spans one or more
/// screen rows, and every *continued* row ends in a `>` marker. Returns the
/// hardware cursor position, if on screen.
fn render_text_wrapped(f: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) -> Option<Position> {
    let normal = Style::default().fg(theme.text_fg).bg(theme.panel_bg);
    let found_bg = found_line_bg(theme);
    // The selected block follows the theme's selection bar (like the panel
    // cursor) rather than a hardcoded colour.
    let block_style = theme.cursor;
    let marker_style = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
    let block = ed.block_range();
    let cols = area.width as usize;
    let total_lines = ed.buf.len_lines();
    let (cl, cs, cvcol) = ed.cursor_visual();

    let mut lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    let mut cursor_screen = None;
    let mut pos = (ed.top_line, ed.top_sub);
    let mut past_end = ed.top_line >= total_lines;
    for row in 0..area.height as usize {
        if past_end {
            lines.push(Line::from(Span::styled(" ".repeat(cols), normal)));
            continue;
        }
        let (line, sub) = pos;
        let breaks = ed.line_breaks(line);
        let start = breaks[sub];
        let end = breaks.get(sub + 1).copied().unwrap_or(ed.buf.line_len(line));
        let has_marker = sub + 1 < breaks.len();
        let line_start = ed.buf.line_to_char(line);
        // A "Find all" hit tints every visual row of the wrapped line.
        let line_bg = if ed.line_found(line) { found_bg } else { theme.panel_bg };
        let chars: Vec<char> = ed.buf.line_text(line).chars().collect();
        let mut fg = ed.line_fg(line, chars.len(), theme.text_fg);
        let hashes = crate::ui::hexcolor::hex_color_hashes(&chars);
        if !hashes.is_empty() {
            let v = fg.get_or_insert_with(|| vec![theme.text_fg; chars.len()]);
            if v.len() < chars.len() {
                v.resize(chars.len(), theme.text_fg);
            }
            for (i, color) in hashes {
                v[i] = color;
            }
        }

        let mut spans: Vec<Span> = Vec::new();
        let mut run = String::new();
        let mut run_style = normal;
        for vc in 0..cols {
            // Reserve the final column for the `>` continuation marker.
            if has_marker && vc + 1 == cols {
                if !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), run_style));
                }
                spans.push(Span::styled(">".to_string(), marker_style));
                run_style = normal;
                break;
            }
            let ci = start + vc;
            let (ch, style) = if ci < end {
                let abs = line_start + ci;
                if block.map(|(s, e)| abs >= s && abs < e).unwrap_or(false) {
                    (chars[ci], block_style)
                } else {
                    let color = fg.as_ref().map(|v| v[ci]).unwrap_or(theme.text_fg);
                    (chars[ci], Style::default().fg(color).bg(line_bg))
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

        if (line, sub) == (cl, cs) {
            let x = area.x + cvcol.min(cols.saturating_sub(1)) as u16;
            cursor_screen = Some(Position::new(x, area.y + row as u16));
        }

        match ed.vis_next(line, sub) {
            Some(p) => pos = p,
            None => past_end = true,
        }
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)),
        area,
    );
    cursor_screen
}

fn render_footer(f: &mut Frame, area: Rect, ed: &EditorState, theme: &Theme) {
    let width = area.width as usize;
    if ed.status.is_empty() {
        // Same full-width, number+label styling as the main program. Labels
        // reflect the held modifiers (Save→Save as, Hex→Wrap with Shift/Ctrl).
        let labels = ed.footer_labels();
        crate::ui::fkeys::render(f, area, &labels, theme);
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
