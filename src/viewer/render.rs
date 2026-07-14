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

    render_header(f, header, v, theme);
    // An image file shows the decoded image fullscreen (pixel graphics where
    // available, else half-block cell art); F8 toggles to the raw text/hex.
    if v.active_image().is_some() {
        render_image(f, content, v, theme, gfx);
        render_footer(f, footer, v, theme);
        return;
    }

    // Make sure the lines about to be drawn (plus one past, so the last line's
    // extent is known) are indexed — the rest of the file stays unscanned.
    if v.mode == ViewMode::Text {
        v.extend_to_line(v.top + v.view_rows);
    }
    match v.mode {
        ViewMode::Hex => render_hex(f, content, v, theme),
        // Markdown files render the approximation by default; F8 shows the raw,
        // syntax-highlighted source.
        ViewMode::Text if v.markdown_active() => render_markdown(f, content, v, theme),
        ViewMode::Text => render_text(f, content, v, theme),
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
    if v.outline_sel < v.outline_top {
        v.outline_top = v.outline_sel;
    } else if rows > 0 && v.outline_sel >= v.outline_top + rows {
        v.outline_top = v.outline_sel + 1 - rows;
    }
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
    let mode = match v.mode {
        ViewMode::Hex => crate::l10n::trd("Hex"),
        ViewMode::Text if v.markdown_active() => crate::l10n::trd("Markdown"),
        ViewMode::Text => crate::l10n::trd("Text"),
    };
    let wrap = if v.wrap { crate::l10n::trd("Wrap") } else { crate::l10n::trd("Unwrap") };
    let trunc = if v.truncated {
        format!(" [{}]", crate::l10n::trd("TRUNCATED"))
    } else {
        String::new()
    };
    let total = match v.mode {
        ViewMode::Text => v.line_count(),
        ViewMode::Hex => v.hex_rows(),
    };
    // While the line index is still being built, the total is a lower bound, so
    // flag it with a trailing '+'.
    let more = if v.mode == ViewMode::Text && !v.fully_indexed() { "+" } else { "" };
    let unit = if v.mode == ViewMode::Hex {
        crate::l10n::trd("rows")
    } else {
        crate::l10n::trd("lines")
    };
    let text = format!(
        " {}: {}  [{mode}/{wrap}]  {}/{}{more} {unit}{trunc}",
        crate::l10n::trd("View"),
        ellipsize(&v.name, area.width.saturating_sub(40) as usize),
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

/// Render text as an *approximation* of rendered Markdown: per-line styling from
/// [`render_line`](super::markdown::render_line) (headings colored by level,
/// emphasis/code/links styled, markers dimmed). Fenced code blocks are tracked
/// across lines and framed in a box, their content shown literally (so `#` or
/// `*` inside code isn't mistaken for markup). Mirrors `render_text`'s wrap /
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

    while lines.len() < rows && line_idx < v.line_count() {
        let line = v.line_str(line_idx);
        let raw: Vec<char> = line.chars().collect();

        // A fence line becomes the top (opening) or bottom (closing) of the box.
        if super::markdown::is_fence(&line) {
            let opening = !in_code;
            in_code = !in_code;
            let lang = if opening { super::markdown::fence_info(&line).unwrap_or_default() } else { String::new() };
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

        // Ordinary Markdown line: markup is stripped, leaving display text + styles.
        let (chars, mut styles) = super::markdown::render_line(&raw, theme);
        // Tint the `#` of any hex-color token, regardless of the Markdown styling.
        for (i, color) in crate::ui::hexcolor::hex_color_hashes(&chars) {
            if i < styles.len() {
                styles[i] = styles[i].fg(color);
            }
        }

        if v.wrap {
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

/// A code-box border row spanning the full content width: `┌──…──┐` when
/// `opening` (labeled with the language, if any) or `└──…──┘` when closing.
fn code_border_line(width: usize, opening: bool, lang: &str, border: Style, label: Style) -> Line<'static> {
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
        // Highlight the whole query while it is still marked (pre-filled), so it
        // reads as "type to replace" — like the copy/rename input field.
        let text_style = if v.search_selected && !q.is_empty() {
            Style::default().fg(theme.panel_bg).bg(theme.header_fg)
        } else {
            Style::default()
        };
        let line = Line::from(vec![
            Span::styled("Search: ", Style::default().fg(theme.header_fg)),
            Span::styled(q.clone(), text_style),
        ]);
        f.render_widget(Paragraph::new(line), area);
        let cx = area.x + 8 + v.search_cursor.min(q.chars().count()) as u16;
        f.set_cursor_position(ratatui::layout::Position::new(cx, area.y));
        return;
    }

    // Same full-width, number+label styling as the main program, translated.
    let labels = v.footer_labels().map(crate::l10n::trd);
    crate::ui::fkeys::render(f, area, &labels, theme);
}
