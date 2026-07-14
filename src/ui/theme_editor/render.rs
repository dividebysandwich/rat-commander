//! Rendering for the visual theme editor.
//!
//! The editor's own chrome (title, picker, item list, color picker, buttons) is
//! drawn with the app's stable `theme` so it stays readable no matter how the
//! edited colors look. Only the preview pane is drawn with a [`Theme`] derived
//! live from the working [`ThemeSpec`], so the user sees exactly what the theme
//! will look like.

use super::{Focus, Overlay, ThemeEditor, SWATCHES, rgb_of};
use crate::ui::dialog::widgets::centered;
use crate::ui::theme::{PreviewKind, Theme, THEME_FIELDS};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

const LEFT_W: u16 = 40;

/// Fill every cell of `area` with `style` (a background wash).
fn fill(f: &mut Frame, area: Rect, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let row = " ".repeat(area.width as usize);
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        buf.set_string(area.x, y, &row, style);
    }
}

/// Write `s` at `(x, y)` if it is inside `area`, clipped to the right edge.
fn put(f: &mut Frame, area: Rect, x: u16, y: u16, s: &str, style: Style) {
    if y < area.top() || y >= area.bottom() || x >= area.right() {
        return;
    }
    let max = (area.right() - x) as usize;
    let s: String = s.chars().take(max).collect();
    f.buffer_mut().set_string(x, y, s, style);
}

fn hex(c: Color) -> String {
    let (r, g, b) = rgb_of(c);
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn render(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, theme: &Theme) {
    // Backdrop so no panel content shows through.
    fill(f, area, Style::default().bg(theme.dialog_bg).fg(theme.dialog_fg));

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(3),    // body
            Constraint::Length(1), // buttons
        ])
        .split(area);
    render_title(f, rows[0], ed, theme);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(LEFT_W.min(rows[1].width)), Constraint::Min(10)])
        .split(rows[1]);
    render_left(f, body[0], ed, theme);
    render_preview(f, body[1], ed, theme);

    render_buttons(f, rows[2], ed, theme);

    // Snapshot the overlay's Copy fields so the immutable borrow of `ed.overlay`
    // ends before the overlay renderers re-borrow `ed` mutably (to store zones).
    enum Ov {
        None,
        Switch(usize, usize),
        Exit(usize),
        SaveAs(String, usize),
    }
    let ov = match &ed.overlay {
        Overlay::None => Ov::None,
        Overlay::ConfirmSwitch { target, button } => Ov::Switch(*target, *button),
        Overlay::ConfirmExit { button } => Ov::Exit(*button),
        Overlay::SaveAs { name, cursor } => Ov::SaveAs(name.clone(), *cursor),
    };
    match ov {
        Ov::None => {}
        Ov::Switch(target, button) => render_confirm_switch(f, area, ed, target, button, theme),
        Ov::Exit(button) => render_confirm_exit(f, area, ed, button, theme),
        Ov::SaveAs(name, cursor) => render_save_as(f, area, &name, cursor, theme),
    }
}

fn render_title(f: &mut Frame, area: Rect, ed: &ThemeEditor, theme: &Theme) {
    let star = if ed.dirty() { " *" } else { "" };
    let left = format!(" Theme Editor — {}{star} ", ed.spec.name);
    let style = Style::default().bg(theme.dialog_title).fg(theme.dialog_bg).add_modifier(Modifier::BOLD);
    fill(f, area, style);
    put(f, area, area.x, area.y, &left, style);
    let help = "F2 Save   Tab Focus   Esc Close ";
    if (help.len() as u16) < area.width {
        put(f, area, area.right() - help.len() as u16, area.y, help, style);
    }
}

// ---------------------------------------------------------------------------
// Left column: theme picker, item list, color picker
// ---------------------------------------------------------------------------

fn render_left(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, theme: &Theme) {
    let picker_h = 3u16;
    let color_h = if ed.truecolor { 6u16 } else { 8u16 };
    let cols = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(picker_h),
            Constraint::Min(3),
            Constraint::Length(color_h.min(area.height.saturating_sub(picker_h + 3))),
        ])
        .split(area);
    render_picker(f, cols[0], ed, theme);
    render_item_list(f, cols[1], ed, theme);
    render_color_picker(f, cols[2], ed, theme);
}

fn boxed<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let border = if focused { theme.dialog_title } else { theme.dialog_border_fg };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border).bg(theme.dialog_bg))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme.dialog_title).bg(theme.dialog_bg).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg))
}

fn render_picker(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, theme: &Theme) {
    let block = boxed("Theme", ed.focus == Focus::Picker, theme);
    let inner = block.inner(area);
    ed.z_picker = inner;
    f.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let name = ed.names.get(ed.picker).map(String::as_str).unwrap_or("");
    let text = format!("◄ {name} ►");
    let style = if ed.focus == Focus::Picker {
        theme.dialog_selection
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(text, style))).alignment(Alignment::Center),
        inner,
    );
}

fn render_item_list(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, theme: &Theme) {
    let block = boxed("Colors", ed.focus == Focus::List, theme);
    let inner = block.inner(area);
    ed.z_list = inner;
    f.render_widget(block, area);
    let h = inner.height as usize;
    if h == 0 || inner.width < 12 {
        return;
    }
    // Keep the selected item within the visible window.
    if ed.item < ed.item_top {
        ed.item_top = ed.item;
    } else if ed.item >= ed.item_top + h {
        ed.item_top = ed.item + 1 - h;
    }
    let max_top = THEME_FIELDS.len().saturating_sub(h);
    ed.item_top = ed.item_top.min(max_top);

    let iw = inner.width as usize;
    let hexw = 8usize; // " #rrggbb"
    let sw = 2usize; // swatch
    let text_w = iw.saturating_sub(hexw + sw + 1);
    for (row, i) in (ed.item_top..THEME_FIELDS.len()).take(h).enumerate() {
        let y = inner.y + row as u16;
        let meta = &THEME_FIELDS[i];
        let selected = i == ed.item;
        let base = if selected {
            theme.dialog_selection
        } else {
            Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
        };
        // Row background.
        put(f, inner, inner.x, y, &" ".repeat(iw), base);
        let marker = if selected { "▶ " } else { "  " };
        let label = format!("{marker}{} · {}", meta.group, meta.label);
        let label: String = label.chars().take(text_w).collect();
        put(f, inner, inner.x, y, &label, base);
        // Swatch + hex, right-aligned.
        let color = ed.spec.color_at(i);
        let sx = inner.x + inner.width - (hexw + sw) as u16;
        put(f, inner, sx, y, "  ", Style::default().bg(color));
        put(f, inner, sx + sw as u16, y, &format!(" {}", hex(color)), base);
    }
}

fn render_color_picker(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, theme: &Theme) {
    let title = THEME_FIELDS[ed.item.min(THEME_FIELDS.len() - 1)].label;
    let block = boxed(title, ed.focus == Focus::Color, theme);
    let inner = block.inner(area);
    ed.z_color = inner;
    f.render_widget(block, area);
    if inner.height == 0 || inner.width < 8 {
        return;
    }
    let color = ed.spec.color_at(ed.item);
    let (r, g, b) = rgb_of(color);
    // Header: the hex code — editable, type six digits as an alternative to the
    // sliders — plus a swatch of the current color.
    let (htext, caret) = match &ed.hex_input {
        Some(buf) => (format!("#{}{}", buf, "_".repeat(6usize.saturating_sub(buf.len()))), Some(buf.len())),
        None => (hex(color), None),
    };
    let hstyle = if caret.is_some() {
        Style::default().fg(theme.dialog_bg).bg(theme.dialog_title).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg).add_modifier(Modifier::BOLD)
    };
    put(f, inner, inner.x, inner.y, &htext, hstyle);
    let sx = inner.x + inner.width.saturating_sub(6);
    put(f, inner, sx, inner.y, "      ", Style::default().bg(color));
    if let Some(n) = caret {
        let cx = inner.x + 1 + n as u16;
        if cx < sx {
            f.set_cursor_position((cx, inner.y));
        }
    }
    if ed.truecolor {
        // R/G/B gauges.
        let chans = [("R", r, Color::Rgb(255, 60, 60)), ("G", g, Color::Rgb(60, 255, 60)), ("B", b, Color::Rgb(80, 120, 255))];
        for (idx, (name, val, accent)) in chans.iter().enumerate() {
            let y = inner.y + 1 + idx as u16;
            if y >= inner.bottom() {
                break;
            }
            let active = ed.focus == Focus::Color && ed.channel == idx;
            let lstyle = if active {
                Style::default().fg(theme.dialog_bg).bg(*accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
            };
            put(f, inner, inner.x, y, &format!("{}{} {:>3} ", if active { "▶" } else { " " }, name, val), lstyle);
            // Gauge bar.
            let bx = inner.x + 7;
            let bw = inner.right().saturating_sub(bx) as usize;
            if bw > 0 {
                let filled = (*val as usize * bw) / 255;
                put(f, inner, bx, y, &"█".repeat(filled), Style::default().fg(*accent).bg(theme.dialog_bg));
                put(f, inner, bx + filled as u16, y, &"░".repeat(bw - filled), Style::default().fg(theme.panel_border).bg(theme.dialog_bg));
            }
        }
    } else {
        // 16-swatch grid (4 columns).
        let cell_w = 4u16;
        for (i, sw) in SWATCHES.iter().enumerate() {
            let col = (i % 4) as u16;
            let rowi = (i / 4) as u16;
            let x = inner.x + col * cell_w;
            let y = inner.y + 1 + rowi;
            if y >= inner.bottom() || x + 3 > inner.right() {
                continue;
            }
            let selected = ed.focus == Focus::Color && ed.swatch == i;
            let sc = Color::Rgb(sw.0, sw.1, sw.2);
            put(f, inner, x, y, "   ", Style::default().bg(sc));
            if selected {
                put(f, inner, x, y, " ▪ ", Style::default().bg(sc).fg(contrast(sc)));
            }
        }
    }
}

/// Black or white, whichever reads better on `bg`.
fn contrast(bg: Color) -> Color {
    let (r, g, b) = rgb_of(bg);
    let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    if luma > 140.0 { Color::Black } else { Color::White }
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

fn render_buttons(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, theme: &Theme) {
    fill(f, area, Style::default().bg(theme.dialog_bg).fg(theme.dialog_fg));
    let labels = ["[ Save ]", "[ Save as… ]", "[ Cancel ]"];
    let gap = 2usize;
    let total: usize = labels.iter().map(|l| l.chars().count()).sum::<usize>() + gap * 2;
    let mut x = area.x + (area.width.saturating_sub(total as u16)) / 2;
    for (i, label) in labels.iter().enumerate() {
        let focused = ed.focus == Focus::Buttons && ed.button == i;
        let style = if focused { theme.button_focused } else { theme.button };
        let w = label.chars().count() as u16;
        put(f, area, x, area.y, label, style);
        ed.z_buttons[i] = Rect { x, y: area.y, width: w, height: 1 };
        x += w + gap as u16;
    }
}

// ---------------------------------------------------------------------------
// Live preview
// ---------------------------------------------------------------------------

fn render_preview(f: &mut Frame, area: Rect, ed: &ThemeEditor, chrome: &Theme) {
    let block = boxed("Preview", false, chrome);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 8 || inner.height < 4 {
        return;
    }
    let pt = Theme::from_spec(&ed.spec, ed.truecolor);
    match ed.preview_kind() {
        PreviewKind::Panels => preview_panels(f, inner, ed, &pt),
        PreviewKind::Dialog => preview_dialog(f, inner, &pt),
        PreviewKind::Editor => preview_editor(f, inner, &pt),
    }
}

fn preview_panels(f: &mut Frame, area: Rect, ed: &ThemeEditor, pt: &Theme) {
    fill(f, area, Style::default().bg(pt.panel_bg).fg(pt.panel_fg));
    // Menu bar.
    let mb = Style::default().bg(pt.menu_bg).fg(pt.menu_fg);
    put(f, area, area.x, area.y, &" ".repeat(area.width as usize), pt.menubar);
    put(f, area, area.x + 1, area.y, " Left   File   Command   Options   Right", pt.menubar);

    let body = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height.saturating_sub(2) };
    let halves = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body);
    for (side, ph) in halves.iter().enumerate() {
        let active = side == 0;
        let border = if active { pt.panel_border_active } else { pt.panel_border };
        let blk = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border).bg(pt.panel_bg))
            .style(Style::default().bg(pt.panel_bg).fg(pt.panel_fg));
        let pi = blk.inner(*ph);
        f.render_widget(blk, *ph);
        if pi.height == 0 {
            continue;
        }
        put(f, pi, pi.x, pi.y, "Name        Size", Style::default().fg(pt.header_fg).bg(pt.panel_bg).add_modifier(Modifier::BOLD));
        let files: [(&str, Color); 6] = [
            ("/..", pt.dir_fg),
            ("src", pt.dir_fg),
            ("run.sh", pt.exec_fg),
            ("link", pt.symlink_fg),
            ("data.zip", pt.archive_fg),
            ("photo.jpg", pt.marked_fg),
        ];
        for (row, (name, color)) in files.iter().enumerate() {
            let y = pi.y + 1 + row as u16;
            if y >= pi.bottom() {
                break;
            }
            // Cursor bar on the second row of each panel.
            if row == 1 {
                let cur = if active { pt.cursor } else { pt.cursor_inactive };
                put(f, pi, pi.x, y, &" ".repeat(pi.width as usize), cur);
                put(f, pi, pi.x, y, name, cur);
            } else {
                put(f, pi, pi.x, y, name, Style::default().fg(*color).bg(pt.panel_bg));
            }
        }
    }

    // Function-key bar along the bottom.
    let fy = area.bottom().saturating_sub(1);
    let labels = ["Help", "Menu", "View", "Edit", "Copy", "RenMov", "Mkdir", "Delete", "PullDn", "Quit"];
    let total = area.width as usize;
    let seg = total / labels.len().max(1);
    let mut x = area.x as usize;
    put(f, area, area.x, fy, &" ".repeat(total), pt.fkey_label);
    for (i, label) in labels.iter().enumerate() {
        let num = (i + 1).to_string();
        put(f, area, x as u16, fy, &num, pt.fkey_num);
        let lstyle = if pt.truecolor {
            Style::default().bg(pt.gradient_at(x, total)).fg(pt.bar_fg)
        } else {
            pt.fkey_label
        };
        put(f, area, (x + num.len()) as u16, fy, label, lstyle);
        x += seg;
    }

    // A little pulldown menu when a menu color is selected.
    if THEME_FIELDS[ed.item].group == "Pulldown menu" {
        let mw = 20u16.min(area.width.saturating_sub(2));
        let menu = Rect { x: area.x + 8, y: area.y + 1, width: mw, height: 5.min(area.height.saturating_sub(2)) };
        let blk = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(pt.menu_fg).bg(pt.menu_bg))
            .style(Style::default().bg(pt.menu_bg).fg(pt.menu_fg));
        let mi = blk.inner(menu);
        f.render_widget(Clear, menu);
        f.render_widget(blk, menu);
        let items = ["Settings", "Edit themes", "Quit"];
        for (row, it) in items.iter().enumerate() {
            let y = mi.y + row as u16;
            if y >= mi.bottom() {
                break;
            }
            if row == 1 {
                put(f, mi, mi.x, y, &" ".repeat(mi.width as usize), pt.menu_selection);
                put(f, mi, mi.x, y, it, pt.menu_selection);
            } else {
                put(f, mi, mi.x, y, it, Style::default().fg(pt.menu_fg).bg(pt.menu_bg));
            }
            // Underlined hotkey letter.
            put(f, mi, mi.x, y, &it.chars().next().map(|c| c.to_string()).unwrap_or_default(),
                Style::default().fg(pt.hotkey_fg).bg(if row == 1 { pt.menu_selection.bg.unwrap_or(pt.menu_bg) } else { pt.menu_bg }).add_modifier(Modifier::UNDERLINED));
        }
    }
    let _ = mb;
}

fn preview_dialog(f: &mut Frame, area: Rect, pt: &Theme) {
    fill(f, area, Style::default().bg(pt.panel_bg).fg(pt.panel_fg));
    let w = 40u16.min(area.width.saturating_sub(2));
    let h = 11u16.min(area.height);
    let d = centered(area, w, h);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pt.dialog_border_fg).bg(pt.dialog_border_bg))
        .title(Span::styled(" Rename ", Style::default().fg(pt.dialog_title).bg(pt.dialog_border_bg).add_modifier(Modifier::BOLD)))
        .title_alignment(Alignment::Center)
        .style(Style::default().fg(pt.dialog_fg).bg(pt.dialog_bg));
    let di = blk.inner(d);
    f.render_widget(Clear, d);
    f.render_widget(blk, d);
    if di.height < 5 {
        return;
    }
    put(f, di, di.x + 1, di.y, "New name:", Style::default().fg(pt.dialog_fg).bg(pt.dialog_bg));
    // Input field.
    let iy = di.y + 1;
    put(f, di, di.x + 1, iy, &" ".repeat(di.width.saturating_sub(2) as usize), Style::default().bg(pt.input_bg).fg(pt.input_fg));
    put(f, di, di.x + 1, iy, "document.txt", Style::default().bg(pt.input_bg).fg(pt.input_fg));
    // Selected option row.
    let sy = di.y + 3;
    put(f, di, di.x + 1, sy, &" ".repeat(di.width.saturating_sub(2) as usize), pt.dialog_selection);
    put(f, di, di.x + 1, sy, "(•) Selected option", pt.dialog_selection);
    put(f, di, di.x + 1, sy + 1, "( ) Another option", Style::default().fg(pt.dialog_fg).bg(pt.dialog_bg));
    // Error line.
    put(f, di, di.x + 1, sy + 2, "! name already exists", Style::default().fg(pt.error_fg).bg(pt.dialog_bg).add_modifier(Modifier::BOLD));
    // Buttons.
    let by = di.bottom().saturating_sub(1);
    let ok = "[ OK ]";
    let cancel = "[ Cancel ]";
    let bx = di.x + (di.width.saturating_sub((ok.len() + cancel.len() + 3) as u16)) / 2;
    put(f, di, bx, by, ok, pt.button_focused);
    put(f, di, bx + ok.len() as u16 + 3, by, cancel, pt.button);
}

fn preview_editor(f: &mut Frame, area: Rect, pt: &Theme) {
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pt.panel_border_active).bg(pt.panel_bg))
        .title(Span::styled(" editor.rs ", Style::default().fg(pt.header_fg).bg(pt.panel_bg).add_modifier(Modifier::BOLD)))
        .style(Style::default().bg(pt.panel_bg).fg(pt.text_fg));
    let ei = blk.inner(area);
    f.render_widget(blk, area);
    if ei.height < 2 {
        return;
    }
    let text = Style::default().fg(pt.text_fg).bg(pt.panel_bg);
    let lines = [
        "fn main() {",
        "    let accent = \"#ff8800\";",
        "    println!(\"hello, world\");",
        "}",
    ];
    for (row, ln) in lines.iter().enumerate() {
        let y = ei.y + row as u16;
        if y >= ei.bottom().saturating_sub(1) {
            break;
        }
        put(f, ei, ei.x, y, ln, text);
        // Tint any #rrggbb token with the color it denotes (like the real editor).
        let chars: Vec<char> = ln.chars().collect();
        for (idx, color) in crate::ui::hexcolor::hex_color_hashes(&chars) {
            let token: String = chars[idx..].iter().take_while(|c| **c == '#' || c.is_ascii_hexdigit()).collect();
            put(f, ei, ei.x + idx as u16, y, &token, Style::default().fg(color).bg(pt.panel_bg).add_modifier(Modifier::BOLD));
        }
    }
    // Status line.
    let sy = ei.bottom().saturating_sub(1);
    put(f, ei, ei.x, sy, &" ".repeat(ei.width as usize), pt.menubar);
    put(f, ei, ei.x, sy, " Ln 1/4   Col 1   UTF-8 ", pt.menubar);
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

fn render_confirm_switch(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, target: usize, button: usize, theme: &Theme) {
    let d = centered(area, 54u16.min(area.width.saturating_sub(4)), 7);
    let blk = confirm_block(" Unsaved changes ", theme);
    let di = blk.inner(d);
    f.render_widget(Clear, d);
    f.render_widget(blk, d);
    let to = ed.names.get(target).map(String::as_str).unwrap_or("");
    let msg = format!("Save changes to \"{}\" before switching to \"{}\"?", ed.spec.name, to);
    f.render_widget(
        Paragraph::new(msg)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)),
        Rect { height: di.height.saturating_sub(1), ..di },
    );
    render_button_row(f, di, di.bottom().saturating_sub(1), &["[ Save ]", "[ Discard ]", "[ Cancel ]"], button, theme, &mut ed.z_overlay);
}

fn render_confirm_exit(f: &mut Frame, area: Rect, ed: &mut ThemeEditor, button: usize, theme: &Theme) {
    let d = centered(area, 54u16.min(area.width.saturating_sub(4)), 7);
    let blk = confirm_block(" Unsaved changes ", theme);
    let di = blk.inner(d);
    f.render_widget(Clear, d);
    f.render_widget(blk, d);
    let msg = format!("Save changes to \"{}\" before closing the theme editor?", ed.spec.name);
    f.render_widget(
        Paragraph::new(msg)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)),
        Rect { height: di.height.saturating_sub(1), ..di },
    );
    render_button_row(f, di, di.bottom().saturating_sub(1), &["[ Save ]", "[ Discard ]", "[ Cancel ]"], button, theme, &mut ed.z_overlay);
}

fn confirm_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.dialog_border_fg).bg(theme.dialog_border_bg))
        .title(Span::styled(title.to_string(), Style::default().fg(theme.dialog_title).bg(theme.dialog_border_bg).add_modifier(Modifier::BOLD)))
        .title_alignment(Alignment::Center)
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg))
}

fn render_save_as(f: &mut Frame, area: Rect, name: &str, cursor: usize, theme: &Theme) {
    let w = 50u16.min(area.width.saturating_sub(4));
    let d = centered(area, w, 6);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.dialog_border_fg).bg(theme.dialog_border_bg))
        .title(Span::styled(" Save theme as ", Style::default().fg(theme.dialog_title).bg(theme.dialog_border_bg).add_modifier(Modifier::BOLD)))
        .title_alignment(Alignment::Center)
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
    let di = blk.inner(d);
    f.render_widget(Clear, d);
    f.render_widget(blk, d);
    if di.height < 3 {
        return;
    }
    put(f, di, di.x + 1, di.y, "Theme name:", Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
    let iy = di.y + 1;
    put(f, di, di.x + 1, iy, &" ".repeat(di.width.saturating_sub(2) as usize), Style::default().bg(theme.input_bg).fg(theme.input_fg));
    put(f, di, di.x + 1, iy, name, Style::default().bg(theme.input_bg).fg(theme.input_fg));
    put(f, di, di.x + 3, di.bottom().saturating_sub(1), "Enter Save   Esc Cancel", Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
    let cx = di.x + 1 + (cursor.min(di.width.saturating_sub(3) as usize)) as u16;
    f.set_cursor_position((cx, iy));
}

fn render_button_row(f: &mut Frame, area: Rect, y: u16, labels: &[&str], sel: usize, theme: &Theme, zones: &mut [Rect; 3]) {
    let gap = 2usize;
    let total: usize = labels.iter().map(|l| l.chars().count()).sum::<usize>() + gap * labels.len().saturating_sub(1);
    let mut x = area.x + (area.width.saturating_sub(total as u16)) / 2;
    for (i, label) in labels.iter().enumerate() {
        let style = if i == sel { theme.button_focused } else { theme.button };
        let w = label.chars().count() as u16;
        put(f, area, x, y, label, style);
        if i < zones.len() {
            zones[i] = Rect { x, y, width: w, height: 1 };
        }
        x += w + gap as u16;
    }
}
