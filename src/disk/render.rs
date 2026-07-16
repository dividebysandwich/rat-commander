//! Rendering of the [`DiskView`] treemap.

use super::{human_gb, DiskEntry, DiskView};
use crate::ui::graphics::{raster, Gfx, Slot};
use crate::ui::theme::Theme;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

pub fn render(f: &mut Frame, area: Rect, dv: &mut DiskView, theme: &Theme, gfx: Option<&mut Gfx>) {
    let title = format!(
        " {} — {}  ({}) ",
        crate::l10n::trd("Disk Explorer"),
        dv.cwd.display(),
        human_gb(dv.total())
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.panel_border_active).bg(theme.panel_bg))
        .title(Span::styled(
            crate::util::text::ellipsize(&title, area.width.saturating_sub(2) as usize),
            Style::default()
                .fg(theme.panel_border_active)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme.panel_base());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 || inner.width < 4 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // selected-box readout
            Constraint::Min(1),    // treemap
            Constraint::Length(1), // shortcut bar
        ])
        .split(inner);
    let header = rows[0];
    let body = rows[1];
    render_footer(f, rows[2], theme);

    dv.rects.clear();
    if dv.scanning {
        render_header(f, header, None, 0, theme);
        render_scanning(f, body, dv.scan_done, dv.scan_total, theme, gfx);
        return;
    }
    if dv.entries.is_empty() {
        render_header(f, header, None, 0, theme);
        center_text(f, body, &crate::l10n::trd("(no subdirectories)"), theme);
        return;
    }
    let selected = dv.entries.get(dv.selected.min(dv.entries.len() - 1));
    render_header(f, header, selected, dv.total(), theme);

    let rects = treemap(&dv.entries, body);
    dv.rects = rects.clone();
    if dv.selected >= dv.entries.len() {
        dv.selected = dv.entries.len() - 1;
    }
    let n = dv.entries.len();
    match gfx {
        // Graphics terminal: draw the whole treemap as one image of nested
        // "pillow" boxes, then overlay the text labels on top.
        Some(g) if g.available() => {
            render_treemap_graphics(f, body, &dv.entries, &rects, dv.selected, theme, g);
        }
        // Fallback: classic character-cell boxes.
        _ => {
            for (i, (entry, rect)) in dv.entries.iter().zip(rects.iter()).enumerate() {
                draw_box(f, *rect, entry, i == dv.selected, i, n, theme);
            }
        }
    }
}

/// Show the selected box's name and size at the top, so the selection is always
/// legible even when its box is too small to render a label.
fn render_header(f: &mut Frame, area: Rect, selected: Option<&DiskEntry>, total: u64, theme: &Theme) {
    let spans = match selected {
        Some(e) => {
            let pct = if total > 0 {
                100.0 * e.size as f32 / total as f32
            } else {
                0.0
            };
            vec![
                Span::styled(
                    " ▶ ",
                    Style::default().fg(theme.panel_border_active).bg(theme.panel_bg),
                ),
                Span::styled(
                    ellipsize(&e.name, area.width.saturating_sub(28) as usize),
                    Style::default()
                        .fg(theme.cursor_fg)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("   {}   {:.0}% {}", human_gb(e.size), pct, crate::l10n::trd("of total")),
                    Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
                ),
            ]
        }
        None => vec![Span::styled(
            format!(" {}", crate::l10n::trd("(nothing selected)")),
            Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
        )],
    };
    f.render_widget(Paragraph::new(Line::from(spans)).style(theme.panel_base()), area);
}

fn render_footer(f: &mut Frame, area: Rect, theme: &Theme) {
    let hint = "←↑↓→/click move   Enter/dbl-click open   g go to dir   Bksp up   Esc close";
    // Draw as a highlighted bar (like the F-key row) so it's clearly visible.
    let line = pad_right(&format!(" {}", crate::l10n::trd(hint)), area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(line, theme.fkey_label)))
            .style(theme.fkey_label),
        area,
    );
}

/// Show scanning progress: a centered label and a horizontal progress bar. The
/// bar is determinate once the subdirectory count is known, indeterminate (just
/// a label) before that.
fn render_scanning(
    f: &mut Frame,
    area: Rect,
    done: usize,
    total: usize,
    theme: &Theme,
    gfx: Option<&mut Gfx>,
) {
    let label = if total > 0 {
        format!("{} {done} / {total} {}", crate::l10n::trd("Scanning…"), crate::l10n::trd("directories"))
    } else {
        crate::l10n::trd("Scanning… (enumerating directories)")
    };
    let mid = area.y + area.height / 2;
    f.render_widget(
        Paragraph::new(Line::from(label))
            .alignment(Alignment::Center)
            .style(theme.panel_base()),
        Rect { y: mid.saturating_sub(1), height: 1, ..area },
    );

    if total == 0 {
        return;
    }
    // A centered bar ~60% of the body width.
    let bar_w = (area.width as usize * 3 / 5).clamp(10, area.width as usize);
    let bar_x = area.x + (area.width as usize - bar_w) as u16 / 2;
    let ratio = (done as f32 / total as f32).clamp(0.0, 1.0);

    // Graphics path: a gradient pill in the panel accent color.
    if let Some(g) = gfx
        && g.available() {
            let bar_area = Rect { x: bar_x, y: mid + 1, height: 1, width: bar_w as u16 };
            let (pw, ph) = g.px_size(bar_area);
            let base = raster::rgb(theme.panel_border_active);
            let dark = raster::over((0, 0, 0), base, 0.55);
            let bright = raster::over(base, (255, 255, 255), 0.30);
            let img = raster::gradient_bar(
                pw,
                ph,
                ratio as f64,
                |t| raster::over(dark, bright, t),
                raster::rgb(theme.panel_border),
                raster::rgb(theme.panel_bg),
            );
            g.draw(f, bar_area, Slot::DiskScanBar, img);
            f.render_widget(
                Paragraph::new(Line::from(format!("{:.0}%", ratio * 100.0)))
                    .style(theme.panel_base()),
                Rect { x: bar_x + bar_w as u16 + 1, y: mid + 1, height: 1, width: 5 },
            );
            return;
        }

    let filled = (ratio * bar_w as f32).round() as usize;
    let mut spans = Vec::with_capacity(bar_w + 1);
    for i in 0..bar_w {
        // Use the same animated pulse fill as the file-copy progress bars.
        let (ch, color) = if i < filled {
            ('█', crate::ui::dialog::pulse_fill(theme, theme.panel_border_active, i, bar_w))
        } else {
            ('░', theme.panel_border)
        };
        spans.push(Span::styled(ch.to_string(), Style::default().fg(color).bg(theme.panel_bg)));
    }
    spans.push(Span::styled(
        format!(" {:.0}%", ratio * 100.0),
        Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
    ));
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.panel_base()),
        Rect { x: bar_x, y: mid + 1, height: 1, width: area.width - (bar_x - area.x) },
    );
}

fn center_text(f: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    let row = Rect {
        y: area.y + area.height / 2,
        height: 1,
        ..area
    };
    f.render_widget(
        Paragraph::new(Line::from(text.to_string()))
            .alignment(Alignment::Center)
            .style(theme.panel_base()),
        row,
    );
}

/// Character-cell rendering of one treemap box (the fallback used when there is
/// no terminal-graphics protocol).
fn draw_box(f: &mut Frame, rect: Rect, entry: &DiskEntry, selected: bool, idx: usize, n: usize, theme: &Theme) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let color = if selected {
        theme.panel_border_active
    } else if theme.truecolor {
        theme.gradient_at(idx, n.max(1))
    } else {
        theme.panel_border
    };

    // Tiny boxes: just a colored block (no room for a border or labels).
    if rect.width < 4 || rect.height < 3 {
        let style = Style::default().fg(color).bg(theme.panel_bg);
        let buf = f.buffer_mut();
        for yy in rect.y..rect.y + rect.height {
            buf.set_string(rect.x, yy, "█".repeat(rect.width as usize), style);
        }
        return;
    }

    let mut border = Style::default().fg(color).bg(theme.panel_bg);
    if selected {
        border = border.add_modifier(Modifier::BOLD);
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .style(theme.panel_base());
    let bi = block.inner(rect);
    f.render_widget(block, rect);
    if bi.width == 0 || bi.height == 0 {
        return;
    }

    let name_style = if selected {
        Style::default().fg(theme.cursor_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.panel_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD)
    };
    let size_style = Style::default().fg(color).bg(theme.panel_bg);

    let name = ellipsize(&entry.name, bi.width as usize);
    let size = human_gb(entry.size);
    if bi.height >= 2 {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(name, name_style)))
                .alignment(Alignment::Center),
            Rect { height: 1, ..bi },
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(ellipsize(&size, bi.width as usize), size_style)))
                .alignment(Alignment::Center),
            Rect { y: bi.y + 1, height: 1, ..bi },
        );
        // Big enough: list the largest files (path relative to this box + size).
        if bi.height >= 5 && bi.width >= 16 && !entry.files.is_empty() {
            let list = Rect { y: bi.y + 3, height: bi.height - 3, ..bi };
            draw_file_list(f, list, &entry.files, color, theme);
        }
    } else {
        // One interior row: show the name only.
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(name, name_style)))
                .alignment(Alignment::Center),
            bi,
        );
    }
}

/// List the biggest files inside a box: each row is `relative/path … SIZE`,
/// the path left-aligned (dim) and the size right-aligned in the box color.
fn draw_file_list(
    f: &mut Frame,
    area: Rect,
    files: &[super::FileEntry],
    color: ratatui::style::Color,
    theme: &Theme,
) {
    let w = area.width as usize;
    let rows = area.height as usize;
    for (k, file) in files.iter().take(rows).enumerate() {
        // In truecolor, fade each successive row's background a little darker so
        // the list reads as a gradient down the box.
        let bg = if theme.truecolor {
            darken(theme.panel_bg, (k as f32 * 0.08).min(0.6))
        } else {
            theme.panel_bg
        };
        let path_style = Style::default().fg(theme.panel_fg).bg(bg);
        let size_style = Style::default().fg(color).bg(bg);

        let size = human_gb(file.size);
        // Reserve "<space>SIZE" on the right; the path fills the rest.
        let path_w = w.saturating_sub(size.chars().count() + 1).max(1);
        let path = ellipsize(&file.rel, path_w);
        let line = Line::from(vec![
            Span::styled(pad_right(&path, path_w), path_style),
            Span::styled(format!(" {size}"), size_style),
        ]);
        f.render_widget(
            Paragraph::new(line).style(Style::default().bg(bg)),
            Rect { y: area.y + k as u16, height: 1, ..area },
        );
    }
}

/// Render the whole treemap as a single graphics image of nested "pillow" boxes
/// — every directory a cushion-shaded box (each a distinct hue) subdivided into
/// recessed, semi-transparent sub-boxes for its largest files, with the names
/// baked into the pixels. Used whenever the terminal has a graphics protocol;
/// [`draw_box`] is the cell fallback.
fn render_treemap_graphics(
    f: &mut Frame,
    body: Rect,
    entries: &[DiskEntry],
    rects: &[Rect],
    selected: usize,
    theme: &Theme,
    g: &mut Gfx,
) {
    let (cw, ch) = g.cell();
    let (iw, ih) = g.px_size(body);
    let accent = raster::rgb(theme.panel_border_active);

    // The treemap image is expensive to build (per-pixel pillow shading + baked
    // labels) but stays identical across frames unless its inputs change. Compute
    // a cheap signature of those inputs so `draw_cached` rebuilds only on change —
    // otherwise a burst of redraws (e.g. after the terminal regains focus) would
    // rebuild the full-screen image on the main thread and peg a core for seconds.
    let sig = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (iw, ih, cw, ch, selected).hash(&mut h);
        raster::rgb(theme.panel_bg).hash(&mut h);
        accent.hash(&mut h);
        raster::rgb(theme.cursor_fg).hash(&mut h);
        for (entry, rect) in entries.iter().zip(rects) {
            entry.name.hash(&mut h);
            entry.size.hash(&mut h);
            (rect.x, rect.y, rect.width, rect.height).hash(&mut h);
            for fe in entry.files.iter().take(16) {
                fe.rel.hash(&mut h);
                fe.size.hash(&mut h);
            }
        }
        h.finish()
    };

    let build = move || build_treemap_image(body, cw, ch, entries, rects, selected, theme, accent);
    g.draw_cached(f, body, Slot::Treemap(0), sig, build);
}

/// Build the full treemap image: one nested "pillow" box per entry with baked
/// labels. Split out so [`render_treemap_graphics`] can skip it entirely when the
/// cached signature is unchanged.
#[allow(clippy::too_many_arguments)]
fn build_treemap_image(
    body: Rect,
    cw: u32,
    ch: u32,
    entries: &[DiskEntry],
    rects: &[Rect],
    selected: usize,
    theme: &Theme,
    accent: raster::Rgb,
) -> image::RgbaImage {
    let (iw, ih) = (body.width as u32 * cw, body.height as u32 * ch);
    let mut img = raster::canvas(iw, ih, raster::rgb(theme.panel_bg));

    for (i, (entry, rect)) in entries.iter().zip(rects).enumerate() {
        let ox = (rect.x.saturating_sub(body.x)) as u32 * cw;
        let oy = (rect.y.saturating_sub(body.y)) as u32 * ch;
        let (bw, bh) = (rect.width as u32 * cw, rect.height as u32 * ch);
        if bw < 3 || bh < 3 {
            continue;
        }
        // A distinct hue per box (golden-angle spread); the selected box keeps the
        // accent color and gets a bright border. Both stay stable frame-to-frame
        // so the encoded image is cached rather than re-transmitted.
        let fill = if i == selected {
            accent
        } else {
            raster::hsv(i as f64 * 137.508, 0.55, 0.72)
        };
        // Squarify the box's largest files into its interior, below the name/size
        // header (sized to the label scales so it doesn't overlap the sub-boxes).
        let (inner_w, inner_h) = (bw.saturating_sub(2) as f64, bh.saturating_sub(2) as f64);
        let (name_px, size_px) = label_px(bw, bh);
        let header_px = header_layout(bh, name_px, size_px).0 as f64;
        let region_h = inner_h - header_px;
        let mut frects = if entry.files.len() >= 2 && inner_w > 10.0 && region_h > 10.0 {
            let sizes: Vec<f64> = entry.files.iter().take(16).map(|fe| fe.size.max(1) as f64).collect();
            // `squarify` expects each area pre-scaled to fill the region (like
            // `treemap` does), so normalize the file sizes to the region's pixels —
            // otherwise the raw byte counts produce a degenerate, incomplete layout.
            let total: f64 = sizes.iter().sum::<f64>().max(1.0);
            let region_area = inner_w * region_h;
            let areas: Vec<f64> = sizes.iter().map(|s| s / total * region_area).collect();
            squarify(&areas, 0.0, 0.0, inner_w, region_h)
        } else {
            Vec::new()
        };
        for r in &mut frects {
            r.y += header_px; // shift the sub-treemap below the header
        }
        let subs: Vec<raster::SubBox> = frects
            .iter()
            .enumerate()
            .map(|(k, r)| raster::SubBox {
                x: r.x as f32,
                y: r.y as f32,
                w: r.w as f32,
                h: r.h as f32,
                color: raster::over((0, 0, 0), fill, 0.42 + 0.12 * (k % 3) as f64),
            })
            .collect();
        let border = (i == selected).then(|| raster::rgb(theme.cursor_fg));
        raster::pillow_into(&mut img, ox, oy, bw, bh, fill, &subs, border);
        // Bake the labels into the pixels so they survive every graphics protocol
        // (cell text drawn over an image is painted over by Kitty/Sixel).
        bake_labels(&mut img, (ox, oy, bw, bh), entry, fill, &frects, i == selected, theme);
    }
    img
}

/// Font pixel sizes (name, size) for a box's labels, chosen from its pixel size —
/// bigger boxes get bigger, more legible anti-aliased text.
fn label_px(bw: u32, bh: u32) -> (f32, f32) {
    let name = if bw >= 300 && bh >= 170 {
        30.0
    } else if bw >= 72 && bh >= 40 {
        20.0
    } else {
        14.0
    };
    let size = if bw >= 110 && bh >= 58 { 17.0 } else { 12.0 };
    (name, size)
}

/// The header height (pixels reserved above the sub-boxes) for a box, and whether
/// the size line fits under the name. Keeps the sub-treemap and [`bake_labels`]
/// in agreement about where the header ends.
fn header_layout(bh: u32, name_px: f32, size_px: f32) -> (u32, bool) {
    let name_h = raster::text_height(name_px);
    if bh < name_h + 12 {
        return (0, false);
    }
    let size_h = raster::text_height(size_px);
    let with_size = bh >= name_h + size_h + 24;
    let px = 3 + name_h + if with_size { 3 + size_h } else { 0 } + 4;
    (px, with_size)
}

/// Bake a box's labels into the treemap image: the directory name + size near the
/// top, and each file's base name on any sub-box big enough to hold it. Text is
/// baked as pixels (not cells) so it survives every graphics protocol; the font
/// scale grows with the box so labels stay readable.
fn bake_labels(
    img: &mut image::RgbaImage,
    (ox, oy, bw, bh): (u32, u32, u32, u32),
    entry: &DiskEntry,
    fill: raster::Rgb,
    frects: &[FRect],
    selected: bool,
    theme: &Theme,
) {
    let name_fg = if selected { raster::rgb(theme.cursor_fg) } else { (250, 250, 250) };
    let plate = raster::over(fill, (0, 0, 0), 0.6);
    let (name_px, size_px) = label_px(bw, bh);
    let (_, with_size) = header_layout(bh, name_px, size_px);
    // How many characters of `text` fit in `width_px` at font size `px`.
    let fit = |width: u32, px: f32| (width.saturating_sub(4) as f32 / raster::char_advance(px)).max(1.0) as usize;

    // Directory name, centered near the top.
    let name = ellipsize(&entry.name, fit(bw, name_px));
    let nx = ox as i32 + (bw as i32 - raster::text_width(&name, name_px) as i32) / 2;
    raster::draw_text(img, nx, oy as i32 + 3, &name, name_fg, Some(plate), name_px);
    // Size, centered under the name.
    if with_size {
        let size = ellipsize(&human_gb(entry.size), fit(bw, size_px));
        let sx = ox as i32 + (bw as i32 - raster::text_width(&size, size_px) as i32) / 2;
        let sy = oy as i32 + 3 + raster::text_height(name_px) as i32 + 3;
        raster::draw_text(img, sx, sy, &size, (230, 230, 230), Some(plate), size_px);
    }

    // File names on sub-boxes large enough to hold them (bigger where there's room).
    for (k, r) in frects.iter().enumerate() {
        let Some(file) = entry.files.get(k) else { break };
        let fpx = if r.w >= 120.0 && r.h >= 34.0 { 18.0 } else { 13.0 };
        if r.w < raster::char_advance(fpx) as f64 * 3.0 + 4.0 || r.h < raster::text_height(fpx) as f64 + 6.0 {
            continue;
        }
        let sub_rgb = raster::over((0, 0, 0), fill, 0.42 + 0.12 * (k % 3) as f64);
        let base = file.rel.rsplit(['/', '\\']).next().unwrap_or(&file.rel);
        let label = ellipsize(base, fit(r.w as u32, fpx));
        let sx = ox as i32 + 1 + r.x as i32 + 2;
        let sy = oy as i32 + 1 + r.y as i32 + 2;
        raster::draw_text(img, sx, sy, &label, (240, 240, 240), Some(sub_rgb), fpx);
    }
}

/// Scale an RGB color toward black by `t` (0 = unchanged, 1 = black).
fn darken(c: ratatui::style::Color, t: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    if let Color::Rgb(r, g, b) = c {
        let f = |x: u8| (x as f32 * (1.0 - t)).round().clamp(0.0, 255.0) as u8;
        Color::Rgb(f(r), f(g), f(b))
    } else {
        c
    }
}

// ---------------------------------------------------------------------------
// Squarified treemap (Bruls, Huizing & van Wijk)
// ---------------------------------------------------------------------------

struct FRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Lay out `entries` (already sorted largest-first) as a treemap filling `area`,
/// returning one integer rect per entry in the same order.
fn treemap(entries: &[DiskEntry], area: Rect) -> Vec<Rect> {
    let n = entries.len();
    if n == 0 || area.width == 0 || area.height == 0 {
        return vec![Rect { width: 0, height: 0, ..area }; n];
    }
    let total: u64 = entries.iter().map(|e| e.size).sum();
    // A floor so even small/empty directories stay visible and navigable.
    let base = (total / n as u64 / 12).max(1);
    let weights: Vec<f64> = entries.iter().map(|e| (e.size + base) as f64).collect();
    let wsum: f64 = weights.iter().sum();
    let total_area = area.width as f64 * area.height as f64;
    let areas: Vec<f64> = weights.iter().map(|w| w / wsum * total_area).collect();

    let frects = squarify(&areas, area.x as f64, area.y as f64, area.width as f64, area.height as f64);
    let x_max = (area.x + area.width) as f64;
    let y_max = (area.y + area.height) as f64;
    frects
        .iter()
        .map(|r| {
            let x0 = r.x.round().clamp(area.x as f64, x_max);
            let y0 = r.y.round().clamp(area.y as f64, y_max);
            let x1 = (r.x + r.w).round().clamp(x0, x_max);
            let y1 = (r.y + r.h).round().clamp(y0, y_max);
            Rect {
                x: x0 as u16,
                y: y0 as u16,
                width: (x1 - x0) as u16,
                height: (y1 - y0) as u16,
            }
        })
        .collect()
}

fn squarify(areas: &[f64], x: f64, y: f64, w: f64, h: f64) -> Vec<FRect> {
    let mut out: Vec<FRect> = Vec::with_capacity(areas.len());
    let mut rect = FRect { x, y, w, h };
    let mut row: Vec<f64> = Vec::new();
    let mut i = 0;
    while i < areas.len() {
        let length = rect.w.min(rect.h);
        if length <= 0.0 {
            // No space left; emit zero rects for the remainder.
            for _ in i..areas.len() {
                out.push(FRect { x: rect.x, y: rect.y, w: 0.0, h: 0.0 });
            }
            return out;
        }
        let a = areas[i];
        row.push(a);
        let with = worst(&row, length);
        row.pop();
        let without = if row.is_empty() { f64::MAX } else { worst(&row, length) };
        if row.is_empty() || without >= with {
            row.push(a);
            i += 1;
        } else {
            layout_row(&row, &mut rect, &mut out);
            row.clear();
        }
    }
    if !row.is_empty() {
        layout_row(&row, &mut rect, &mut out);
    }
    out
}

/// Worst (largest) aspect ratio in a row laid along side `length`.
fn worst(row: &[f64], length: f64) -> f64 {
    let sum: f64 = row.iter().sum();
    if sum <= 0.0 || length <= 0.0 {
        return f64::MAX;
    }
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let l2 = length * length;
    let s2 = sum * sum;
    f64::max(l2 * max / s2, s2 / (l2 * min))
}

fn layout_row(row: &[f64], rect: &mut FRect, out: &mut Vec<FRect>) {
    let sum: f64 = row.iter().sum();
    if sum <= 0.0 {
        for _ in row {
            out.push(FRect { x: rect.x, y: rect.y, w: 0.0, h: 0.0 });
        }
        return;
    }
    if rect.w >= rect.h {
        // Lay the row as a column down the left edge.
        let col_w = sum / rect.h;
        let mut yy = rect.y;
        for &a in row {
            let cell_h = a / col_w;
            out.push(FRect { x: rect.x, y: yy, w: col_w, h: cell_h });
            yy += cell_h;
        }
        rect.x += col_w;
        rect.w -= col_w;
    } else {
        // Lay the row across the top edge.
        let row_h = sum / rect.w;
        let mut xx = rect.x;
        for &a in row {
            let cell_w = a / row_h;
            out.push(FRect { x: xx, y: rect.y, w: cell_w, h: row_h });
            xx += cell_w;
        }
        rect.y += row_h;
        rect.h -= row_h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, size: u64) -> DiskEntry {
        DiskEntry { name: name.into(), size, files: vec![] }
    }

    #[test]
    fn scanning_shows_a_progress_bar() {
        use crate::disk::DiskView;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut dv = DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = true;
        dv.scan_done = 3;
        dv.scan_total = 12;
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(80, 20)).unwrap();
        t.draw(|f| render(f, f.area(), &mut dv, &theme, None)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("3 / 12 directories"), "progress label");
        assert!(s.contains('█') && s.contains('░'), "determinate bar drawn");
        assert!(s.contains("25%"), "percentage shown");
    }

    #[test]
    fn big_box_lists_its_largest_files() {
        use crate::disk::{DiskView, FileEntry};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut dv = DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = false;
        // A single entry fills the whole treemap → its box is large.
        dv.entries = vec![DiskEntry {
            name: "project".into(),
            size: 9_000_000,
            files: vec![
                FileEntry { rel: "target/huge.bin".into(), size: 5_000_000 },
                FileEntry { rel: "assets/movie.mp4".into(), size: 3_000_000 },
            ],
        }];
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| render(f, f.area(), &mut dv, &theme, None)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("target/huge.bin"), "largest file path shown");
        assert!(s.contains("assets/movie.mp4"), "second file path shown");
        assert!(s.contains("4.8 MB"), "file size shown");
    }

    #[test]
    fn big_box_pillow_graphics_path_renders_without_panic() {
        use crate::disk::{DiskView, FileEntry};
        use crate::ui::graphics::Gfx;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut dv = DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = false;
        dv.entries = vec![DiskEntry {
            name: "project".into(),
            size: 9_000_000,
            files: vec![
                FileEntry { rel: "target/huge.bin".into(), size: 5_000_000 },
                FileEntry { rel: "assets/movie.mp4".into(), size: 3_000_000 },
                FileEntry { rel: "docs/manual.pdf".into(), size: 1_000_000 },
            ],
        }];
        let theme = crate::ui::theme::Theme::mc();
        let mut gfx = Gfx::test_halfblocks();
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        // Exercises draw_box_pillow: squarify sub-layout + pillow_box + g.draw.
        t.draw(|f| render(f, f.area(), &mut dv, &theme, Some(&mut gfx))).unwrap();
        // The whole treemap is one graphics image (labels are baked into pixels,
        // so they render as image cells, not readable text). Assert it painted.
        let b = t.backend().buffer();
        let image_cells = (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| matches!(b[(x, y)].symbol(), "\u{2580}" | "\u{2584}"))
            .count();
        assert!(image_cells > 100, "the graphical pillow treemap should paint many image cells");
    }

    #[test]
    fn renders_treemap_with_boxes_and_footer() {
        use crate::disk::DiskView;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut dv = DiskView::new(std::path::PathBuf::from("/tmp"));
        dv.scanning = false;
        dv.entries = vec![e("big", 9_000_000), e("small", 100_000)];
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| render(f, f.area(), &mut dv, &theme, None)).unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("Disk Explorer"), "title");
        assert!(s.contains("big"), "largest box labeled");
        assert!(s.contains("click") && s.contains("Esc close"), "footer guidance (incl. mouse)");
        assert!(s.contains("▶") && s.contains("of total"), "selected-box header");
        assert_eq!(dv.rects.len(), 2, "geometry recorded for navigation");
    }

    #[test]
    fn treemap_covers_area_and_orders_by_input() {
        let entries = vec![e("a", 800), e("b", 150), e("c", 50)];
        let area = Rect { x: 0, y: 0, width: 40, height: 20 };
        let rects = treemap(&entries, area);
        assert_eq!(rects.len(), 3);
        // Largest entry gets the largest box.
        let areas: Vec<u32> = rects.iter().map(|r| r.width as u32 * r.height as u32).collect();
        assert!(areas[0] >= areas[1] && areas[1] >= areas[2], "areas: {areas:?}");
        // Every box lies within the area.
        for r in &rects {
            assert!(r.x + r.width <= area.x + area.width);
            assert!(r.y + r.height <= area.y + area.height);
        }
    }
}
