//! Rendering of a single [`Panel`] into a Ratatui area.

use super::{Panel, ViewFormat};
use crate::ui::theme::Theme;
use crate::util::bytes::{format_time, human_size};
use crate::util::text::{ellipsize, pad_left, pad_right};
use crate::vfs::{VfsEntry, VfsKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// The vertical line drawn between columns.
const COL_SEP: &str = "│";
/// The horizontal rule drawn between the listing and the mini-status line.
const COL_SEP_H: &str = "─";

/// Build a horizontal-gradient line for the given text (used for the cursor
/// bar when truecolor is available).
fn gradient_line(text: &str, width: usize, fg: Color, theme: &Theme) -> Line<'static> {
    let spans: Vec<Span> = text
        .chars()
        .take(width)
        .enumerate()
        .map(|(i, ch)| {
            Span::styled(
                ch.to_string(),
                Style::default()
                    .bg(theme.gradient_at(i, width))
                    .fg(fg)
                    .add_modifier(Modifier::BOLD),
            )
        })
        .collect();
    Line::from(spans)
}

/// Draw a panel (border, header, listing, mini-status) into `area`.
pub fn render_panel(f: &mut Frame, area: Rect, panel: &mut Panel, active: bool, theme: &Theme) {
    let border_color = if active {
        theme.panel_border_active
    } else {
        theme.panel_border
    };
    let title = format!(" {} ", ellipsize(&panel.cwd.display(), area.width.saturating_sub(4) as usize));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(border_color).bg(theme.panel_bg))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.panel_border_active)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Volume capacity on the bottom border (used / total), MC-style.
    render_disk_usage(f, area, panel, border_color, theme);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if let Some(err) = &panel.error {
        let p = Paragraph::new(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(theme.error_fg).bg(theme.panel_bg),
        )));
        f.render_widget(p, inner);
        return;
    }

    // Reserve the last inner row for the mini-status (selected file name); when
    // there's room, also reserve a separator rule above it dividing the listing
    // from the mini-status, like Midnight Commander.
    let reserve: u16 = if inner.height >= 3 { 2 } else { 1 };
    let list_height = inner.height.saturating_sub(reserve);
    let list_area = Rect {
        height: list_height,
        ..inner
    };

    match panel.format {
        ViewFormat::Full => render_full(f, list_area, panel, active, theme),
        ViewFormat::Brief => render_brief(f, list_area, panel, active, theme),
    }

    let status_y = if reserve == 2 {
        let sep_y = inner.y + list_height;
        render_panel_separator(f, area, sep_y, border_color, theme);
        sep_y + 1
    } else {
        inner.y + list_height
    };
    let status_area = Rect {
        y: status_y,
        height: 1,
        ..inner
    };
    render_mini_status(f, status_area, panel, theme);
}

/// Draw a horizontal rule across the panel's interior at row `y`, joining the
/// left/right frame with `├`/`┤` — separates the listing from the mini-status.
fn render_panel_separator(f: &mut Frame, area: Rect, y: u16, border_color: Color, theme: &Theme) {
    let style = Style::default().fg(border_color).bg(theme.panel_bg);
    let inner_x = area.x + 1;
    let inner_w = area.width.saturating_sub(2) as usize;
    let buf = f.buffer_mut();
    buf.set_string(inner_x, y, COL_SEP_H.repeat(inner_w), style);
    buf.set_string(area.x, y, "├", style);
    buf.set_string(area.x + area.width - 1, y, "┤", style);
}

/// Write the volume's "used / total (NN%)" onto the bottom border, right-aligned.
fn render_disk_usage(f: &mut Frame, area: Rect, panel: &Panel, border_color: Color, theme: &Theme) {
    let Some(du) = panel.disk else {
        return;
    };
    if area.height == 0 || area.width < 24 {
        return;
    }
    let text = format!(
        " {} / {} ({}%) ",
        human_size(du.used()),
        human_size(du.total),
        du.percent_used()
    );
    let w = text.chars().count() as u16;
    // Keep a column of border on each side of the label.
    if w + 4 > area.width {
        return;
    }
    let y = area.y + area.height - 1;
    let x = area.x + area.width - 1 - w - 1;
    let style = Style::default()
        .fg(border_color)
        .bg(theme.panel_bg)
        .add_modifier(Modifier::BOLD);
    f.buffer_mut().set_string(x, y, text, style);
}

/// Foreground color for an entry's name based on its kind/mark.
fn name_style(e: &VfsEntry, marked: bool, theme: &Theme) -> Style {
    let base = Style::default().bg(theme.panel_bg);
    if marked {
        return base.fg(theme.marked_fg).add_modifier(Modifier::BOLD);
    }
    match e.kind {
        VfsKind::Dir => base.fg(theme.dir_fg).add_modifier(Modifier::BOLD),
        VfsKind::Symlink => base.fg(theme.symlink_fg),
        VfsKind::File if e.is_executable() => base.fg(theme.exec_fg).add_modifier(Modifier::BOLD),
        _ => base.fg(theme.panel_fg),
    }
}

/// Cursor row style (active vs inactive panel). When the entry under the
/// cursor is also marked, the foreground is forced to the marked color so the
/// selection remains discernible beneath the cursor highlight.
fn cursor_style(active: bool, marked: bool, theme: &Theme) -> Style {
    let base = if active {
        theme.cursor
    } else {
        theme.cursor_inactive
    };
    if marked {
        base.fg(theme.marked_fg).add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

fn ensure_visible(cursor: usize, offset: &mut usize, height: usize) {
    if height == 0 {
        return;
    }
    if cursor < *offset {
        *offset = cursor;
    } else if cursor >= *offset + height {
        *offset = cursor + 1 - height;
    }
}

fn render_full(f: &mut Frame, area: Rect, panel: &mut Panel, active: bool, theme: &Theme) {
    let width = area.width as usize;
    let size_w = 8usize;
    let time_w = 12usize;
    let name_w = width.saturating_sub(size_w + time_w + 2).max(4);

    // Header row, with vertical separators matching the data rows.
    let header_style = Style::default()
        .fg(theme.header_fg)
        .bg(theme.panel_bg)
        .add_modifier(Modifier::BOLD);
    let sep_style = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let header_line = Line::from(vec![
        Span::styled(pad_right("Name", name_w), header_style),
        Span::styled(COL_SEP, sep_style),
        Span::styled(pad_left("Size", size_w), header_style),
        Span::styled(COL_SEP, sep_style),
        Span::styled(pad_left("Modify time", time_w), header_style),
    ]);
    let header_area = Rect { height: 1, ..area };
    f.render_widget(Paragraph::new(header_line), header_area);

    let body_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    let rows = body_area.height as usize;
    ensure_visible(panel.cursor, &mut panel.offset, rows);

    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for i in 0..rows {
        let idx = panel.offset + i;
        let Some(e) = panel.entries.get(idx) else {
            // Empty row: still draw the column separators full height.
            lines.push(Line::from(vec![
                Span::styled(" ".repeat(name_w), normal),
                Span::styled(COL_SEP, sep_style),
                Span::styled(" ".repeat(size_w), normal),
                Span::styled(COL_SEP, sep_style),
                Span::styled(" ".repeat(time_w), normal),
            ]));
            continue;
        };
        let is_cursor = idx == panel.cursor;
        let marked = panel.selection.is_marked(&e.name);

        let size_str = if e.kind == VfsKind::Dir {
            if e.name == ".." {
                "UP--DIR".to_string()
            } else {
                "DIR".to_string()
            }
        } else {
            human_size(e.size)
        };
        let time_str = e.mtime.map(format_time).unwrap_or_default();

        if is_cursor {
            // The whole row (separators included) is highlighted; marked entries
            // keep a yellow foreground so the selection stays visible.
            let text = format!(
                "{}{COL_SEP}{}{COL_SEP}{}",
                pad_right(&display_name(e), name_w),
                pad_left(&size_str, size_w),
                pad_left(&time_str, time_w)
            );
            if active && theme.truecolor {
                let fg = if marked { theme.marked_fg } else { theme.cursor_fg };
                lines.push(gradient_line(&text, width, fg, theme));
            } else {
                lines.push(Line::from(Span::styled(
                    text,
                    cursor_style(active, marked, theme),
                )));
            }
        } else {
            let spans = vec![
                Span::styled(pad_right(&display_name(e), name_w), name_style(e, marked, theme)),
                Span::styled(COL_SEP, sep_style),
                Span::styled(pad_left(&size_str, size_w), normal),
                Span::styled(COL_SEP, sep_style),
                Span::styled(pad_left(&time_str, time_w), normal),
            ];
            lines.push(Line::from(spans));
        }
    }
    f.render_widget(Paragraph::new(lines), body_area);
}

fn render_brief(f: &mut Frame, area: Rect, panel: &mut Panel, active: bool, theme: &Theme) {
    let width = area.width as usize;
    let rows = area.height as usize;
    if rows == 0 {
        return;
    }
    let cell_w = 16usize.min(width.max(1));
    let columns = (width / cell_w).max(1);
    // Each column reserves one cell for a vertical separator between names.
    let name_w = cell_w.saturating_sub(1).max(1);
    let sep_style = Style::default().fg(theme.panel_border).bg(theme.panel_bg);

    // Keep the cursor's row visible (offset aligned to a row boundary).
    let cursor_row = panel.cursor / columns;
    let mut first_row = panel.offset / columns;
    if cursor_row < first_row {
        first_row = cursor_row;
    } else if cursor_row >= first_row + rows {
        first_row = cursor_row + 1 - rows;
    }
    panel.offset = first_row * columns;

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut spans: Vec<Span> = Vec::with_capacity(columns * 2);
        for c in 0..columns {
            let idx = (first_row + r) * columns + c;
            match panel.entries.get(idx) {
                Some(e) => {
                    let is_cursor = idx == panel.cursor;
                    let marked = panel.selection.is_marked(&e.name);
                    let text = pad_right(&display_name(e), name_w);
                    let style = if is_cursor {
                        cursor_style(active, marked, theme)
                    } else {
                        name_style(e, marked, theme)
                    };
                    spans.push(Span::styled(text, style));
                }
                None => spans.push(Span::styled(
                    " ".repeat(name_w),
                    Style::default().bg(theme.panel_bg),
                )),
            }
            // Separator after every column except the last.
            if c + 1 < columns {
                spans.push(Span::styled(COL_SEP, sep_style));
            }
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Name as shown in the list: directories get no slash here (mc style keeps
/// names plain and colors them), `..` shown as-is.
fn display_name(e: &VfsEntry) -> String {
    e.name.clone()
}

fn render_mini_status(f: &mut Frame, area: Rect, panel: &Panel, theme: &Theme) {
    let text = if panel.selection.count() > 0 {
        let total: u64 = panel
            .entries
            .iter()
            .filter(|e| panel.selection.is_marked(&e.name))
            .map(|e| e.size)
            .sum();
        format!("{} selected, {}", panel.selection.count(), human_size(total))
    } else if let Some(e) = panel.current_entry() {
        let kind = match e.kind {
            VfsKind::Dir => "<DIR>".to_string(),
            _ => human_size(e.size),
        };
        let link = e
            .symlink_target
            .as_ref()
            .map(|t| format!(" -> {t}"))
            .unwrap_or_default();
        format!("{}  {}{}", e.name, kind, link)
    } else {
        String::new()
    };
    let line = Line::from(Span::styled(
        pad_right(&text, area.width as usize),
        Style::default()
            .fg(theme.panel_border_active)
            .bg(theme.panel_bg),
    ));
    f.render_widget(Paragraph::new(line), area);
}
