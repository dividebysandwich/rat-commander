//! Rendering for the internal viewer (text / hex).

use super::{ViewMode, ViewerState};
use crate::ui::dialog::widgets::{centered, dialog_block, draw_shadow};
use crate::ui::theme::Theme;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

pub fn render(
    f: &mut Frame,
    area: Rect,
    v: &mut ViewerState,
    theme: &Theme,
    gfx: Option<&mut crate::ui::graphics::Gfx>,
) {
    if area.height < 3 {
        return;
    }
    // A document draws as its own viewer; the footer is this one's, so F4 and
    // F8 name the way back to the bytes, and its rects are this one's too, so
    // a click on the bar comes here first.
    if let Some(part) = v.doc_part_mut() {
        render(f, area, part, theme, gfx);
        let (content, footer) = (part.content_area, part.footer_area);
        v.content_area = content;
        v.footer_area = footer;
        render_footer(f, footer, v, theme);
        return;
    }
    let header = Rect { height: 1, ..area };
    let content = Rect { y: area.y + 1, height: area.height - 2, ..area };
    let footer = Rect { y: area.y + area.height - 1, height: 1, ..area };
    // The blame column takes the left of the content, and the text the rest —
    // which is the width wrapping and scrolling have to measure against.
    let gutter_w = if v.active_blame().is_some() { blame_gutter_width(content.width) } else { 0 };
    let text_area = Rect { x: content.x + gutter_w, width: content.width - gutter_w, ..content };
    // One page for the background gradient, however a full-width cursor bar
    // cuts across it.
    crate::ui::gradient::mark_surface(crate::ui::theme::GradRole::PanelBg, content);

    v.view_rows = content.height as usize;
    v.view_cols = text_area.width as usize;
    v.content_area = content;
    v.footer_area = footer;

    // Make sure the lines about to be drawn (plus one past, so the last line's
    // extent is known) are indexed — the rest of the file stays unscanned — and
    // pull the view back if a resize or jump left it beyond the last full
    // screen, so blank space never shows below the end of the file.
    // A model, like an image, replaces the document entirely, so there is no
    // page to index or clamp for it either. Nor does the table, which pages
    // by record rather than by line — and is built before the header, which
    // reports its cursor.
    if v.table_active() {
        v.ensure_table();
    }
    if v.active_image().is_none()
        && v.active_model().is_none()
        && v.active_audio().is_none()
        && v.active_certs().is_none()
    {
        if v.mode == ViewMode::Text && !v.table_active() {
            v.extend_to_line(v.top + v.view_rows);
        }
        v.top = v.top.min(v.max_top());
    }

    render_header(f, header, v, theme);
    // An audio file shows its spectrogram or waveform with transport controls
    // beneath; F8 toggles to the raw text/hex.
    if let Some(a) = v.active_audio() {
        f.render_widget(Clear, content);
        crate::audio::widget::render(
            f,
            content,
            a,
            theme,
            gfx,
            crate::ui::graphics::Slot::ViewerAudio,
        );
        render_footer(f, footer, v, theme);
        return;
    }
    // A certificate or key file shows what it holds; F8 toggles to the raw text.
    if v.active_certs().is_some() {
        super::certs::render(f, content, v, theme);
        render_footer(f, footer, v, theme);
        return;
    }
    // An image file shows the decoded image fullscreen (pixel graphics where
    // available, else half-block cell art); F8 toggles to the raw text/hex.
    if v.active_image().is_some() {
        render_image(f, content, v, theme, gfx);
        render_footer(f, footer, v, theme);
        return;
    }
    // A model file orbits a rasterized mesh in the same place, on the same three
    // presentation tiers; F8 toggles to the raw text/hex.
    if v.active_model().is_some() {
        render_model(f, content, v, theme, gfx);
        render_footer(f, footer, v, theme);
        return;
    }
    match v.mode {
        ViewMode::Binary => render_binary(f, content, v, theme),
        ViewMode::Map => render_map(f, content, v, theme, gfx),
        ViewMode::Hex => render_hex(f, content, v, theme),
        // Markdown files render the approximation by default; F8 shows the raw,
        // syntax-highlighted source.
        ViewMode::Text if v.table_active() => super::table::render(f, content, v, theme),
        ViewMode::Text if v.markdown_active() => render_markdown(f, content, v, theme),
        ViewMode::Text => {
            let rows = render_text(f, text_area, v, theme);
            if gutter_w > 0 {
                render_blame_gutter(f, Rect { width: gutter_w, ..content }, v, &rows, theme);
            }
        }
    }
    render_footer(f, footer, v, theme);
    // The F6 document outline draws over the content as a modal overlay.
    if v.outline_open {
        render_outline(f, content, v, theme);
    }
}

/// Draw the F6 document-outline navigator: a centered, bordered list of the
/// document's headings, indented by nesting level and colored per level (the
/// selected entry highlighted). Records its interior rect on the viewer for
/// mouse hit-testing and keeps the selection scrolled into view.
fn render_outline(f: &mut Frame, area: Rect, v: &mut ViewerState, theme: &Theme) {
    let items = v.outline.clone().unwrap_or_default();
    let title = crate::l10n::trd("Outline");
    let empty = crate::l10n::trd("(no headings)");

    // Size the box to the longest entry (indent + text), clamped to what the
    // content area allows (guarding against narrow terminals and huge headings so
    // the arithmetic below can never overflow or clamp with an inverted range).
    let label_w = items
        .iter()
        .map(|it| it.level.saturating_sub(1) * 2 + it.text.chars().count())
        .max()
        .unwrap_or(0)
        .max(empty.chars().count())
        .max(title.chars().count() + 2);
    let avail_w = area.width.saturating_sub(2); // interior width inside the borders
    let want_w = label_w.min(u16::MAX as usize) as u16;
    let inner_w = want_w.saturating_add(1).clamp(1, avail_w.max(1));
    let box_w = inner_w.saturating_add(2).min(area.width.max(2));
    let avail_h = area.height.saturating_sub(2);
    let want_h = items.len().max(1).min(u16::MAX as usize) as u16;
    let inner_h = want_h.clamp(1, avail_h.max(1));
    let box_h = inner_h.saturating_add(2).min(area.height.max(2));
    let rect = centered(area, box_w, box_h);

    draw_shadow(f, rect, theme);
    f.render_widget(Clear, rect);
    let block = dialog_block(&title, theme);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    v.outline_area = inner;
    let rows = inner.height as usize;

    // Keep the selection within the visible window, then clamp the scroll offset.
    v.outline_top = crate::util::scroll::scroll_to_visible(v.outline_top, v.outline_sel, rows);
    v.outline_top = v.outline_top.min(items.len().saturating_sub(rows));

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(rows.max(1));
    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            pad_right(&empty, width),
            Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg).add_modifier(Modifier::ITALIC),
        )));
    } else {
        for (i, it) in items.iter().enumerate().skip(v.outline_top).take(rows) {
            let indent = "  ".repeat(it.level.saturating_sub(1));
            let text = pad_right(&format!("{indent}{}", it.text), width);
            let style = if i == v.outline_sel {
                theme.dialog_selection
            } else {
                // The per-level heading colors are tuned for the panel background;
                // keep them legible on themes whose dialog background is bright.
                let fg = crate::ui::theme::readable_on(
                    super::markdown::heading_color(it.level, theme),
                    theme.dialog_bg,
                );
                Style::default().fg(fg).bg(theme.dialog_bg)
            };
            lines.push(Line::from(Span::styled(text, style)));
        }
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)), inner);
}

/// Build a styled line from `chars`, coloring each by `fg[base + j]` (falling
/// back to `default`), merging adjacent same-color runs.
fn build_spans(
    chars: &[char],
    base: usize,
    fg: &[Color],
    default: Color,
    bg: Color,
) -> Line<'static> {
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

/// Like [`build_spans`] but carrying a full per-character [`Style`] (so Markdown
/// bold/italic/underline modifiers survive), merging adjacent same-style runs.
fn build_styled(chars: &[char], base: usize, styles: &[Style], default: Style) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let mut run = String::new();
    let mut cur = default;
    for (j, &ch) in chars.iter().enumerate() {
        let st = styles.get(base + j).copied().unwrap_or(default);
        if st != cur && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), cur));
        }
        cur = st;
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, cur));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), default));
    }
    Line::from(spans)
}

/// Colour for one map cell.
///
/// The density ramp runs blue → cyan → green → yellow → red, which is ordered by
/// brightness as well as by hue: that is what keeps it readable after the
/// half-block and ASCII fallbacks throw the hue away and leave only luminance.
fn map_color(
    c: &crate::viewer::fingerprint::Cell,
    by_class: bool,
) -> crate::ui::graphics::raster::Rgb {
    use crate::viewer::fingerprint::ByteClass;
    if by_class {
        return match c.class {
            ByteClass::Zero => (32, 36, 48),
            ByteClass::Ascii => (96, 200, 120),
            ByteClass::High => (220, 150, 70),
            ByteClass::Mixed => (120, 130, 190),
        };
    }
    let t = c.density.clamp(0.0, 1.0) as f64;
    crate::ui::graphics::raster::hsv(240.0 * (1.0 - t), 0.85, 0.25 + 0.70 * t)
}

/// Draw the byte map: the whole file as one picture, with a cursor.
fn render_map(
    f: &mut Frame,
    area: Rect,
    v: &mut ViewerState,
    theme: &Theme,
    gfx: Option<&mut crate::ui::graphics::Gfx>,
) {
    let Some(fp) = v.active_map() else {
        return;
    };
    let bg = crate::ui::graphics::raster::rgb(theme.panel_bg);
    let n = fp.cells.len().max(1);
    // Lay the cells out row-major, so a row is a contiguous run of the file and
    // the picture reads in the same order as the hex dump it jumps into. The
    // column count is chosen to make cells as square as the area allows.
    let aspect = (area.width as f32).max(1.0) / (area.height as f32 * 2.0).max(1.0);
    let cols = ((n as f32 * aspect).sqrt().round() as usize).clamp(1, n);
    let rows = n.div_ceil(cols);
    v.map_cols = cols;
    let cur = v.map_cursor();
    let by_class = v.map_by_class();
    let fp = v.active_map().expect("still in map mode");

    let build = |w: u32, h: u32| {
        let mut img = crate::ui::graphics::raster::canvas(w, h, bg);
        let cw = (w as f32 / cols as f32).max(1.0);
        let ch = (h as f32 / rows as f32).max(1.0);
        for (i, cell) in fp.cells.iter().enumerate() {
            let (cx, cy) = (i % cols, i / cols);
            let (x0, y0) = ((cx as f32 * cw) as u32, (cy as f32 * ch) as u32);
            let (x1, y1) = (((cx + 1) as f32 * cw) as u32, ((cy + 1) as f32 * ch) as u32);
            let c = map_color(cell, by_class);
            crate::ui::graphics::raster::fill_rect(
                &mut img,
                x0,
                y0,
                (x1.saturating_sub(x0)).max(1),
                (y1.saturating_sub(y0)).max(1),
                c,
            );
            if i == cur {
                // Lit from within rather than outlined: a one-pixel ring would
                // be invisible once the cell art downsamples the raster.
                crate::ui::graphics::raster::fill_rect(
                    &mut img,
                    x0,
                    y0,
                    (x1.saturating_sub(x0)).max(1),
                    (y1.saturating_sub(y0)).max(1),
                    (255, 255, 255),
                );
            }
        }
        img
    };

    f.render_widget(ratatui::widgets::Clear, area);
    match gfx {
        Some(g) if g.available() => {
            let (cwp, chp) = g.cell();
            let (mut w, mut h) = (area.width as u32 * cwp, area.height as u32 * chp);
            let long = w.max(h);
            if long > MODEL_MAX_PX {
                w = w * MODEL_MAX_PX / long;
                h = h * MODEL_MAX_PX / long;
            }
            let sig = {
                use std::hash::{Hash, Hasher};
                let mut hs = std::collections::hash_map::DefaultHasher::new();
                fp.len.hash(&mut hs);
                n.hash(&mut hs);
                cur.hash(&mut hs);
                by_class.hash(&mut hs);
                (w, h).hash(&mut hs);
                hs.finish()
            };
            g.draw_cached(f, area, crate::ui::graphics::Slot::ViewerFingerprint, sig, || {
                build(w.max(1), h.max(1))
            });
        }
        _ => {
            let img = build(area.width as u32, area.height as u32 * 2);
            if theme.truecolor {
                crate::util::img::render_halfblocks(f, area, &img, theme.panel_bg);
            } else {
                crate::util::img::render_ascii_ramp(f, area, &img, theme);
            }
        }
    }
}

/// Binary mode's header: the file, what it is, where the highlight is in the
/// list on screen, and the filter narrowing the lists.
fn render_binary_header(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    let what = match v.active_binary() {
        None => crate::l10n::trd("Analyzing…"),
        Some(view) => {
            let len = view.len(view.tab);
            let at = if len == 0 { 0 } else { view.selected() + 1 };
            let filter = view
                .filter_term()
                .map(|t| format!("  {}: \"{t}\"", crate::l10n::trd("Filter")))
                .unwrap_or_default();
            format!(
                "{}  {} {at}/{len}{filter}",
                view.bin.summary,
                crate::l10n::trd(view.tab.label())
            )
        }
    };
    let text = format!(
        " {}: {}  [{}]  {what}",
        crate::l10n::trd("View"),
        ellipsize(&v.name, 24),
        crate::l10n::trd("Binary"),
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&ellipsize(&text, area.width as usize), area.width as usize),
            theme.menubar.add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

/// One cell of a Binary-mode row: its text and how it is drawn.
type Cell = (String, Style);

/// Draw Binary mode: the strip of tabs, the column headings of the one that is
/// up, and its rows — fixed-width columns first, then an open-ended one (a
/// name, a string) that takes the rest of the width and scrolls sideways.
fn render_binary(f: &mut Frame, area: Rect, v: &mut ViewerState, theme: &Theme) {
    use super::binary::{Tab, hex};
    let base = Style::default().fg(theme.text_fg).bg(theme.panel_bg);
    f.render_widget(ratatui::widgets::Block::default().style(base), area);
    let Some(view) = v.active_binary_mut() else {
        // The analysis has not landed yet.
        let msg = crate::l10n::trd("Analyzing…");
        let row = Rect { y: area.y + area.height / 2, height: 1, ..area };
        f.render_widget(
            Paragraph::new(msg).style(base).alignment(ratatui::layout::Alignment::Center),
            row,
        );
        return;
    };
    if area.height < 3 || area.width < 8 {
        return;
    }
    let strip = Rect { height: 1, ..area };
    // A column of margin on the left, as a panel's border would give.
    let heading = Rect { x: area.x + 1, y: area.y + 1, width: area.width - 1, height: 1 };
    let list = Rect { y: area.y + 2, height: area.height - 2, ..heading };
    view.page = list.height as usize;
    view.list_area = list;
    view.strip_row = strip.y;
    render_binary_tabs(f, strip, view, theme);

    let width = list.width as usize;
    let top = view.scroll_into_view(list.height as usize);
    let len = view.len(view.tab);
    let b = &view.bin;
    let number = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let dim = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let aw = if b.is_64 { 16 } else { 8 };
    // Offsets are as wide as the largest one in the file needs, eight at least.
    let file_end = b.sections.iter().filter_map(|s| s.offset.map(|o| o + s.size)).max();
    let ow = file_end.unwrap_or(0).max(b.strings.last().map_or(0, |s| s.offset)).max(1);
    let ow = (format!("{ow:x}").len()).max(8);
    let addr = |a: u64| (hex(a, b.is_64), number);
    let off = |o: Option<u64>| (o.map_or_else(|| "-".into(), |o| format!("{o:0ow$x}")), number);
    let longest = |it: &mut dyn Iterator<Item = usize>, cap: usize| it.max().unwrap_or(0).min(cap);

    // Each tab's columns: (heading, width, right-aligned) for the fixed ones,
    // then the open-ended heading.
    let (fixed, open): (Vec<(&str, usize, bool)>, &str) = match view.tab {
        Tab::Info => {
            let w = longest(
                &mut b.facts.iter().map(|x| {
                    unicode_width::UnicodeWidthStr::width(crate::l10n::trd(x.label).as_str())
                }),
                30,
            );
            (vec![("", w, true)], "")
        }
        Tab::Sections => (
            vec![
                ("Address", aw, false),
                ("Offset", ow, false),
                ("Size", 10, true),
                ("Type", 6, false),
            ],
            "Name",
        ),
        Tab::Libraries => (vec![], "Name"),
        Tab::Imports => {
            let w = longest(&mut b.imports.iter().map(|s| s.library.chars().count()), 32);
            (if w == 0 { vec![] } else { vec![("Library", w.max(7), false)] }, "Name")
        }
        Tab::Exports => (vec![("Address", aw, false)], "Name"),
        Tab::Functions => (vec![("Address", aw, false), ("Size", 8, true)], "Name"),
        Tab::Strings => {
            let w = longest(&mut b.sections.iter().map(|s| s.name.chars().count()), 20);
            (vec![("Offset", ow, false), ("Section", w.max(7), false), ("", 3, false)], "Text")
        }
    };

    // The Info tab reads as `label : value`, the way the Details view lays out
    // a file's metadata; the lists space their columns further apart.
    let sep = if view.tab == Tab::Info { " " } else { "  " };
    // The column headings.
    let head = Style::default().fg(theme.header_fg).bg(theme.panel_bg).add_modifier(Modifier::BOLD);
    let mut cells: Vec<Cell> = Vec::new();
    for (title, w, right) in &fixed {
        let t = if title.is_empty() { String::new() } else { crate::l10n::trd(title) };
        cells.push((fit(&t, *w, *right), head));
        cells.push((sep.into(), head));
    }
    let open_title = if open.is_empty() { String::new() } else { crate::l10n::trd(open) };
    cells.push((open_title, head));
    f.render_widget(Paragraph::new(binary_line(cells, width, head)), heading);

    let mut lines = Vec::with_capacity(list.height as usize);
    if len == 0 {
        let none = dim.add_modifier(Modifier::ITALIC);
        lines.push(binary_line(vec![(crate::l10n::trd("(none)"), none)], width, base));
    }
    let h = view.h_offset;
    let scrolled = |s: &str| -> String {
        s.chars().skip(h).map(|c| if c.is_control() { ' ' } else { c }).collect()
    };
    for i in top..(top + list.height as usize).min(len) {
        let Some(idx) = view.row(view.tab, i) else { break };
        // The fixed cells, then the open-ended one and anything that follows it.
        let (fixed_cells, tail): (Vec<Cell>, Vec<Cell>) = match view.tab {
            Tab::Info => {
                let x = &b.facts[idx];
                let value = if x.translate { crate::l10n::trd(&x.value) } else { x.value.clone() };
                let label = Style::default().fg(theme.header_fg).bg(theme.panel_bg);
                (
                    vec![(crate::l10n::trd(x.label), label)],
                    vec![(": ".into(), label), (scrolled(&value), base)],
                )
            }
            Tab::Sections => {
                let s = &b.sections[idx];
                (
                    vec![
                        addr(s.address),
                        off(s.offset),
                        (s.size.to_string(), number),
                        (s.kind.into(), dim),
                    ],
                    vec![(scrolled(&s.name), base)],
                )
            }
            Tab::Libraries => {
                let l = &b.libraries[idx];
                (vec![], vec![(scrolled(&l.name), base), (format!("  {}", l.note), dim)])
            }
            Tab::Imports => {
                let s = &b.imports[idx];
                (vec![(s.library.clone(), dim)], vec![(scrolled(&view.name(s)), base)])
            }
            Tab::Exports => {
                let s = &b.exports[idx];
                let shown = if s.address == 0 { ("-".into(), number) } else { addr(s.address) };
                (
                    vec![shown],
                    vec![(scrolled(&view.name(s)), base), (format!("  {}", s.library), dim)],
                )
            }
            Tab::Functions => {
                let s = &b.functions[idx];
                let size = if s.size == 0 { String::new() } else { s.size.to_string() };
                // A function no symbol names is drawn dimmed, as the stand-in it is.
                let style = if s.name.is_empty() { dim } else { base };
                (vec![addr(s.address), (size, number)], vec![(scrolled(&view.name(s)), style)])
            }
            Tab::Strings => {
                let s = &b.strings[idx];
                let section = b.section_at(s.offset).unwrap_or("").to_string();
                let enc = if s.wide { "u16" } else { "" };
                (
                    vec![off(Some(s.offset)), (section, dim), (enc.into(), dim)],
                    vec![(scrolled(&s.text), base)],
                )
            }
        };
        let mut cells: Vec<Cell> = Vec::new();
        for ((text, style), (_, w, right)) in fixed_cells.into_iter().zip(&fixed) {
            cells.push((fit(&text, *w, *right), style));
            cells.push((sep.into(), base));
        }
        cells.extend(tail);
        let selected = i == view.selected();
        let cells = if selected {
            cells.into_iter().map(|(t, _)| (t, theme.cursor)).collect()
        } else {
            cells
        };
        lines.push(binary_line(cells, width, if selected { theme.cursor } else { base }));
    }
    f.render_widget(Paragraph::new(lines).style(base), list);
}

/// The strip of tab titles above Binary mode's list, each with its row count —
/// a `+` after one that stopped at the row cap. On a screen too narrow for all
/// of it, only the tab that is up keeps its count, and the strip starts late
/// enough that that tab is always on it.
fn render_binary_tabs(
    f: &mut Frame,
    area: Rect,
    view: &mut super::binary::view::BinaryView,
    theme: &Theme,
) {
    use super::binary::Tab;
    use unicode_width::UnicodeWidthStr;
    let normal = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    let count_style = Style::default().fg(theme.panel_border).bg(theme.panel_bg);
    let titles: Vec<(String, String)> = Tab::ALL
        .iter()
        .map(|&t| {
            let more = if view.bin.capped[t.index()] { "+" } else { "" };
            let count =
                if t == Tab::Info { String::new() } else { format!("{}{more}", view.len(t)) };
            (crate::l10n::trd(t.label()), count)
        })
        .collect();
    let width = area.width as usize;
    let measure = |compact: bool| -> Vec<usize> {
        titles
            .iter()
            .zip(Tab::ALL)
            .map(|((title, count), t)| {
                let shown = if compact && t != view.tab { 0 } else { count.width() + 1 };
                title.width() + 2 + if count.is_empty() { 0 } else { shown } + 1
            })
            .collect()
    };
    let mut compact = false;
    let mut widths = measure(false);
    if widths.iter().sum::<usize>() > width {
        compact = true;
        widths = measure(true);
    }
    let active = view.tab.index();
    let mut first = 0;
    while first < active && widths[first..=active].iter().sum::<usize>() > width {
        first += 1;
    }

    let mut spans = Vec::new();
    let mut hits = Vec::new();
    let mut x = area.x;
    for (i, (title, count)) in titles.iter().enumerate().skip(first) {
        let tab = Tab::ALL[i];
        let style = if tab == view.tab { theme.cursor } else { normal };
        let start = x;
        spans.push(Span::styled(format!(" {title}"), style));
        if !count.is_empty() && !(compact && tab != view.tab) {
            let cs = if tab == view.tab { theme.cursor } else { count_style };
            spans.push(Span::styled(format!(" {count}"), cs));
        }
        spans.push(Span::styled(" ", style));
        spans.push(Span::styled(" ", normal));
        x = x.saturating_add(widths[i] as u16);
        hits.push((start, x.saturating_sub(1), tab));
    }
    view.tab_hits = hits;
    f.render_widget(Paragraph::new(Line::from(spans)).style(normal), area);
}

/// `s` in exactly `w` columns, padded on the left or the right, cut with `~`.
fn fit(s: &str, w: usize, right: bool) -> String {
    let s = ellipsize(s, w);
    if right { crate::util::text::pad_left(&s, w) } else { pad_right(&s, w) }
}

/// A row of cells, cut at `width` columns and padded out to it in `fill`, so a
/// highlighted row is a bar across the whole list.
fn binary_line(cells: Vec<Cell>, width: usize, fill: Style) -> Line<'static> {
    let mut used = 0;
    let mut spans = Vec::with_capacity(cells.len() + 1);
    for (text, style) in cells {
        if used >= width {
            break;
        }
        let (t, w) = crate::util::text::truncate_width(&text, width - used);
        used += w;
        spans.push(Span::styled(t, style));
    }
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), fill));
    }
    Line::from(spans)
}

/// Largest model raster built per frame.
///
/// The same bound, and the same reasoning, as the 3D panel's: an orbit rebuilds
/// this on every key press, and an uncapped full-screen raster would re-encode
/// several megapixels each time. The image is scaled back up to the cell area.
const MODEL_MAX_PX: u32 = 1280;

/// Draw the mesh fullscreen — pixel graphics where available, else half-block
/// cell art, else an ASCII luminance ramp.
fn render_model(
    f: &mut Frame,
    area: Rect,
    v: &ViewerState,
    theme: &Theme,
    gfx: Option<&mut crate::ui::graphics::Gfx>,
) {
    let Some(m) = v.active_model() else {
        return;
    };
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(theme.panel_bg)),
        area,
    );
    let bg = crate::ui::graphics::raster::rgb(theme.panel_bg);
    // The pale solid the 3D landscape stands its platforms on. A near-white
    // surface is what makes a shaded form read; a saturated one loses the
    // gradient that describes the curve.
    let base = crate::space3d::ScenePalette::from_theme(theme).platform;
    let draw = |w: u32, h: u32| {
        crate::space3d::raster3d::render_mesh(
            w,
            h,
            &m.mesh.tris,
            m.mesh.min.y,
            m.cam.eye(),
            m.cam.target,
            bg,
            base,
        )
    };
    match gfx {
        Some(g) if g.available() => {
            let (cw, ch) = g.cell();
            let (mut w, mut h) = (area.width as u32 * cw, area.height as u32 * ch);
            let long = w.max(h);
            if long > MODEL_MAX_PX {
                w = w * MODEL_MAX_PX / long;
                h = h * MODEL_MAX_PX / long;
            }
            // Built inside the closure so an unmoved camera costs nothing: the
            // cache compares the signature first and only then rasterizes.
            g.draw_cached(f, area, crate::ui::graphics::Slot::ViewerModel, m.sig(), || {
                draw(w.max(1), h.max(1))
            });
        }
        // One pixel per half-cell, so the rasterizer's square-pixel assumption
        // holds and the same focal length is correct here too.
        _ => {
            let img = draw(area.width as u32, area.height as u32 * 2);
            if theme.truecolor {
                crate::util::img::render_halfblocks(f, area, &img, theme.panel_bg);
            } else {
                crate::util::img::render_ascii_ramp(f, area, &img, theme);
            }
        }
    }
}

/// Draw the decoded image fullscreen — pixel graphics where available, else
/// centred half-block cell art.
fn render_image(
    f: &mut Frame,
    area: Rect,
    v: &ViewerState,
    theme: &Theme,
    gfx: Option<&mut crate::ui::graphics::Gfx>,
) {
    let Some(iv) = v.active_image() else {
        return;
    };
    // Clear the content region so the letterbox is clean under either renderer.
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(
        ratatui::widgets::Block::default().style(Style::default().bg(theme.panel_bg)),
        area,
    );
    match gfx {
        Some(g) if g.available() => {
            let target =
                crate::util::img::center_rect(area, iv.img.width(), iv.img.height(), g.cell());
            let (sig, img) = (iv.sig, &iv.img);
            g.draw_cached(f, target, crate::ui::graphics::Slot::ViewerImage, sig, || img.clone());
        }
        _ => crate::util::img::render_halfblocks(f, area, &iv.img, theme.panel_bg),
    }
}

fn render_header(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    if v.active_certs().is_some() {
        return super::certs::render_header(f, area, v, theme);
    }
    // In audio mode the header names the file, its format, which picture is
    // up, and where playback is.
    if let Some(a) = v.active_audio() {
        let display = crate::l10n::trd(a.display().label());
        let state = if a.playing() { "▶" } else { "❚❚" };
        let time = format!(
            "{state} {} / {}",
            crate::audio::format_time(a.position()),
            crate::audio::format_time(a.duration())
        );
        let summary = a.info.summary();
        let fixed = 12 + summary.chars().count() + display.chars().count() + time.chars().count();
        let mut text = format!(
            " {}: {}  [{summary}]  [{display}]  {time}",
            crate::l10n::trd("View"),
            ellipsize(&v.name, (area.width as usize).saturating_sub(fixed).max(8)),
        );
        // The track's tags, when the file has them and there is room.
        let tags: Vec<&str> =
            [a.info.artist.as_deref(), a.info.title.as_deref()].into_iter().flatten().collect();
        if !tags.is_empty() {
            text.push_str("  ");
            text.push_str(&tags.join(" – "));
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&ellipsize(&text, area.width as usize), area.width as usize),
                theme.menubar.add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    // In model mode the header names the file, the format read, and how many
    // triangles it turned out to hold — the size that actually matters here.
    if let Some(m) = v.active_model() {
        let text = format!(
            " {}: {}  [{} {} {}]",
            crate::l10n::trd("View"),
            ellipsize(&v.name, area.width.saturating_sub(28) as usize),
            m.mesh.format,
            m.mesh.tris.len(),
            crate::l10n::trd("triangles"),
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&text, area.width as usize),
                theme.menubar.add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    // In image mode the header names the file and its original pixel dimensions.
    if let Some(iv) = v.active_image() {
        let (w, h) = iv.orig;
        let text = format!(
            " {}: {}  [{} {w}×{h}]",
            crate::l10n::trd("View"),
            ellipsize(&v.name, area.width.saturating_sub(24) as usize),
            crate::l10n::trd("Image"),
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&text, area.width as usize),
                theme.menubar.add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    // The map has a header of its own: a mode/rows readout means nothing for a
    // view with no rows, where what matters is the cell under the cursor.
    if let Some(fp) = v.active_map() {
        let cell = fp.cells.get(v.map_cursor());
        let text = format!(
            " {}: {}  [{}]  {} 0x{:X}  {} {:.2}",
            crate::l10n::trd("View"),
            ellipsize(&v.name, area.width.saturating_sub(44) as usize),
            crate::l10n::trd("Map"),
            crate::l10n::trd("at"),
            cell.map_or(0, |c| c.start),
            crate::l10n::trd("entropy"),
            cell.map_or(0.0, |c| c.entropy),
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&text, area.width as usize),
                theme.menubar.add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    if v.mode == ViewMode::Binary {
        render_binary_header(f, area, v, theme);
        return;
    }
    // The table counts records rather than lines, and says where the cursor is.
    if v.table_active()
        && let Some((row, total, exact)) = v.table_status()
    {
        let more = if exact { "" } else { "+" };
        let text = format!(
            " {}: {}  [{}]  {}/{}{more} {}",
            crate::l10n::trd("View"),
            ellipsize(&v.name, area.width.saturating_sub(36) as usize),
            crate::l10n::trd("Table"),
            row + 1,
            total,
            crate::l10n::trd("rows"),
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&text, area.width as usize),
                theme.menubar.add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    let mode = match v.mode {
        ViewMode::Hex => crate::l10n::trd("Hex"),
        ViewMode::Text if v.markdown_active() => crate::l10n::trd("Markdown"),
        ViewMode::Text => crate::l10n::trd("Text"),
        ViewMode::Map => crate::l10n::trd("Map"),
        ViewMode::Binary => crate::l10n::trd("Binary"),
    };
    let wrap = if v.wrap { crate::l10n::trd("Wrap") } else { crate::l10n::trd("Unwrap") };
    let trunc =
        if v.truncated { format!(" [{}]", crate::l10n::trd("TRUNCATED")) } else { String::new() };
    let total = match v.mode {
        ViewMode::Text => v.line_count(),
        ViewMode::Hex => v.hex_rows(),
        ViewMode::Map | ViewMode::Binary => 1,
    };
    // While the line index is still being built, the total is a lower bound, so
    // flag it with a trailing '+'.
    let more = if v.mode == ViewMode::Text && !v.fully_indexed() { "+" } else { "" };
    let unit =
        if v.mode == ViewMode::Hex { crate::l10n::trd("rows") } else { crate::l10n::trd("lines") };
    // Follow mode: on, or paused with a count of what arrived since.
    let follow = match v.follow_status() {
        None => String::new(),
        Some((false, _)) => format!("  [{}]", crate::l10n::trd("Follow")),
        Some((true, 0)) => format!("  [{}]", crate::l10n::trd("Paused")),
        Some((true, n)) => format!("  [{} +{n}]", crate::l10n::trd("Paused")),
    };
    // The blamed cursor line's commit takes the header's spare room.
    if let Some((blame, cursor)) = v.active_blame() {
        let who = match blame.commit_of(cursor) {
            Some(c) if c.uncommitted() => crate::l10n::trd("Not committed yet"),
            Some(c) => {
                let (y, m, d, ..) = crate::util::bytes::civil_parts(c.time);
                format!("{}  {}  {y:04}-{m:02}-{d:02}  {}", c.short(), c.author, c.summary)
            }
            None => String::new(),
        };
        let text = format!(
            " {}: {}  [{}]{follow}  {}/{}{more}  {who}",
            crate::l10n::trd("View"),
            ellipsize(&v.name, 24),
            crate::l10n::trd("Blame"),
            cursor + 1,
            total.max(1),
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&ellipsize(&text, area.width as usize), area.width as usize),
                theme.menubar.add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    let loading = if v.blame_loading() {
        format!("  [{}…]", crate::l10n::trd("Blame"))
    } else {
        String::new()
    };
    let follow = follow + &loading;
    let text = format!(
        " {}: {}  [{mode}/{wrap}]{follow}  {}/{}{more} {unit}{trunc}",
        crate::l10n::trd("View"),
        ellipsize(&v.name, area.width.saturating_sub(40 + follow.len() as u16) as usize),
        v.top + 1,
        total.max(1),
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&text, area.width as usize),
            theme.menubar.add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

/// Draw the text view. Returns, for each screen row drawn, the line on it and
/// whether that row is the line's first (the rest are its wrapped tail) — what
/// the blame column needs to line up with the text.
fn render_text(
    f: &mut Frame,
    area: Rect,
    v: &mut ViewerState,
    theme: &Theme,
) -> Vec<(usize, bool)> {
    let default = theme.text_fg;
    let bg = theme.panel_bg;
    // A "Find all" hit tints the whole line, reusing the theme's inactive-cursor
    // bar exactly as the editor does.
    let found_bg = theme.cursor_inactive.bg.unwrap_or(bg);
    let width = area.width as usize;
    let rows = area.height as usize;
    let highlighted = v.has_syntax();
    let log_levels = v.log_levels();
    let blame_cursor = v.active_blame().map(|(_, cursor)| cursor);
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    let mut row_lines: Vec<(usize, bool)> = Vec::with_capacity(rows);
    let mut tinted: Vec<(usize, Color)> = Vec::new();
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
        } else if log_levels
            && let Some(color) =
                super::loglevel::level_of(&raw).and_then(|l| super::loglevel::color(l, theme))
        {
            // A log line naming a severity is drawn whole in its colour.
            vec![color; chars.len()]
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

        // Every visual row of a matched line carries the tint, and so does the
        // blame cursor's line.
        let row_bg =
            if v.line_found(line_idx) || blame_cursor == Some(line_idx) { found_bg } else { bg };
        let first_row = lines.len();
        if v.wrap {
            if chars.is_empty() {
                lines.push(build_spans(&[], 0, &fg, default, row_bg));
                row_lines.push((line_idx, true));
            } else {
                let mut start = 0;
                while start < chars.len() && lines.len() < rows {
                    let end = (start + width.max(1)).min(chars.len());
                    lines.push(build_spans(&chars[start..end], start, &fg, default, row_bg));
                    row_lines.push((line_idx, start == 0));
                    start = end;
                }
            }
        } else {
            let from = v.h_offset.min(chars.len());
            lines.push(build_spans(&chars[from..], from, &fg, default, row_bg));
            row_lines.push((line_idx, true));
        }
        if row_bg != bg {
            tinted.extend((first_row..lines.len()).map(|r| (r, row_bg)));
        }
        line_idx += 1;
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)), area);
    // A tinted line (a "Find all" hit, the blame cursor) is a bar across the
    // whole row, not just a shade behind however much text it has.
    for (row, color) in tinted {
        let rect = Rect { y: area.y + row as u16, height: 1, ..area };
        f.buffer_mut().set_style(rect, Style::default().bg(color));
    }
    row_lines
}

/// Width of the blame column: author and date where the screen can spare it,
/// just the date where it can't.
fn blame_gutter_width(content_width: u16) -> u16 {
    if content_width >= 80 {
        BLAME_WIDE
    } else if content_width >= 30 {
        BLAME_NARROW
    } else {
        0
    }
}

/// `▌ author       2026-09-12 ` and `▌ 2026-09-12 `.
const BLAME_WIDE: u16 = 26;
const BLAME_NARROW: u16 = 13;
const BLAME_AUTHOR: usize = 12;

/// Draw the blame column beside the text. Each line gets a bar shaded by the age
/// of the commit that last touched it — the newest brightest — and the author
/// and date are written once per run of lines from the same commit, so a block
/// of code reads as one change rather than a column of repeated names.
fn render_blame_gutter(
    f: &mut Frame,
    area: Rect,
    v: &ViewerState,
    rows: &[(usize, bool)],
    theme: &Theme,
) {
    let Some((blame, cursor)) = v.active_blame() else { return };
    let ages = blame.age_ranks();
    let wide = area.width >= BLAME_WIDE;
    let base = Style::default().bg(theme.panel_bg);
    let mut out: Vec<Line> = Vec::with_capacity(area.height as usize);
    for (row, &(li, first)) in rows.iter().enumerate() {
        let commit_idx = blame.lines.get(li).copied();
        let Some(commit) = commit_idx.and_then(|i| blame.commits.get(i as usize)) else {
            // Past the end of the blame: a line added since, in a followed file.
            out.push(Line::from(Span::styled(" ".repeat(area.width as usize), base)));
            continue;
        };
        // Label the first row of a run, the top of the screen (so a run
        // scrolled into from above still says whose it is), and the cursor.
        let run_start = li == 0 || blame.lines.get(li - 1).copied() != commit_idx;
        let label = if first && (run_start || row == 0 || li == cursor) {
            let date = if commit.uncommitted() {
                crate::l10n::trd("uncommitted")
            } else {
                let (y, m, d, ..) = crate::util::bytes::civil_parts(commit.time);
                format!("{y:04}-{m:02}-{d:02}")
            };
            if wide && !commit.uncommitted() {
                format!(
                    "{} {date}",
                    pad_right(&ellipsize(&commit.author, BLAME_AUTHOR), BLAME_AUTHOR)
                )
            } else {
                date
            }
        } else {
            String::new()
        };
        let label = pad_right(&ellipsize(&label, area.width as usize - 3), area.width as usize - 3);
        let age = commit_idx.and_then(|i| ages.get(i as usize)).copied().unwrap_or(0.0);
        let (bar, bar_fg) = age_bar(commit, age, theme);
        let text_style = if li == cursor {
            theme.cursor
        } else {
            base.fg(if commit.uncommitted() { theme.marked_fg } else { theme.panel_border })
        };
        out.push(Line::from(vec![
            Span::styled(bar.to_string(), base.fg(bar_fg)),
            Span::styled(" ", base),
            Span::styled(label, text_style),
            Span::styled(" ", base),
        ]));
    }
    f.render_widget(Paragraph::new(out).style(base), area);
}

/// The bar glyph and colour for a commit's `age` rank in the file (`0.0`
/// newest, `1.0` oldest — see `Blame::age_ranks`): the newest change in the
/// accent colour, fading towards the background the older it is. Without
/// truecolor, density glyphs carry the age instead.
fn age_bar(commit: &crate::git::blame::BlameCommit, age: f64, theme: &Theme) -> (char, Color) {
    if commit.uncommitted() {
        return ('▌', theme.marked_fg);
    }
    if !theme.truecolor {
        return (['█', '▓', '▒', '░'][((age * 4.0) as usize).min(3)], theme.hotkey_fg);
    }
    use crate::ui::graphics::raster::{over, rgb};
    let (r, g, b) = over(rgb(theme.panel_bg), rgb(theme.hotkey_fg), 1.0 - 0.7 * age);
    ('▌', Color::Rgb(r, g, b))
}

/// Render text as an *approximation* of rendered Markdown: per-line styling from
/// [`render_line`](super::markdown::render_line) (headings colored by level,
/// emphasis/code/links styled, markers dimmed). Fenced code blocks are tracked
/// across lines and framed in a box, their content shown literally (so `#` or
/// `*` inside code isn't mistaken for markup); pipe tables are laid out as a
/// whole and drawn boxed with aligned columns. Mirrors `render_text`'s wrap /
/// horizontal-scroll handling.
fn render_markdown(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    let bg = theme.panel_bg;
    let default = Style::default().fg(theme.text_fg).bg(bg);
    let border = Style::default().fg(theme.panel_border).bg(bg);
    let code = Style::default().fg(theme.doc_fg).bg(bg);
    let width = area.width as usize;
    let rows = area.height as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    let mut line_idx = v.top;
    // Whether the top of the viewport is already inside a code block whose
    // opening fence scrolled off the top.
    let mut in_code = v.in_code_fence_at(v.top);
    // The table being drawn: its source-line range and its rendered rows.
    let mut table: Option<(std::ops::Range<usize>, Vec<Vec<super::markdown::Row>>)> = None;

    while lines.len() < rows && line_idx < v.line_count() {
        let line = v.line_str(line_idx);
        let raw: Vec<char> = line.chars().collect();

        // A fence line becomes the top (opening) or bottom (closing) of the box.
        if super::markdown::is_fence(&line) {
            let opening = !in_code;
            in_code = !in_code;
            let lang = if opening {
                super::markdown::fence_info(&line).unwrap_or_default()
            } else {
                String::new()
            };
            lines.push(code_border_line(width, opening, &lang, border, code));
            line_idx += 1;
            continue;
        }

        // Code content: draw literally between the vertical box borders.
        if in_code {
            push_code_line(&mut lines, &raw, width, rows, v.h_offset, v.wrap, border, code);
            line_idx += 1;
            continue;
        }

        // Table line: the whole table is laid out once, as it is first met, and
        // each of its lines draws its rows (borders included) unwrapped.
        if !table.as_ref().is_some_and(|(r, _)| r.contains(&line_idx)) {
            table = v.table_block_at(line_idx).map(|r| {
                let src: Vec<String> = r.clone().map(|i| v.line_str(i)).collect();
                (r, super::markdown::render_table(&src, theme, true))
            });
        }
        if let Some((r, table_rows)) = &table
            && r.contains(&line_idx)
        {
            for (chars, styles) in &table_rows[line_idx - r.start] {
                if lines.len() >= rows {
                    break;
                }
                let styles = tint_hex_colors(chars, styles.clone());
                let from = if v.wrap { 0 } else { v.h_offset.min(chars.len()) };
                lines.push(build_styled(&chars[from..], from, &styles, default));
            }
            line_idx += 1;
            continue;
        }

        // Ordinary Markdown line: markup is stripped, leaving display text + styles.
        let (chars, styles) = super::markdown::render_line_with(&raw, theme, !v.prose);
        let styles = tint_hex_colors(&chars, styles);

        if v.wrap && v.prose {
            for (start, end) in super::markdown::word_rows(&chars, width) {
                if lines.len() >= rows {
                    break;
                }
                lines.push(build_styled(&chars[start..end], start, &styles, default));
            }
        } else if v.wrap {
            if chars.is_empty() {
                lines.push(build_styled(&[], 0, &styles, default));
            } else {
                let mut start = 0;
                while start < chars.len() && lines.len() < rows {
                    let end = (start + width.max(1)).min(chars.len());
                    lines.push(build_styled(&chars[start..end], start, &styles, default));
                    start = end;
                }
            }
        } else {
            let from = v.h_offset.min(chars.len());
            lines.push(build_styled(&chars[from..], from, &styles, default));
        }
        line_idx += 1;
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(bg)), area);
}

/// Tint the `#` of any hex-color token in `chars`, regardless of the Markdown
/// styling.
fn tint_hex_colors(chars: &[char], mut styles: Vec<Style>) -> Vec<Style> {
    for (i, color) in crate::ui::hexcolor::hex_color_hashes(chars) {
        if i < styles.len() {
            styles[i] = styles[i].fg(color);
        }
    }
    styles
}

/// A code-box border row spanning the full content width: `┌──…──┐` when
/// `opening` (labeled with the language, if any) or `└──…──┘` when closing.
fn code_border_line(
    width: usize,
    opening: bool,
    lang: &str,
    border: Style,
    label: Style,
) -> Line<'static> {
    let (corner_l, corner_r) = if opening { ('┌', '┐') } else { ('└', '┘') };
    if width < 2 {
        return Line::from(Span::styled("─".repeat(width), border));
    }
    let inner = width - 2; // dashes between the two corners
    let mut spans = vec![Span::styled(corner_l.to_string(), border)];
    let label_w = lang.chars().count();
    // Embed "─ lang " in the opening border when it comfortably fits.
    if opening && label_w > 0 && inner >= label_w + 4 {
        let after = inner - 3 - label_w; // "─ " (2) + label + " " (1) + dashes
        spans.push(Span::styled("─ ".to_string(), border));
        spans.push(Span::styled(lang.to_string(), label));
        spans.push(Span::styled(format!(" {}", "─".repeat(after)), border));
    } else {
        spans.push(Span::styled("─".repeat(inner), border));
    }
    spans.push(Span::styled(corner_r.to_string(), border));
    Line::from(spans)
}

/// Push the boxed code content for one source line — `│ …code… │` — honoring
/// wrap / horizontal-scroll the same way ordinary lines are handled. Falls back
/// to plain text when the area is too narrow for a box.
#[allow(clippy::too_many_arguments)]
fn push_code_line(
    lines: &mut Vec<Line<'static>>,
    raw: &[char],
    width: usize,
    rows: usize,
    h_offset: usize,
    wrap: bool,
    border: Style,
    code: Style,
) {
    // Interior text width: the two `│` borders plus a space of padding each side.
    let code_w = width.saturating_sub(4);
    if width < 4 || code_w == 0 {
        let from = h_offset.min(raw.len());
        lines.push(Line::from(Span::styled(raw[from..].iter().collect::<String>(), code)));
        return;
    }
    let push_seg = |lines: &mut Vec<Line<'static>>, seg: &[char]| {
        let pad = code_w - seg.len();
        lines.push(Line::from(vec![
            Span::styled("│ ".to_string(), border),
            Span::styled(seg.iter().collect::<String>(), code),
            Span::styled(" ".repeat(pad + 1), code),
            Span::styled("│".to_string(), border),
        ]));
    };
    if wrap {
        if raw.is_empty() {
            push_seg(lines, &[]);
        } else {
            let mut start = 0;
            while start < raw.len() && lines.len() < rows {
                let end = (start + code_w).min(raw.len());
                push_seg(lines, &raw[start..end]);
                start = end;
            }
        }
    } else {
        let from = h_offset.min(raw.len());
        let end = (from + code_w).min(raw.len());
        push_seg(lines, &raw[from..end]);
    }
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
            ascii.push(if b.is_ascii_graphic() || *b == b' ' { *b as char } else { '.' });
        }
        let line = format!("{off:08x}  {hex:<49} |{ascii}|");
        lines.push(Line::from(Span::styled(line, style)));
    }
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)), area);
}

fn render_footer(f: &mut Frame, area: Rect, v: &ViewerState, theme: &Theme) {
    // Same full-width, number+label styling as the main program, translated.
    let labels = v.footer_labels().map(crate::l10n::trd);
    crate::ui::fkeys::render(f, area, &labels, theme);
}
