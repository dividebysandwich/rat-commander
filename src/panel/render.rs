//! Rendering of a single [`Panel`] into a Ratatui area.

use super::{Panel, ViewFormat};
use crate::ui::theme::{GradRole, Theme};
use crate::util::bytes::{format_time, human_size};
use crate::util::text::{ellipsize, pad_left, pad_right};
use crate::vfs::{VfsEntry, VfsKind, VfsPath};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

/// The vertical line drawn between columns.
const COL_SEP: &str = "│";
/// The horizontal rule drawn between the listing and the mini-status line.
const COL_SEP_H: &str = "─";

/// Build a gradient line for the given text (used for the cursor bar when
/// truecolor is available): the cursor's own gradient if the theme gives it one,
/// else the theme's accent ramp.
fn gradient_line(text: &str, width: usize, fg: Color, theme: &Theme) -> Line<'static> {
    let spans: Vec<Span> = text
        .chars()
        .take(width)
        .enumerate()
        .map(|(i, ch)| {
            let bg = theme
                .bar_bg(GradRole::CursorBg, i, width)
                .unwrap_or_else(|| theme.cursor.bg.unwrap_or(theme.panel_bg));
            Span::styled(
                ch.to_string(),
                Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD),
            )
        })
        .collect();
    Line::from(spans)
}

/// Draw a panel (border, header, listing, mini-status) into `area`.
#[allow(clippy::too_many_arguments)]
pub fn render_panel(
    f: &mut Frame,
    area: Rect,
    panel: &mut Panel,
    active: bool,
    details: &crate::details::DetailsData,
    theme: &Theme,
    brief_columns: usize,
    quick_search: Option<&str>,
    graphics: bool,
    nerd: bool,
) {
    let border_color = if active { theme.panel_border_active } else { theme.panel_border };
    // The Details and 3D views have no listing of their own (they describe the
    // *other* panel), so their titles are fixed labels rather than a path — the
    // panel's own directory is not what is on screen, and showing it just names
    // wherever the panel happened to be when the view was switched on.
    // In Tree view the title tracks the directory last committed with Enter (not
    // the fixed tree-root path).
    let mut title_path = match (panel.format, panel.tree.as_ref()) {
        (ViewFormat::Details, _) => {
            crate::l10n::tr("&Details view").chars().filter(|&c| c != '&').collect()
        }
        // The log is of the other panel's tree, so that is the path it names.
        (ViewFormat::Activity, _) => {
            let label: String =
                crate::l10n::tr("&Activity log").chars().filter(|&c| c != '&').collect();
            match panel.activity.as_ref().and_then(|a| a.root.as_ref()) {
                Some(root) => format!("{label}: {}", root.display()),
                None => label,
            }
        }
        // Under the time machine the title names the commit the scene is of,
        // which is the one thing a scrub changes and the track has no room for.
        (ViewFormat::Space3d, _) => match panel.scrub.as_ref() {
            Some(row) => row.label.clone(),
            None => crate::l10n::trd("3D Directory View"),
        },
        (ViewFormat::Tree, Some(t)) => t.current.display(),
        _ => panel.cwd.display(),
    };
    // Surface an active listing filter in the title so hidden entries are obvious
    // (not in the views that have no filterable listing of their own).
    if !matches!(panel.format, ViewFormat::Details | ViewFormat::Space3d)
        && let Some(filter) = &panel.filter
    {
        title_path = format!("{title_path}  [{filter}]");
    }
    // Reserve a cell at the left of the title for the ◀ arrow, and a cell at the
    // right border for the ▶ arrow, drawn over the border below (only when the
    // panel is wide enough for them).
    let arrows = area.width >= 12;
    let (lpad, reserve) = if arrows { ("  ", 6usize) } else { (" ", 4usize) };
    let title =
        format!("{lpad}{} ", ellipsize(&title_path, (area.width as usize).saturating_sub(reserve)));
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
    // Clear the whole panel area first so the panel is fully opaque: the console
    // backdrop is painted behind the panels, and `Block` only recolours cells
    // (it doesn't overwrite their symbols), so without this the backdrop text
    // would bleed through cells the listing doesn't fill.
    f.render_widget(Clear, area);
    f.render_widget(block, area);

    // Volume capacity on the bottom border (used / total), MC-style.
    render_disk_usage(f, area, panel, border_color, theme);

    // Clickable back/forward arrows on the top border (over the reserved pad).
    render_history_arrows(f, area, panel, arrows, theme);

    // Git branch + ahead/behind on the bottom-left border (when in a repo).
    render_git_branch(f, area, panel, theme);

    // Reset hit geometry; set below once the listing is drawn.
    panel.hit = None;
    // The quick-search caret (set below when a search is rendering).
    panel.quick_caret = None;
    // Cleared unless a Details image preview reserves an area below.
    panel.preview_image_area = None;
    // The same for a Details audio preview's controls: set again only if they
    // are drawn this frame.
    if let Some(av) = &details.audio {
        av.clear_hits();
    }
    // Refilled by the thumbnail grid, when it is the format.
    panel.thumb_cells.clear();

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
    // body fills the whole interior and there's nothing to hit-test. A pixel-image
    // preview reports its target rect, drawn by the root layer (which owns `Gfx`).
    if matches!(panel.format, ViewFormat::Details) {
        panel.preview_image_area = crate::details::render(f, inner, details, theme, graphics);
        return;
    }

    // Reserve the last inner row for the mini-status (selected file name); when
    // there's room, also reserve a separator rule above it dividing the listing
    // from the mini-status, like Midnight Commander.
    let mut reserve: u16 = if inner.height >= 3 { 2 } else { 1 };
    // The time machine takes one more row for its scrub track. Taken from the
    // scene rather than floated over it: a dialog would blank the 3D image
    // entirely (the root layer skips pixel graphics while one is up), and text
    // drawn over a Kitty or Sixel image is not visible at all.
    let scrubbing = panel.scrub.is_some() && inner.height >= 4;
    if scrubbing {
        reserve += 1;
    }

    // The tab strip takes the first interior row, but only when there is more
    // than one tab — the single-tab case (which is almost everyone, almost all
    // the time) costs nothing at all.
    let inner = render_tab_strip(f, inner, panel, active, theme);

    let list_height = inner.height.saturating_sub(reserve);
    let list_area = Rect { height: list_height, ..inner };

    match panel.format {
        ViewFormat::Full => render_full(f, list_area, panel, active, theme, nerd),
        ViewFormat::Brief => render_brief(f, list_area, panel, active, theme, brief_columns, nerd),
        ViewFormat::Tree => render_tree(f, list_area, panel, active, theme),
        ViewFormat::Space3d => render_space3d(f, list_area, panel, theme, graphics),
        ViewFormat::Thumbs => render_thumbs(f, list_area, panel, active, theme, graphics, nerd),
        ViewFormat::Activity => {
            if let Some(log) = panel.activity.as_mut() {
                let now = std::time::Instant::now();
                panel.page = crate::activity::render::render(f, list_area, log, active, theme, now);
            }
        }
        ViewFormat::Details => unreachable!("Details is rendered earlier and returns"),
    }

    // Record geometry for mouse hit-testing (offset is now post-render).
    let (body, brief, columns, rows, cell_w) = match panel.format {
        ViewFormat::Details => unreachable!("Details is rendered earlier and returns"),
        ViewFormat::Full => (
            Rect { y: list_area.y + 1, height: list_area.height.saturating_sub(1), ..list_area },
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
        // The 3D view hit-tests against projected box bounds rather than rows,
        // but the body rect is still what tells `panel_at` a click landed here.
        ViewFormat::Space3d => (list_area, false, 1usize, 1usize, list_area.width),
        // One log row per body line.
        ViewFormat::Activity => (list_area, false, 1usize, 1usize, list_area.width),
        ViewFormat::Thumbs => (list_area, false, panel.cols, panel.brief_rows, list_area.width),
    };
    let grid = (panel.format == ViewFormat::Thumbs).then(|| {
        let (cw, ch) = panel
            .thumbs
            .as_ref()
            .map_or_else(|| crate::config::ThumbSize::default().cell(), |t| t.size.cell());
        (panel.cols.max(1), cw, ch)
    });
    // The tree scrolls independently of the flat listing, so hit-testing must use
    // the tree's own offset.
    let offset = match (panel.format, panel.tree.as_ref()) {
        (ViewFormat::Tree, Some(t)) => t.offset,
        (ViewFormat::Activity, _) => panel.activity.as_ref().map_or(0, |a| a.offset),
        _ => panel.offset,
    };
    panel.hit =
        Some(crate::panel::PanelHit { area, body, brief, offset, columns, rows, cell_w, grid });

    // The scrub track sits directly under the scene, above the separator, so the
    // commit you are on reads next to the shape it produced.
    let mut next_y = inner.y + list_height;
    panel.scrub_area = None;
    if scrubbing && let Some(row) = panel.scrub.clone() {
        let track = Rect { y: next_y, height: 1, ..inner };
        render_scrub_row(f, track, &row, active, theme);
        panel.scrub_area = Some(track);
        next_y += 1;
    }

    let status_y = if reserve >= 2 {
        let sep_y = next_y;
        render_panel_separator(f, area, sep_y, border_color, theme);
        sep_y + 1
    } else {
        next_y
    };
    let status_area = Rect { y: status_y, height: 1, ..inner };
    if let Some(query) = quick_search {
        render_quick_search(f, status_area, query, panel, theme);
    } else {
        render_mini_status(f, status_area, panel, theme, nerd);
    }
}

/// Draw the time machine's scrub track: which commit the scene is showing, and
/// where that sits in the history.
///
/// Plain cell text, deliberately. It is drawn *outside* the 3D image's rect, so
/// it stays crisp at any terminal size and needs none of the pixel-baked text
/// the labels inside the scene do.
fn render_scrub_row(
    f: &mut Frame,
    area: Rect,
    row: &crate::panel::ScrubRow,
    active: bool,
    theme: &Theme,
) {
    if area.width == 0 {
        return;
    }
    let pos = format!(" {}/{} ", row.index, row.total);
    // `◀`/`▶` are clickable, and say which way time runs without a legend.
    let (lead, tail) = ("◀ ", " ▶");
    let w = area.width as usize;
    let fixed = lead.chars().count() + tail.chars().count() + pos.chars().count();
    let bar_w = w.saturating_sub(fixed).max(1);

    // Where along the bar you are. Clamped inside the bar rather than allowed to
    // reach its width: at the newest commit the marker would otherwise fall off
    // the end and the track would show no position at all.
    let filled = if row.total <= 1 {
        0
    } else {
        (row.index.saturating_sub(1) * (bar_w - 1)) / (row.total - 1)
    }
    .min(bar_w - 1);
    let mut bar = String::with_capacity(bar_w);
    for i in 0..bar_w {
        bar.push(if i == filled {
            '●'
        } else if i < filled {
            '━'
        } else {
            '─'
        });
    }

    let dim = if active { theme.panel_fg } else { theme.panel_border };
    let line = Line::from(vec![
        Span::styled(lead, Style::default().fg(theme.panel_border_active)),
        Span::styled(bar, Style::default().fg(dim)),
        Span::styled(pos, Style::default().fg(theme.panel_border_active)),
        Span::styled(tail, Style::default().fg(theme.panel_border_active)),
    ]);
    f.render_widget(Paragraph::new(line), area);
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

/// Draw the `◀` back arrow at the top-left and the `▶` forward arrow at the
/// top-right of the border, and record their screen rects on the panel for mouse
/// hit-testing. A dim arrow means there is nowhere to go in that direction.
fn render_history_arrows(
    f: &mut Frame,
    area: Rect,
    panel: &mut Panel,
    enabled: bool,
    theme: &Theme,
) {
    panel.back_arrow = None;
    panel.fwd_arrow = None;
    if !enabled || area.width < 12 {
        return;
    }
    let y = area.y;
    let bx = area.x + 1; // ◀ top-left
    let fx = area.x + area.width - 2; // ▶ top-right (just before the corner)
    let live = Style::default()
        .fg(theme.panel_border_active)
        .bg(theme.panel_bg)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let buf = f.buffer_mut();
    buf.set_string(bx, y, "◀", if panel.can_back() { live } else { dim });
    buf.set_string(fx, y, "▶", if panel.can_forward() { live } else { dim });
    panel.back_arrow = Some(Rect { x: bx, y, width: 1, height: 1 });
    panel.fwd_arrow = Some(Rect { x: fx, y, width: 1, height: 1 });
}

/// Write the current Git branch (and ahead/behind) onto the bottom-left border.
/// Capped to about half the width so it never collides with the disk readout on
/// the bottom-right.
fn render_git_branch(f: &mut Frame, area: Rect, panel: &Panel, theme: &Theme) {
    let Some(git) = panel.git.as_ref() else {
        return;
    };
    if area.height < 2 || area.width < 16 || git.branch.is_empty() {
        return;
    }
    let label = format!(" ⎇ {} ", git.branch_label());
    let maxw = (area.width as usize / 2).max(8);
    let label = ellipsize(&label, maxw);
    let y = area.y + area.height - 1;
    let style = Style::default().fg(theme.exec_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD);
    f.buffer_mut().set_string(area.x + 1, y, label, style);
}

/// Write the volume's "used / total (NN%)" onto the bottom border, right-aligned.
fn render_disk_usage(f: &mut Frame, area: Rect, panel: &Panel, border_color: Color, theme: &Theme) {
    let Some(du) = panel.disk else {
        return;
    };
    if area.height == 0 || area.width < 24 {
        return;
    }
    let text =
        format!(" {} / {} ({}%) ", human_size(du.used()), human_size(du.total), du.percent_used());
    let w = text.chars().count() as u16;
    // Keep a column of border on each side of the label.
    if w + 4 > area.width {
        return;
    }
    let y = area.y + area.height - 1;
    let x = area.x + area.width - 1 - w - 1;
    let style = Style::default().fg(border_color).bg(theme.panel_bg).add_modifier(Modifier::BOLD);
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
            None => base.fg(theme.file_fg),
        },
        VfsKind::Dir => base.fg(theme.dir_fg),
        // Anything else uses the normal foreground.
        _ => base.fg(theme.panel_fg),
    }
}

/// Map a file extension to its category accent color, if any.
///
/// The extension tables live in [`crate::util::filetype`] so the 3D view's fsn
/// style, which also shapes its file solids by category, cannot drift from what
/// the listing paints.
fn category_color(ext: &str, theme: &Theme) -> Option<Color> {
    crate::util::filetype::categorize(ext).map(|c| crate::util::filetype::category_color(c, theme))
}

/// The `ls -F`-style classify character placed before each name so types are
/// distinguished by symbol (and alignment is preserved) rather than only color:
/// `/` directory, `*` executable, `@`/`!` valid/broken symlink, ` ` otherwise.
/// With Nerd Font symbols enabled, [`entry_marker`] uses a per-type glyph
/// instead.
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
    let base = if active { theme.cursor } else { theme.cursor_inactive };
    if marked { base.fg(theme.marked_fg).add_modifier(Modifier::BOLD) } else { base }
}

fn ensure_visible(cursor: usize, offset: &mut usize, height: usize) {
    if height == 0 {
        return;
    }
    *offset = crate::util::scroll::scroll_to_visible(*offset, cursor, height);
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

fn render_full(
    f: &mut Frame,
    area: Rect,
    panel: &mut Panel,
    active: bool,
    theme: &Theme,
    nerd: bool,
) {
    let width = area.width as usize;
    let (name_w, size_w, time_w) = full_columns(width);

    // Header row, with vertical separators matching the data rows.
    let header_style =
        Style::default().fg(theme.header_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD);
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

    let body_area = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
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
        let gstate = panel.git.as_ref().and_then(|g| g.state_of(&e.name));

        let size_str = size_field(e);
        let time_str = e.mtime.map(format_time).unwrap_or_default();

        if is_cursor && active {
            // The whole row (separators included) is highlighted; marked entries
            // keep a yellow foreground so the selection stays visible. Only the
            // active panel shows a cursor.
            let text = format!(
                "{}{COL_SEP}{}{COL_SEP}{}",
                pad_right(&display_name_git(e, gstate, nerd), name_w),
                pad_left(&size_str, size_w),
                pad_left(&time_str, time_w)
            );
            if theme.truecolor {
                let fg = if marked { theme.marked_fg } else { theme.cursor_fg };
                // The row already carries the cursor's own ramp, cell by cell.
                crate::ui::gradient::mark_painted(Rect {
                    y: body_area.y + i as u16,
                    height: 1,
                    ..body_area
                });
                lines.push(gradient_line(&text, width, fg, theme));
            } else {
                lines.push(Line::from(Span::styled(text, cursor_style(true, marked, theme))));
            }
        } else {
            // Marked rows are highlighted across all columns, not just the name.
            let data_style = if marked {
                Style::default().fg(theme.marked_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD)
            } else {
                normal
            };
            let spans = vec![
                Span::styled(
                    pad_right(&display_name_git(e, gstate, nerd), name_w),
                    entry_name_style(e, marked, gstate, theme),
                ),
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
    nerd: bool,
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
    first_col = crate::util::scroll::scroll_to_visible(first_col, cursor_col, columns);
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
                    let gstate = panel.git.as_ref().and_then(|g| g.state_of(&e.name));
                    let text = pad_right(&display_name_git(e, gstate, nerd), name_w);
                    // Only the active panel shows a cursor highlight.
                    let style = if is_cursor && active {
                        cursor_style(true, marked, theme)
                    } else {
                        entry_name_style(e, marked, gstate, theme)
                    };
                    spans.push(Span::styled(text, style));
                }
                None => spans
                    .push(Span::styled(" ".repeat(name_w), Style::default().bg(theme.panel_bg))),
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

/// Draw the thumbnail grid: row by row, each cell a picture (or the file's
/// type, for what has none) over its name. Ready pictures are drawn with pixel
/// graphics by the root layer when `graphics` is on — the renderer only notes
/// where — and as half-block cell art (or an ASCII ramp) otherwise.
fn render_thumbs(
    f: &mut Frame,
    area: Rect,
    panel: &mut Panel,
    active: bool,
    theme: &Theme,
    graphics: bool,
    nerd: bool,
) {
    let size = panel.thumbs.as_ref().map_or_else(Default::default, |t| t.size);
    let (cw, ch) = size.cell();
    let cols = (area.width / cw).max(1) as usize;
    let rows = (area.height / ch).max(1) as usize;
    panel.page = cols * rows;
    panel.cols = cols;
    panel.brief_rows = rows;
    let first_row =
        crate::util::scroll::scroll_to_visible(panel.offset / cols, panel.cursor / cols, rows);
    panel.offset = first_row * cols;
    f.render_widget(Block::default().style(Style::default().bg(theme.panel_bg)), area);

    let bg_rgb = crate::ui::graphics::raster::rgb(theme.panel_bg);
    for i in 0..cols * rows {
        let idx = panel.offset + i;
        let Some(e) = panel.entries.get(idx) else { break };
        let (c, r) = ((i % cols) as u16, (i / cols) as u16);
        let cell = Rect { x: area.x + c * cw, y: area.y + r * ch, width: cw, height: ch };
        if cell.bottom() > area.bottom() || cell.right() > area.right() {
            continue;
        }
        let is_cursor = idx == panel.cursor && active;
        let marked = panel.selection.is_marked(&e.name);
        let gstate = panel.git.as_ref().and_then(|g| g.state_of(&e.name));
        // The cursor is a plate behind the picture and its name: text can't be
        // drawn over a pixel image, but the cells around one can be coloured.
        let plate = Rect { width: cw - 1, height: ch - 1, ..cell };
        let plate_bg =
            if is_cursor { theme.cursor.bg.unwrap_or(theme.panel_bg) } else { theme.panel_bg };
        f.render_widget(Block::default().style(Style::default().bg(plate_bg)), plate);
        let pic = Rect { x: cell.x + 1, y: cell.y + 1, width: cw - 3, height: ch - 3 };

        let key = crate::thumbs::ThumbKey::new(&panel.cwd, e, size, bg_rgb);
        let state = crate::thumbs::kind_of(e, &panel.cwd)
            .and_then(|_| panel.thumbs.as_mut().and_then(|t| t.get(&key)));
        match state {
            Some(crate::thumbs::ThumbState::Ready(t)) => {
                // The picture colours its own cells: a background gradient must
                // not re-ramp the ones that happen to match the panel colour.
                crate::ui::gradient::mark_painted(pic);
                if graphics {
                    panel.thumb_cells.push((pic, t.clone()));
                } else if theme.truecolor {
                    crate::util::img::render_halfblocks(f, pic, &t.img, plate_bg);
                } else {
                    crate::util::img::render_ascii_ramp(f, pic, &t.img, theme);
                }
            }
            Some(crate::thumbs::ThumbState::Loading) => {
                let dim = Style::default().fg(theme.panel_border).bg(plate_bg);
                centered_text(f, pic, "…", dim);
            }
            _ => {
                // No picture: the file's type, large enough to scan for.
                let label = type_label(e, nerd);
                let style = name_style(e, false, theme).bg(plate_bg).add_modifier(Modifier::BOLD);
                centered_text(f, pic, &label, style);
            }
        }

        let name_row = Rect { y: cell.y + ch - 2, height: 1, ..plate };
        let name = crate::util::text::ellipsize(
            &display_name_git(e, gstate, nerd),
            name_row.width as usize,
        );
        let style = if is_cursor {
            cursor_style(true, marked, theme)
        } else {
            entry_name_style(e, marked, gstate, theme).bg(plate_bg)
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(name, style)))
                .alignment(ratatui::layout::Alignment::Center),
            name_row,
        );
    }
}

/// What a grid cell without a picture shows: the Nerd Font glyph for the type,
/// or a word — `DIR`, `..`, or the extension.
fn type_label(e: &VfsEntry, nerd: bool) -> String {
    if nerd {
        return crate::panel::icons::icon(e).to_string();
    }
    match e.kind {
        VfsKind::Dir if e.name == ".." => "..".to_string(),
        VfsKind::Dir => "DIR".to_string(),
        _ => match e.extension() {
            "" => "·".to_string(),
            ext => format!(".{}", ext.to_uppercase()),
        },
    }
}

/// One line of text, centred in `area` both ways.
fn centered_text(f: &mut Frame, area: Rect, text: &str, style: Style) {
    if area.height == 0 {
        return;
    }
    let row = Rect { y: area.y + (area.height - 1) / 2, height: 1, ..area };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            crate::util::text::ellipsize(text, area.width as usize),
            style,
        )))
        .alignment(ratatui::layout::Alignment::Center),
        row,
    );
}

/// Render the 3D view, or note where the root layer should composite its image.
fn render_space3d(f: &mut Frame, area: Rect, panel: &mut Panel, theme: &Theme, graphics: bool) {
    panel.scene_area = None;
    let Some(sp) = panel.space3d.as_mut() else {
        // A non-local panel has nothing crawlable to show.
        let p = Paragraph::new(Line::from(Span::styled(
            crate::l10n::trd("3D view needs a local directory"),
            Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
        )));
        f.render_widget(p, area);
        return;
    };
    panel.scene_area = crate::space3d::render::render(f, area, sp, theme, graphics);
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
                // The row already carries the cursor's own ramp, cell by cell.
                crate::ui::gradient::mark_painted(Rect { y: area.y + i as u16, height: 1, ..area });
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

/// The marker drawn before an entry's name: the one-character `ls -F` classify
/// prefix (see [`classify_prefix`]), or — with Nerd Font symbols on — the
/// entry's type glyph followed by a space, so names stay aligned either way.
fn entry_marker(e: &VfsEntry, nerd: bool) -> String {
    if nerd { format!("{} ", crate::panel::icons::icon(e)) } else { classify_prefix(e).to_string() }
}

/// Name as shown in the list: the type marker (see [`entry_marker`]) followed by
/// the entry name.
fn display_name(e: &VfsEntry, nerd: bool) -> String {
    format!("{}{}", entry_marker(e, nerd), e.name)
}

/// Like [`display_name`], but the leading type marker is replaced by the git
/// status glyph (`M` / `+` / `?` / `!`) when the entry has a VCS state.
fn display_name_git(e: &VfsEntry, gstate: Option<crate::git::GitState>, nerd: bool) -> String {
    match gstate {
        // Keep the same width as the type marker so the names still line up.
        Some(s) if nerd => format!("{} {}", s.glyph(), e.name),
        Some(s) => format!("{}{}", s.glyph(), e.name),
        None => display_name(e, nerd),
    }
}

/// The foreground colour for a git state, reusing semantic theme colours so it
/// tracks the active theme: modified → accent, staged → exec/green, untracked →
/// dim border, conflict → error/red.
fn git_color(state: crate::git::GitState, theme: &Theme) -> Color {
    use crate::git::GitState;
    match state {
        GitState::Modified => theme.hotkey_fg,
        GitState::Staged => theme.exec_fg,
        GitState::Untracked => theme.panel_border,
        GitState::Conflict => theme.error_fg,
    }
}

/// The name style for a listing entry: a user mark wins, then a git state tints
/// the name by its status colour, otherwise the normal by-type colour.
fn entry_name_style(
    e: &VfsEntry,
    marked: bool,
    gstate: Option<crate::git::GitState>,
    theme: &Theme,
) -> Style {
    if marked {
        return name_style(e, true, theme);
    }
    match gstate {
        Some(s) => Style::default().bg(theme.panel_bg).fg(git_color(s, theme)),
        None => name_style(e, false, theme),
    }
}

fn render_mini_status(f: &mut Frame, area: Rect, panel: &Panel, theme: &Theme, nerd: bool) {
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
            Paragraph::new(Line::from(Span::styled(
                pad_right(&ellipsize(&text, width), width),
                style,
            ))),
            area,
        );
        return;
    }

    // Activity log: the event rate, and whether it is paused.
    if panel.format == ViewFormat::Activity {
        if let Some(log) = panel.activity.as_ref() {
            crate::activity::render::render_status(f, area, log, theme);
        }
        return;
    }

    // 3D view: name the box the cursor is on, with its size and share of the
    // directory total — the same readout the disk explorer gives.
    if panel.format == ViewFormat::Space3d {
        let text = panel
            .space3d
            .as_ref()
            .map(|sp| match sp.selected_node() {
                Some(d) => {
                    let more = if d.partial { "…" } else { "" };
                    // The share is of the directory above, which is meaningful
                    // wherever the cursor is; the tree's own root has none.
                    match sp.selected_share() {
                        // A real but tiny share reads as "0%", which looks like
                        // a measurement that failed rather than a small number.
                        Some(pct) if pct > 0.0 && pct < 0.5 => {
                            format!("{}  {}{}  <1%", d.name, human_size(d.size), more)
                        }
                        Some(pct) => {
                            format!("{}  {}{}  {:.0}%", d.name, human_size(d.size), more, pct)
                        }
                        None => format!("{}  {}{}", d.name, human_size(d.size), more),
                    }
                }
                None if sp.scanning => crate::l10n::trd("Scanning…"),
                None => crate::l10n::trd("(no subdirectories)"),
            })
            .unwrap_or_default();
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&ellipsize(&text, width), width),
                style,
            ))),
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
            let mut name = display_name(e, nerd);
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

/// Render the quick-search input on the panel's mini-status row: a `>` prompt
/// followed by the live query, styled like a dialog input field. Also records
/// the caret screen position on `panel.quick_caret` so the root draw can place
/// the terminal cursor there.
fn render_quick_search(f: &mut Frame, area: Rect, query: &str, panel: &mut Panel, theme: &Theme) {
    let prompt = ">";
    let prompt_style = Style::default().fg(theme.cursor_fg).bg(theme.input_bg);
    let text_style = Style::default().fg(theme.input_fg).bg(theme.input_bg);
    let line =
        Line::from(vec![Span::styled(prompt, prompt_style), Span::styled(query, text_style)]);
    f.render_widget(Paragraph::new(line), area);
    // Fill the remainder of the row with the input background so the field
    // reads as a single solid bar (mirroring dialog input fields).
    let taken = prompt.chars().count() + query.chars().count();
    let width = area.width as usize;
    if width > taken {
        let fill = Span::styled(" ".repeat(width - taken), text_style);
        let fill_area = Rect { x: area.x + taken as u16, width: (width - taken) as u16, ..area };
        f.render_widget(Paragraph::new(Line::from(fill)), fill_area);
    }
    let caret_x = area.x + taken as u16;
    panel.quick_caret = Some(ratatui::layout::Position::new(caret_x, area.y));
}

/// Draw the tab strip along the top of the panel interior, returning the area
/// left for the listing. A single tab draws nothing and returns `inner` intact.
///
/// Records each label's rect on the panel for click hit-testing, the same way
/// the history arrows do.
fn render_tab_strip(
    f: &mut Frame,
    inner: Rect,
    panel: &mut Panel,
    active: bool,
    theme: &Theme,
) -> Rect {
    panel.tab_hits.clear();
    if panel.tabs.len() < 2 || inner.height < 2 {
        return inner;
    }
    let row = Rect { height: 1, ..inner };
    let rest = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };

    let base = theme.panel_base();
    f.render_widget(Paragraph::new("").style(base), row);

    // Share the width evenly, but never below a legible minimum; anything that
    // doesn't fit is simply not drawn (the picker lists them all anyway).
    let count = panel.tabs.len() as u16;
    let width = (inner.width / count).max(6);
    let mut x = inner.x;
    for (i, tab) in panel.tabs.iter().enumerate() {
        if x >= inner.x + inner.width {
            break;
        }
        let w = width.min(inner.x + inner.width - x);
        let cell = Rect { x, y: row.y, width: w, height: 1 };
        let name = tab_label(&tab.cwd);
        let text = pad_right(&ellipsize(&name, w as usize), w as usize);
        let style = if i == panel.tab && active {
            theme.cursor
        } else if i == panel.tab {
            theme.cursor_inactive
        } else {
            base.fg(theme.panel_border)
        };
        f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), cell);
        panel.tab_hits.push((cell, i));
        x += w;
    }
    rest
}

/// A tab's label: the directory's own name, which is what distinguishes tabs in
/// practice. Falls back to the full path at a filesystem root.
fn tab_label(cwd: &VfsPath) -> String {
    let name = cwd.file_name();
    if name.is_empty() { cwd.display() } else { name }
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
            symlink_dir: false,
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
            term.draw(|t| {
                render_panel(
                    t,
                    t.area(),
                    &mut panel,
                    true,
                    &Default::default(),
                    &theme,
                    2,
                    None,
                    false,
                    false,
                )
            })
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
            assert_eq!(
                mini_seps,
                vec![x0, x1],
                "{fmt:?}: mini-status columns align with the listing"
            );
            // Size and modify date are both shown.
            let text: String = (0..b.area.width).map(|x| b[(x, mini_row)].symbol()).collect();
            assert!(text.contains("9.4M"), "{fmt:?}: size shown in the mini-status");
            assert!(text.contains("1970"), "{fmt:?}: modify date shown in the mini-status");
        }
    }

    #[test]
    fn git_glyphs_and_branch_render() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/repo"));
        panel.entries = vec![
            entry("mod.rs", VfsKind::File, 0o644, false),
            entry("new.rs", VfsKind::File, 0o644, false),
            entry("clean.rs", VfsKind::File, 0o644, false),
        ];
        let mut files = std::collections::HashMap::new();
        files.insert("mod.rs".to_string(), crate::git::GitState::Modified);
        files.insert("new.rs".to_string(), crate::git::GitState::Untracked);
        panel.git = Some(crate::git::GitStatus {
            branch: "main".into(),
            ahead: 2,
            behind: 0,
            files,
            root: "/repo".into(),
        });

        let mut t = Terminal::new(TestBackend::new(44, 8)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let b = t.backend().buffer();
        let all: String = (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .map(|(x, y)| b[(x, y)].symbol().to_string())
            .collect();
        assert!(all.contains(">mod.rs"), "modified file shows the > glyph: {all:?}");
        assert!(all.contains("?new.rs"), "untracked file shows the ? glyph");
        assert!(all.contains(" clean.rs"), "a clean file keeps its plain marker");
        // The branch + ahead count is on the bottom border.
        let bottom: String =
            (0..b.area.width).map(|x| b[(x, b.area.height - 1)].symbol()).collect();
        assert!(bottom.contains("main"), "branch on the border: {bottom:?}");
        assert!(bottom.contains("↑2"), "ahead count on the border: {bottom:?}");
    }

    #[test]
    fn a_single_tab_draws_no_strip() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp/here"));
        panel.entries = vec![entry("a.txt", VfsKind::File, 0o644, false)];

        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let b = t.backend().buffer();
        // The first interior row is the Full view's column header, i.e. the
        // listing starts immediately — no row was given up to a tab strip.
        let first: String = (0..b.area.width).map(|x| b[(x, 1)].symbol()).collect();
        assert!(first.contains("Name"), "the listing header is at the top: {first:?}");
        let second: String = (0..b.area.width).map(|x| b[(x, 2)].symbol()).collect();
        assert!(second.contains("a.txt"), "and the first entry right below: {second:?}");
        assert!(panel.tab_hits.is_empty(), "and nothing is clickable as a tab");
    }

    #[test]
    fn a_tab_strip_renders_and_records_click_targets() {
        use crate::panel::tabs::TabState;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp/here"));
        panel.entries = vec![entry("a.txt", VfsKind::File, 0o644, false)];
        panel.tabs = vec![
            TabState::new(crate::vfs::VfsPath::local("/tmp/here"), panel.format, panel.sort),
            TabState::new(crate::vfs::VfsPath::local("/tmp/other"), panel.format, panel.sort),
        ];
        panel.tab = 0;

        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let b = t.backend().buffer();

        // Row 1 (first interior row) is the strip, naming both directories.
        let strip: String = (0..b.area.width).map(|x| b[(x, 1)].symbol()).collect();
        assert!(strip.contains("here"), "the active tab is named: {strip:?}");
        assert!(strip.contains("other"), "and so is the other: {strip:?}");
        // The listing has moved down to make room (row 2 is the column header
        // in Full view, so the entry itself lands below that).
        let below: String = (2..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect();
        assert!(below.contains("a.txt"), "listing pushed below the strip: {below:?}");
        let strip_row: String = (0..b.area.width).map(|x| b[(x, 1)].symbol()).collect();
        assert!(!strip_row.contains("a.txt"), "the strip row is not the listing");

        // Both labels are clickable, and their rects are on the strip row.
        assert_eq!(panel.tab_hits.len(), 2);
        assert!(panel.tab_hits.iter().all(|(r, _)| r.y == 1));
        assert_eq!(panel.tab_hits[0].1, 0);
        assert_eq!(panel.tab_hits[1].1, 1);
    }

    #[test]
    fn history_arrows_and_filter_badge_render() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp/here"));
        panel.entries = vec![entry("a.txt", VfsKind::File, 0o644, false)];
        // Something to go back to, and an active listing filter.
        panel.back.push(crate::vfs::VfsPath::local("/tmp"));
        panel.filter = Some("*.rs".to_string());

        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let b = t.backend().buffer();
        let top: String = (0..b.area.width).map(|x| b[(x, 0)].symbol()).collect();
        assert!(top.contains('◀') && top.contains('▶'), "history arrows drawn: {top:?}");
        // ◀ sits at the top-left, ▶ at the top-right (before the corner).
        assert_eq!(panel.back_arrow.unwrap().x, 1, "back arrow at the top-left");
        assert_eq!(panel.fwd_arrow.unwrap().x, b.area.width - 2, "forward arrow at the top-right");
        assert_eq!(b[(1, 0)].symbol(), "◀");
        assert_eq!(b[(b.area.width - 2, 0)].symbol(), "▶");
        assert!(top.contains("[*.rs]"), "active filter surfaced in the title: {top:?}");
    }

    /// Build a panel sitting in the 3D view over a small hand-made size cache.
    fn space3d_panel(truecolor: bool) -> (Panel, Theme) {
        let mut theme = Theme::mc();
        theme.truecolor = truecolor;
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp"));
        panel.format = ViewFormat::Space3d;
        let mut sp = crate::space3d::Space3d::new(std::path::PathBuf::from("/tmp"));
        let mut tree = crate::sizes::SizeTree::new();
        let r = tree.ensure(std::path::Path::new("/tmp"));
        tree.mark_listed(r);
        for (name, size) in [("alpha", 9_000_000u64), ("beta", 3_000_000), ("gamma", 400_000)] {
            let p = std::path::PathBuf::from("/tmp").join(name);
            let id = tree.ensure(&p);
            tree.mark_listed(id);
            tree.add_file(id, &p.join("f"), size);
        }
        sp.sync_from(&tree);
        // Run the size animation to completion so the boxes have their real
        // heights rather than starting from zero.
        let mut now = std::time::Instant::now();
        for _ in 0..500 {
            now += std::time::Duration::from_millis(33);
            sp.advance(now);
        }
        panel.space3d = Some(sp);
        (panel, theme)
    }

    fn screen(t: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        s
    }

    /// Without a graphics protocol the 3D view draws itself as half-block cells.
    #[test]
    fn space3d_draws_half_blocks_on_a_truecolor_terminal() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut panel, theme) = space3d_panel(true);
        let mut t = Terminal::new(TestBackend::new(70, 22)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let s = screen(&t);
        assert!(s.contains('▀'), "the scene is drawn as half-block cells");
        assert!(panel.scene_area.is_none(), "nothing is deferred to the root layer");
        // The cursor starts on the focus — the directory the other panel is on —
        // and the mini-status names whatever it is on.
        assert!(s.contains("tmp"), "mini-status names the selected directory");
    }

    /// In the cell-art modes the names are drawn as ordinary terminal text, not
    /// baked into the raster: a name downsampled into half-blocks is an
    /// illegible smudge, and here the cells are ours to write on.
    #[test]
    fn space3d_labels_are_real_text_in_the_cell_modes() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        for truecolor in [true, false] {
            let (mut panel, theme) = space3d_panel(truecolor);
            let mut t = Terminal::new(TestBackend::new(90, 30)).unwrap();
            t.draw(|f| {
                render_panel(
                    f,
                    f.area(),
                    &mut panel,
                    true,
                    &Default::default(),
                    &theme,
                    2,
                    None,
                    false,
                    false,
                )
            })
            .unwrap();
            let s = screen(&t);
            // The body carries the directory names as readable characters.
            assert!(s.contains("alpha"), "no text label (truecolor={truecolor})");
        }
    }

    /// Names get the room the box actually offers, rather than being clipped to
    /// a few characters.
    #[test]
    fn space3d_labels_are_not_cut_short_when_there_is_room() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut theme = Theme::mc();
        theme.truecolor = true;
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp"));
        panel.format = ViewFormat::Space3d;
        let mut sp = crate::space3d::Space3d::new(std::path::PathBuf::from("/tmp"));
        let mut tree = crate::sizes::SizeTree::new();
        let r = tree.ensure(std::path::Path::new("/tmp"));
        tree.mark_listed(r);
        for (name, size) in [("Photographs", 9_000_000u64), ("Downloads", 8_000_000)] {
            let p = std::path::PathBuf::from("/tmp").join(name);
            let id = tree.ensure(&p);
            tree.mark_listed(id);
            tree.add_file(id, &p.join("f"), size);
        }
        sp.sync_from(&tree);
        let mut now = std::time::Instant::now();
        for _ in 0..500 {
            now += std::time::Duration::from_millis(33);
            sp.advance(now);
        }
        panel.space3d = Some(sp);

        let mut t = Terminal::new(TestBackend::new(120, 34)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let s = screen(&t);
        assert!(s.contains("Photographs"), "an 11-character name fits whole: {s:?}");
    }

    /// On a graphics terminal the names must stay baked into the image: cell
    /// text drawn over a Kitty/Sixel raster is never shown.
    #[test]
    fn space3d_labels_are_not_cell_text_under_graphics() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut panel, theme) = space3d_panel(true);
        let mut t = Terminal::new(TestBackend::new(90, 30)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                true,
                false,
            )
        })
        .unwrap();
        let s = screen(&t);
        // The mini-status still names the selection; the scene body does not.
        let body: String = {
            let b = t.backend().buffer();
            let mut out = String::new();
            for y in 2..b.area.height - 3 {
                for x in 1..b.area.width - 1 {
                    out.push_str(b[(x, y)].symbol());
                }
            }
            out
        };
        assert!(!body.contains("alpha"), "the scene body must not carry cell text");
        assert!(s.contains("tmp"), "but the mini-status still reads");
    }

    /// Helper: draw a 3D panel and return its cell buffer as rows of text.
    fn draw_space3d(panel: &mut Panel, theme: &Theme, w: u16, h: u16) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                panel,
                true,
                &Default::default(),
                theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let b = t.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect())
            .collect()
    }

    /// The time machine takes a row of the scene for its track rather than
    /// floating over it — text drawn on top of a Kitty or Sixel image is not
    /// visible at all, and a dialog would blank the image outright.
    #[test]
    fn the_scrub_row_is_drawn_inside_the_panel_and_costs_the_scene_a_row() {
        let (mut panel, theme) = space3d_panel(true);
        let without = draw_space3d(&mut panel, &theme, 70, 22);

        panel.scrub = Some(crate::panel::ScrubRow {
            label: "a9ef3a7 Made it static".into(),
            index: 3,
            total: 10,
        });
        let with = draw_space3d(&mut panel, &theme, 70, 22);

        // Matched by the position marker: the panel's *title* row carries ◀/▶
        // too, for the directory history arrows.
        let track = with.iter().find(|r| r.contains('●')).expect("no scrub row in {with:?}");
        assert!(track.contains("3/10"), "it says where you are: {track:?}");
        assert!(track.contains('●'), "and marks it on the bar: {track:?}");

        // The marker stays on the bar at both ends, rather than falling off it.
        for (index, total) in [(1, 10), (10, 10), (1, 1)] {
            panel.scrub = Some(crate::panel::ScrubRow { label: "x".into(), index, total });
            let rows = draw_space3d(&mut panel, &theme, 70, 22);
            assert!(rows.iter().any(|r| r.contains('●')), "no marker at {index}/{total}: {rows:?}");
        }
        assert!(panel.scrub_area.is_some(), "and records where it landed, for clicks");

        // One row of scene given up, not borrowed on top of it.
        let scene_rows = |rows: &[String]| rows.iter().filter(|r| r.contains('▀')).count();
        assert_eq!(scene_rows(&with) + 1, scene_rows(&without), "exactly one row");
    }

    /// A scrub changes exactly one thing the track has no room for: which commit
    /// the scene is of. That goes in the title.
    #[test]
    fn the_panel_title_names_the_commit_under_the_time_machine() {
        let (mut panel, theme) = space3d_panel(true);
        panel.scrub = Some(crate::panel::ScrubRow {
            label: "a9ef3a7 Made it static".into(),
            index: 1,
            total: 2,
        });
        let rows = draw_space3d(&mut panel, &theme, 70, 22);
        assert!(rows[0].contains("a9ef3a7 Made it static"), "got {:?}", rows[0]);
        assert!(!rows[0].contains("3D Directory View"), "the commit replaces the generic name");
    }

    /// A panel too short to give up a row must still draw, rather than leaving
    /// the scene no height at all.
    #[test]
    fn a_tiny_panel_under_the_time_machine_still_renders() {
        let (mut panel, theme) = space3d_panel(true);
        panel.scrub =
            Some(crate::panel::ScrubRow { label: "a9ef3a7 x".into(), index: 1, total: 1 });
        let rows = draw_space3d(&mut panel, &theme, 30, 5);
        assert_eq!(rows.len(), 5, "it drew without panicking");
    }

    /// The 3D view describes the *other* panel, so its own directory is not what
    /// is on screen — showing it in the title just names wherever this panel
    /// happened to be when the view was switched on.
    #[test]
    fn space3d_titles_itself_rather_than_naming_a_stale_directory() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut panel, theme) = space3d_panel(true);
        panel.cwd = crate::vfs::VfsPath::local("/some/where/else");
        panel.filter = Some("*.rs".into());
        let mut t = Terminal::new(TestBackend::new(70, 22)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let b = t.backend().buffer();
        let top: String = (0..b.area.width).map(|x| b[(x, 0)].symbol()).collect();
        assert!(top.contains("3D Directory View"), "got title {top:?}");
        assert!(!top.contains("else"), "the panel's own directory is not named");
        assert!(!top.contains("*.rs"), "nor a filter that cannot apply to it");
    }

    /// With graphics available the panel only claims the area; the root layer
    /// composites the pixel image, so no cell art is drawn.
    #[test]
    fn space3d_defers_to_the_root_layer_when_graphics_are_available() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut panel, theme) = space3d_panel(true);
        let mut t = Terminal::new(TestBackend::new(70, 22)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                true,
                false,
            )
        })
        .unwrap();
        assert!(panel.scene_area.is_some(), "the target rect is handed to the root layer");
        assert!(!screen(&t).contains('▀'), "and no cell art is drawn over it");
    }

    /// Without truecolor, half-blocks would be a smear of approximated colours,
    /// so an ASCII luminance ramp is drawn instead.
    #[test]
    fn space3d_falls_back_to_an_ascii_ramp_without_truecolor() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut panel, theme) = space3d_panel(false);
        let mut t = Terminal::new(TestBackend::new(70, 22)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        let s = screen(&t);
        assert!(!s.contains('▀'), "no half-blocks without truecolor");
        assert!(
            s.contains('#') || s.contains('%') || s.contains('@') || s.contains('*'),
            "the luminance ramp is drawn"
        );
    }

    /// A panel too narrow for a legible scene says so instead of drawing mush.
    #[test]
    fn space3d_in_a_tiny_panel_says_so_and_does_not_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut panel, theme) = space3d_panel(true);
        for (w, h) in [(14u16, 6u16), (5, 3), (3, 3)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| {
                render_panel(
                    f,
                    f.area(),
                    &mut panel,
                    true,
                    &Default::default(),
                    &theme,
                    2,
                    None,
                    false,
                    false,
                )
            })
            .unwrap();
        }
        // At a size that still has room for the message, it is shown.
        let mut t = Terminal::new(TestBackend::new(20, 8)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        assert!(screen(&t).contains("too small"), "the panel explains itself");
    }

    /// A non-local panel has no crawlable sizes, so it explains rather than
    /// rendering an empty floor.
    #[test]
    fn space3d_on_a_remote_panel_explains_itself() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut panel, theme) = space3d_panel(true);
        panel.space3d = None; // as `build_space3d` leaves it for a remote cwd
        let mut t = Terminal::new(TestBackend::new(70, 22)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
        .unwrap();
        assert!(screen(&t).contains("local directory"), "says why there is nothing to draw");
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
        t.draw(|f| render_brief(f, f.area(), &mut panel, true, &theme, 3, false)).unwrap();
        assert_eq!(panel.cols, 3, "renderer records the configured column count");
        assert_eq!(panel.page, 8 * 3, "page = rows × columns");
    }

    /// A thumbnail grid panel over `names`, its cache holding a ready 40×20
    /// red picture for every `.png`.
    fn thumbs_panel(names: &[&str]) -> Panel {
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/pics"));
        panel.entries = names.iter().map(|n| entry(n, VfsKind::File, 0o644, false)).collect();
        panel.format = ViewFormat::Thumbs;
        let mut cache = crate::thumbs::ThumbCache::default();
        let bg = crate::ui::graphics::raster::rgb(Theme::mc().panel_bg);
        for e in panel.entries.iter().filter(|e| e.name.ends_with(".png")) {
            let key = crate::thumbs::ThumbKey::new(&panel.cwd, e, cache.size, bg);
            cache.start(key.clone());
            let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([220, 0, 0, 255]));
            cache.finish(key, Some(std::sync::Arc::new(crate::thumbs::Thumb { sig: 1, img })));
        }
        panel.thumbs = Some(cache);
        panel
    }

    #[test]
    fn the_thumbnail_grid_lays_out_row_by_row_with_names_under_pictures() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let mut panel = thumbs_panel(&["a.png", "b.txt", "c.png", "d.png", "e.png"]);
        // Medium cells are 18×9: a 40×20 area holds 2 columns and 2 rows.
        let mut t = Terminal::new(TestBackend::new(40, 20)).unwrap();
        t.draw(|f| render_thumbs(f, f.area(), &mut panel, true, &theme, false, false)).unwrap();
        assert_eq!((panel.cols, panel.brief_rows, panel.page), (2, 2, 4));
        let b = t.backend().buffer();
        let row = |y: u16| -> String { (0..40).map(|x| b[(x, y)].symbol()).collect() };
        assert!(row(7).contains("a.png") && row(7).contains("b.txt"), "{:?}", row(7));
        assert!(row(16).contains("c.png") && row(16).contains("d.png"), "{:?}", row(16));
        assert!(row(3).contains(".TXT"), "a file without a picture shows its type: {:?}", row(3));
        // Without pixel graphics, a ready picture is drawn as half-blocks.
        assert_eq!(b[(8, 4)].symbol(), "▀");
        assert_eq!(b[(8, 4)].fg, ratatui::style::Color::Rgb(220, 0, 0));

        // The fifth entry is on the next page; putting the cursor there scrolls
        // by a whole row.
        panel.cursor = 4;
        t.draw(|f| render_thumbs(f, f.area(), &mut panel, true, &theme, true, false)).unwrap();
        assert_eq!(panel.offset, 2, "scrolled one row");
        // With pixel graphics, the ready pictures are handed to the root layer.
        assert_eq!(panel.thumb_cells.len(), 3, "c.png, d.png and e.png");
    }

    #[test]
    fn a_click_on_the_grid_maps_row_by_row() {
        let hit = crate::panel::PanelHit {
            area: Rect::new(0, 0, 40, 20),
            body: Rect::new(1, 1, 38, 18),
            brief: false,
            offset: 2,
            columns: 2,
            rows: 2,
            cell_w: 18,
            grid: Some((2, 18, 9)),
        };
        assert_eq!(hit.index_at(2, 2, 10), Some(2), "first cell of the first row on screen");
        assert_eq!(hit.index_at(20, 2, 10), Some(3), "second cell");
        assert_eq!(hit.index_at(2, 11, 10), Some(4), "next row");
        assert_eq!(hit.index_at(37, 2, 10), None, "the leftover strip right of the grid");
        assert_eq!(hit.index_at(20, 11, 5), None, "past the end of the listing");
    }

    #[tokio::test]
    async fn tree_view_renders_markers_and_selected_path() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
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
        term.draw(|t| {
            render_panel(
                t,
                t.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                false,
            )
        })
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
        assert_eq!(display_name(&entry("dir", VfsKind::Dir, 0o755, false), false), "/dir");
        assert_eq!(display_name(&entry("file", VfsKind::File, 0o644, false), false), " file");
    }

    #[test]
    fn nerd_markers_replace_the_classify_characters() {
        use crate::panel::icons;
        let dir = entry("src", VfsKind::Dir, 0o755, false);
        let rs = entry("main.rs", VfsKind::File, 0o644, false);
        // Off: the `ls -F` prefix. On: the type glyph plus a space, so both
        // spellings occupy the same leading columns and the names stay aligned.
        assert_eq!(display_name(&dir, false), "/src");
        assert_eq!(display_name(&dir, true), format!("{} src", icons::icon(&dir)));
        assert_eq!(display_name(&rs, false), " main.rs");
        assert_eq!(display_name(&rs, true), format!("{} main.rs", icons::icon(&rs)));
        assert_eq!(
            display_name(&dir, true).chars().count(),
            display_name(&dir, false).chars().count() + 1
        );
        // A git state still wins over the type marker, in both spellings.
        let modified = Some(crate::git::GitState::Modified);
        assert_eq!(display_name_git(&rs, modified, false), ">main.rs");
        assert_eq!(display_name_git(&rs, modified, true), "> main.rs");
    }

    #[test]
    fn listing_renders_nerd_glyphs_when_enabled() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp"));
        panel.entries = vec![
            entry("src", VfsKind::Dir, 0o755, false),
            entry("main.rs", VfsKind::File, 0o644, false),
        ];

        let mut t = Terminal::new(TestBackend::new(44, 8)).unwrap();
        t.draw(|f| {
            render_panel(
                f,
                f.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                None,
                false,
                true,
            )
        })
        .unwrap();
        let b = t.backend().buffer();
        let all: String = (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .map(|(x, y)| b[(x, y)].symbol().to_string())
            .collect();
        let folder = crate::panel::icons::icon(&entry("src", VfsKind::Dir, 0o755, false));
        let rust = crate::panel::icons::icon(&entry("main.rs", VfsKind::File, 0o644, false));
        assert!(all.contains(&format!("{folder} src")), "the directory shows its glyph");
        assert!(all.contains(&format!("{rust} main.rs")), "the file shows its type glyph");
        assert!(!all.contains("/src"), "the `ls -F` prefix is replaced, not doubled");
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
        // An archive file gets the archive color.
        assert_eq!(
            name_style(&entry("a.zip", VfsKind::File, 0o644, false), false, &t).fg,
            Some(t.archive_fg)
        );
        // A regular file uses the dedicated normal-file color…
        assert_eq!(
            name_style(&entry("notes.dat", VfsKind::File, 0o644, false), false, &t).fg,
            Some(t.file_fg)
        );
        // …a directory uses the directory color…
        assert_eq!(
            name_style(&entry("subdir", VfsKind::Dir, 0o755, false), false, &t).fg,
            Some(t.dir_fg)
        );
        // …and an executable stays green.
        assert_eq!(
            name_style(&entry("run.sh", VfsKind::File, 0o755, false), false, &t).fg,
            Some(t.exec_fg)
        );
    }

    #[test]
    fn quick_search_renders_input_and_sets_caret() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::mc();
        let backend = crate::vfs::registry::Registry::default().local();
        let mut panel = Panel::new(backend, crate::vfs::VfsPath::local("/tmp"));
        panel.entries = vec![
            entry("alpha", VfsKind::File, 0o644, false),
            entry("hello", VfsKind::File, 0o644, false),
            entry("hi", VfsKind::File, 0o644, false),
        ];
        panel.cursor = 2; // jumped to "hi" by the search

        let mut term = Terminal::new(TestBackend::new(40, 8)).unwrap();
        term.draw(|t| {
            render_panel(
                t,
                t.area(),
                &mut panel,
                true,
                &Default::default(),
                &theme,
                2,
                Some("hi"),
                false,
                false,
            )
        })
        .unwrap();
        let b = term.backend().buffer();
        // The mini-status row is the last interior row of the panel (border is
        // the bottom row, so the status line sits at height-2).
        let row = b.area.height - 2;
        let text: String = (0..b.area.width).map(|x| b[(x, row)].symbol()).collect();
        assert!(text.starts_with("│>hi"), "quick-search input shows the query: {text:?}");
        // The caret lands just after the prompt + query (">" + "hi" = 3 cols
        // past the panel's interior, which starts at x=1 inside the border).
        assert_eq!(panel.quick_caret, Some(ratatui::layout::Position::new(4, row)));
        assert!(!text.contains("DIR"), "mini-status is hidden during quick search");
    }
}
