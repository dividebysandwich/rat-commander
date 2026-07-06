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
pub fn render_panel(
    f: &mut Frame,
    area: Rect,
    panel: &mut Panel,
    active: bool,
    details: &crate::details::DetailsData,
    theme: &Theme,
    brief_columns: usize,
) {
    let border_color = if active {
        theme.panel_border_active
    } else {
        theme.panel_border
    };
    // In Tree view the title tracks the directory last committed with Enter
    // (which also drives the other panel), not the fixed tree-root path.
    let title_path = match (panel.format, panel.tree.as_ref()) {
        (ViewFormat::Tree, Some(t)) => t.current.display(),
        _ => panel.cwd.display(),
    };
    let title = format!(" {} ", ellipsize(&title_path, area.width.saturating_sub(4) as usize));
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

    // Reset hit geometry; set below once the listing is drawn.
    panel.hit = None;

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

    // The Details view shows info about the *other* panel (no own listing): the
    // body fills the whole interior and there's nothing to hit-test.
    if matches!(panel.format, ViewFormat::Details) {
        crate::details::render(f, inner, details, theme);
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
        ViewFormat::Brief => render_brief(f, list_area, panel, active, theme, brief_columns),
        ViewFormat::Tree => render_tree(f, list_area, panel, active, theme),
        ViewFormat::Details => unreachable!("Details is rendered earlier and returns"),
    }

    // Record geometry for mouse hit-testing (offset is now post-render).
    let (body, brief, columns, rows, cell_w) = match panel.format {
        ViewFormat::Details => unreachable!("Details is rendered earlier and returns"),
        ViewFormat::Full => (
            Rect {
                y: list_area.y + 1,
                height: list_area.height.saturating_sub(1),
                ..list_area
            },
            false,
            1usize,
            1usize,
            list_area.width,
        ),
        ViewFormat::Brief => {
            let w = list_area.width as usize;
            let cols = brief_columns.clamp(1, w.max(1));
            let cw = (w / cols).max(1);
            (list_area, true, cols, list_area.height as usize, cw as u16)
        }
        // One tree row per body line; `panel_point` maps a click to a tree row.
        ViewFormat::Tree => (list_area, false, 1usize, 1usize, list_area.width),
    };
    // The tree scrolls independently of the flat listing, so hit-testing must use
    // the tree's own offset.
    let offset = match (panel.format, panel.tree.as_ref()) {
        (ViewFormat::Tree, Some(t)) => t.offset,
        _ => panel.offset,
    };
    panel.hit = Some(crate::panel::PanelHit {
        area,
        body,
        brief,
        offset,
        columns,
        rows,
        cell_w,
    });

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

/// Foreground color for an entry's name based on its kind/mark. Directories use
/// the same color as ordinary files (they're distinguished by the `/` prefix);
/// executables and symlinks keep their accent colors; plain files are tinted by
/// file-type category (archive / document / image / media).
fn name_style(e: &VfsEntry, marked: bool, theme: &Theme) -> Style {
    let base = Style::default().bg(theme.panel_bg);
    if marked {
        return base.fg(theme.marked_fg).add_modifier(Modifier::BOLD);
    }
    match e.kind {
        VfsKind::Symlink => base.fg(theme.symlink_fg),
        VfsKind::File if e.is_executable() => base.fg(theme.exec_fg).add_modifier(Modifier::BOLD),
        VfsKind::File => match category_color(e.extension(), theme) {
            Some(c) => base.fg(c),
            None => base.fg(theme.panel_fg),
        },
        // Directories (and anything else) use the normal foreground.
        _ => base.fg(theme.panel_fg),
    }
}

/// Map a file extension to its category accent color, if any.
fn category_color(ext: &str, theme: &Theme) -> Option<Color> {
    let e = ext.to_ascii_lowercase();
    let e = e.as_str();
    const ARCHIVE: &[&str] = &[
        "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "tbz2", "tbz", "xz", "txz", "zst", "lz",
        "lzma", "z", "deb", "rpm", "jar", "war", "apk", "cab", "arj", "lha", "lzh", "iso", "dmg",
        "pkg", "msi", "xz2",
    ];
    const DOCUMENT: &[&str] = &[
        "txt", "md", "rst", "pdf", "doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "ppt",
        "pptx", "odp", "csv", "tex", "epub", "djvu", "mobi", "log", "json", "xml", "yaml", "yml",
        "toml", "ini", "cfg", "conf", "html", "htm", "css",
    ];
    const IMAGE: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "bmp", "svg", "webp", "tiff", "tif", "ico", "ppm", "pgm",
        "xpm", "heic", "heif", "raw", "cr2", "nef", "psd", "xcf",
    ];
    const MEDIA: &[&str] = &[
        "wav", "mp3", "flac", "ogg", "oga", "opus", "aac", "m4a", "wma", "mid", "midi", "aiff",
        "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "mpg", "mpeg", "3gp", "ts", "vob",
    ];
    if ARCHIVE.contains(&e) {
        Some(theme.archive_fg)
    } else if DOCUMENT.contains(&e) {
        Some(theme.doc_fg)
    } else if IMAGE.contains(&e) {
        Some(theme.image_fg)
    } else if MEDIA.contains(&e) {
        Some(theme.media_fg)
    } else {
        None
    }
}

/// The `ls -F`-style classify character placed before each name so types are
/// distinguished by symbol (and alignment is preserved) rather than only color:
/// `/` directory, `*` executable, `@`/`!` valid/broken symlink, ` ` otherwise.
fn classify_prefix(e: &VfsEntry) -> char {
    match e.kind {
        VfsKind::Dir => '/',
        VfsKind::Symlink => {
            if e.symlink_broken {
                '!'
            } else {
                '@'
            }
        }
        VfsKind::File if e.is_executable() => '*',
        _ => ' ',
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

/// Column widths for the full-format listing — `(name_w, size_w, time_w)` — for
/// a given interior `width`. The two single-cell `│` separators account for the
/// `+ 2`. The mini-status uses the same split so its size/date columns line up
/// with the listing in every view mode.
fn full_columns(width: usize) -> (usize, usize, usize) {
    let size_w = 8usize;
    let time_w = 12usize;
    let name_w = width.saturating_sub(size_w + time_w + 2).max(4);
    (name_w, size_w, time_w)
}

/// The text shown in the size column: `DIR` / `UP--DIR` for directories, a
/// human-readable size otherwise.
fn size_field(e: &VfsEntry) -> String {
    if e.kind == VfsKind::Dir {
        if e.name == ".." { "UP--DIR" } else { "DIR" }.to_string()
    } else {
        human_size(e.size)
    }
}

fn render_full(f: &mut Frame, area: Rect, panel: &mut Panel, active: bool, theme: &Theme) {
    let width = area.width as usize;
    let (name_w, size_w, time_w) = full_columns(width);

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
    panel.page = rows.max(1);
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

        let size_str = size_field(e);
        let time_str = e.mtime.map(format_time).unwrap_or_default();

        if is_cursor && active {
            // The whole row (separators included) is highlighted; marked entries
            // keep a yellow foreground so the selection stays visible. Only the
            // active panel shows a cursor.
            let text = format!(
                "{}{COL_SEP}{}{COL_SEP}{}",
                pad_right(&display_name(e), name_w),
                pad_left(&size_str, size_w),
                pad_left(&time_str, time_w)
            );
            if theme.truecolor {
                let fg = if marked { theme.marked_fg } else { theme.cursor_fg };
                lines.push(gradient_line(&text, width, fg, theme));
            } else {
                lines.push(Line::from(Span::styled(
                    text,
                    cursor_style(true, marked, theme),
                )));
            }
        } else {
            // Marked rows are highlighted across all columns, not just the name.
            let data_style = if marked {
                Style::default()
                    .fg(theme.marked_fg)
                    .bg(theme.panel_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                normal
            };
            let spans = vec![
                Span::styled(pad_right(&display_name(e), name_w), name_style(e, marked, theme)),
                Span::styled(COL_SEP, sep_style),
                Span::styled(pad_left(&size_str, size_w), data_style),
                Span::styled(COL_SEP, sep_style),
                Span::styled(pad_left(&time_str, time_w), data_style),
            ];
            lines.push(Line::from(spans));
        }
    }
    f.render_widget(Paragraph::new(lines), body_area);
}

fn render_brief(
    f: &mut Frame,
    area: Rect,
    panel: &mut Panel,
    active: bool,
    theme: &Theme,
    brief_columns: usize,
) {
    let width = area.width as usize;
    let rows = area.height as usize;
    if rows == 0 {
        return;
    }
    // Exactly `brief_columns` columns (clamped to what the panel width allows),
    // each an equal share of the width.
    let columns = brief_columns.clamp(1, width.max(1));
    let cell_w = (width / columns).max(1);
    // A page is the full grid of visible cells (rows × columns).
    panel.page = (rows * columns).max(1);
    // Record the grid geometry for column-major arrow navigation.
    panel.cols = columns;
    panel.brief_rows = rows;
    // Each column reserves one cell for a vertical separator between names.
    let name_w = cell_w.saturating_sub(1).max(1);
    let sep_style = Style::default().fg(theme.panel_border).bg(theme.panel_bg);

    // Column-major layout: entries fill top-to-bottom, column by column, so each
    // screen column holds `rows` consecutive entries. Scroll horizontally by
    // whole columns to keep the cursor's column on screen (offset is aligned to a
    // column boundary — a multiple of `rows`).
    let cursor_col = panel.cursor / rows;
    let mut first_col = panel.offset / rows;
    if cursor_col < first_col {
        first_col = cursor_col;
    } else if cursor_col >= first_col + columns {
        first_col = cursor_col + 1 - columns;
    }
    panel.offset = first_col * rows;

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut spans: Vec<Span> = Vec::with_capacity(columns * 2);
        for c in 0..columns {
            let idx = panel.offset + c * rows + r;
            match panel.entries.get(idx) {
                Some(e) => {
                    let is_cursor = idx == panel.cursor;
                    let marked = panel.selection.is_marked(&e.name);
                    let text = pad_right(&display_name(e), name_w);
                    // Only the active panel shows a cursor highlight.
                    let style = if is_cursor && active {
                        cursor_style(true, marked, theme)
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

/// Draw the directory tree: one indented row per visible node, an expander
/// glyph (`▾`/`▸`) marking open/closed branches, the cursor row highlighted.
fn render_tree(f: &mut Frame, area: Rect, panel: &mut Panel, active: bool, theme: &Theme) {
    let width = area.width as usize;
    let rows = area.height as usize;
    if rows == 0 || width == 0 {
        return;
    }
    // A page is one screenful of rows (drives PgUp/PgDn via `move_cursor`).
    panel.page = rows.max(1);
    let Some(tree) = panel.tree.as_mut() else {
        return;
    };
    ensure_visible(tree.cursor, &mut tree.offset, rows);

    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let dir_style = Style::default().fg(theme.dir_fg).bg(theme.panel_bg);
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for i in 0..rows {
        let idx = tree.offset + i;
        let Some(node) = tree.rows.get(idx) else {
            lines.push(Line::from(Span::styled(" ".repeat(width), normal)));
            continue;
        };
        let marker = if node.expanded { '▾' } else { '▸' };
        // Two spaces of indent per depth level, then "▸ label".
        let text = format!("{}{marker} {}", "  ".repeat(node.depth), node.label);
        let text = pad_right(&text, width);
        let is_cursor = idx == tree.cursor;
        if is_cursor && active {
            if theme.truecolor {
                lines.push(gradient_line(&text, width, theme.cursor_fg, theme));
            } else {
                lines.push(Line::from(Span::styled(text, cursor_style(true, false, theme))));
            }
        } else {
            lines.push(Line::from(Span::styled(text, dir_style)));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Name as shown in the list: a one-character classify prefix (see
/// [`classify_prefix`]) followed by the entry name.
fn display_name(e: &VfsEntry) -> String {
    format!("{}{}", classify_prefix(e), e.name)
}

fn render_mini_status(f: &mut Frame, area: Rect, panel: &Panel, theme: &Theme) {
    let width = area.width as usize;
    let style = Style::default().fg(theme.panel_border_active).bg(theme.panel_bg);
    let sep_style = Style::default().fg(theme.panel_border).bg(theme.panel_bg);

    // Tree view: show the full path of the highlighted directory.
    if panel.format == ViewFormat::Tree {
        let text = panel
            .tree
            .as_ref()
            .and_then(|t| t.selected_path())
            .map(|p| p.display())
            .unwrap_or_default();
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(pad_right(&ellipsize(&text, width), width), style))),
            area,
        );
        return;
    }

    // A multi-file selection: show the count and combined size (a summary, not a
    // single entry's columns).
    if panel.selection.count() > 0 {
        let total: u64 = panel
            .entries
            .iter()
            .filter(|e| panel.selection.is_marked(&e.name))
            .map(|e| e.size)
            .sum();
        let text = format!("{} selected, {}", panel.selection.count(), human_size(total));
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(pad_right(&text, width), style))),
            area,
        );
        return;
    }

    // The current entry: name, size, and modify time laid out in the same
    // columns as the full-format listing, so the size/date line up in every view
    // mode (including Brief).
    let line = match panel.current_entry() {
        Some(e) => {
            let (name_w, size_w, time_w) = full_columns(width);
            let mut name = display_name(e);
            if let Some(t) = &e.symlink_target {
                name.push_str(&format!(" -> {t}"));
            }
            let size_str = size_field(e);
            let time_str = e.mtime.map(format_time).unwrap_or_default();
            Line::from(vec![
                Span::styled(pad_right(&name, name_w), style),
                Span::styled(COL_SEP, sep_style),
                Span::styled(pad_left(&size_str, size_w), style),
                Span::styled(COL_SEP, sep_style),
                Span::styled(pad_left(&time_str, time_w), style),
            ])
        }
        None => Line::from(Span::styled(" ".repeat(width), style)),
    };
    f.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn entry(name: &str, kind: VfsKind, mode: u32, broken: bool) -> VfsEntry {
        VfsEntry {
            name: name.to_string(),
            kind,
            size: 0,
            mtime: Some(SystemTime::UNIX_EPOCH),
            atime: None,
            ctime: None,
            inode: None,
            mode: Some(mode),
            uid: None,
            gid: None,
            symlink_target: None,
            symlink_broken: broken,
        }
    }

    #[test]
    fn mini_status_shows_size_and_date_aligned_with_columns() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut cur = entry("archive.tar.gz", VfsKind::File, 0o644, false);
        cur.size = 9_876_543;

        for fmt in [ViewFormat::Full, ViewFormat::Brief] {
            let mut panel = Panel::new(backend.clone(), crate::vfs::VfsPath::local("/tmp"));
            panel.entries = vec![entry("readme.txt", VfsKind::File, 0o644, false), cur.clone()];
            panel.format = fmt;
            panel.cursor = 1; // the archive is the current entry
            let mut term = Terminal::new(TestBackend::new(44, 8)).unwrap();
            term.draw(|t| render_panel(t, t.area(), &mut panel, true, &Default::default(), &theme, 2))
                .unwrap();
            let b = term.backend().buffer();
            let seps = |row: u16| -> Vec<u16> {
                (1..b.area.width - 1).filter(|&x| b[(x, row)].symbol() == "│").collect()
            };
            let mini_row = b.area.height - 2; // last interior row
            let mini_seps = seps(mini_row);
            // The two column separators fall at the same x as the full listing's.
            let (name_w, size_w, _) = full_columns((b.area.width - 2) as usize);
            let x0 = 1 + name_w as u16;
            let x1 = x0 + 1 + size_w as u16;
            assert_eq!(mini_seps, vec![x0, x1], "{fmt:?}: mini-status columns align with the listing");
            // Size and modify date are both shown.
            let text: String = (0..b.area.width).map(|x| b[(x, mini_row)].symbol()).collect();
            assert!(text.contains("9.4M"), "{fmt:?}: size shown in the mini-status");
            assert!(text.contains("1970"), "{fmt:?}: modify date shown in the mini-status");
        }
    }

    #[test]
    fn brief_view_records_configured_column_count() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp"));
        panel.entries =
            (0..10).map(|i| entry(&format!("f{i}"), VfsKind::File, 0o644, false)).collect();
        panel.format = ViewFormat::Brief;

        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        // Configured for 3 columns → the renderer must record 3 for grid-aware
        // arrow navigation, and a page of rows × columns.
        t.draw(|f| render_brief(f, f.area(), &mut panel, true, &theme, 3)).unwrap();
        assert_eq!(panel.cols, 3, "renderer records the configured column count");
        assert_eq!(panel.page, 8 * 3, "page = rows × columns");
    }

    #[tokio::test]
    async fn tree_view_renders_markers_and_selected_path() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("rc-tree-render-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::create_dir_all(root.join("beta")).unwrap();

        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local(&root));
        panel.format = ViewFormat::Tree;
        panel.build_tree().await;

        // Wide enough that the mini-status path isn't ellipsized.
        let mut term = Terminal::new(TestBackend::new(90, 12)).unwrap();
        term.draw(|t| render_panel(t, t.area(), &mut panel, true, &Default::default(), &theme, 2))
            .unwrap();
        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();

        // The two child directories appear under an expander marker.
        assert!(text.contains("alpha"), "tree lists the alpha directory");
        assert!(text.contains("beta"), "tree lists the beta directory");
        assert!(text.contains('▾') || text.contains('▸'), "an expander glyph is drawn");
        // The mini-status shows the highlighted directory's full path (the root).
        assert!(
            text.contains(&root.to_string_lossy().into_owned()),
            "the selected path is shown in the mini-status"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn classify_prefixes_by_type() {
        assert_eq!(classify_prefix(&entry("d", VfsKind::Dir, 0o755, false)), '/');
        assert_eq!(classify_prefix(&entry("x", VfsKind::File, 0o755, false)), '*');
        assert_eq!(classify_prefix(&entry("f", VfsKind::File, 0o644, false)), ' ');
        assert_eq!(classify_prefix(&entry("l", VfsKind::Symlink, 0o777, false)), '@');
        assert_eq!(classify_prefix(&entry("l", VfsKind::Symlink, 0o777, true)), '!');
    }

    #[test]
    fn display_name_includes_prefix() {
        assert_eq!(display_name(&entry("dir", VfsKind::Dir, 0o755, false)), "/dir");
        assert_eq!(display_name(&entry("file", VfsKind::File, 0o644, false)), " file");
    }

    #[test]
    fn category_color_maps_extensions() {
        let t = Theme::mc();
        assert_eq!(category_color("zip", &t), Some(t.archive_fg));
        assert_eq!(category_color("deb", &t), Some(t.archive_fg));
        assert_eq!(category_color("PNG", &t), Some(t.image_fg), "case-insensitive");
        assert_eq!(category_color("wav", &t), Some(t.media_fg));
        assert_eq!(category_color("mp4", &t), Some(t.media_fg));
        assert_eq!(category_color("pdf", &t), Some(t.doc_fg));
        assert_eq!(category_color("xyz", &t), None);
        assert_eq!(category_color("", &t), None);
    }

    #[test]
    fn name_style_tints_plain_files_by_type() {
        let t = Theme::mc();
        // An archive file gets the archive color; an unknown one stays normal.
        assert_eq!(
            name_style(&entry("a.zip", VfsKind::File, 0o644, false), false, &t).fg,
            Some(t.archive_fg)
        );
        assert_eq!(
            name_style(&entry("notes.dat", VfsKind::File, 0o644, false), false, &t).fg,
            Some(t.panel_fg)
        );
        // Executables and directories are unaffected by category coloring.
        assert_eq!(
            name_style(&entry("run.sh", VfsKind::File, 0o755, false), false, &t).fg,
            Some(t.exec_fg)
        );
    }
}
