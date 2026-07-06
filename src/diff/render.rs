//! Side-by-side rendering of the [`DiffView`].

use super::DiffView;
use crate::ui::theme::Theme;
use crate::util::text::{ellipsize, pad_right};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

const GUTTER: u16 = 3;
const LINENO_W: usize = 5; // "1234 "

pub fn render(f: &mut Frame, area: Rect, dv: &mut DiffView, theme: &Theme) {
    if area.height < 3 || area.width < 12 {
        return;
    }
    let status = Rect { height: 1, ..area };
    let footer = Rect { y: area.y + area.height - 1, height: 1, ..area };
    let body = Rect { y: area.y + 1, height: area.height - 2, ..area };

    let side_w = (body.width.saturating_sub(GUTTER)) / 2;
    let left_x = body.x;
    let gutter_x = body.x + side_w;
    let right_x = gutter_x + GUTTER;
    let right_w = body.width - side_w - GUTTER;

    render_status(f, status, dv, theme);

    dv.view_rows = body.height as usize;
    if dv.cursor < dv.top {
        dv.top = dv.cursor;
    } else if dv.cursor >= dv.top + dv.view_rows {
        dv.top = dv.cursor + 1 - dv.view_rows;
    }

    let base_bg = theme.panel_bg;
    let absent_bg = mix(base_bg, theme.panel_border, 0.5);

    for r in 0..dv.view_rows {
        let y = body.y + r as u16;
        let idx = dv.top + r;
        let row = dv.rows.get(idx).copied();
        let is_cursor = idx == dv.cursor;
        let in_delta = row.is_some_and(|rr| rr.delta.is_some());
        let is_active = row.is_some_and(|rr| rr.delta.is_some() && rr.delta == dv.active);

        // Per-side accent color (None = unchanged / blank).
        let (left_accent, right_accent) = match row {
            Some(rr) => match (rr.left, rr.right, rr.delta) {
                (_, _, None) => (None, None),
                (Some(_), Some(_), Some(_)) => (Some(theme.header_fg), Some(theme.header_fg)),
                (Some(_), None, Some(_)) => (Some(theme.error_fg), None),
                (None, Some(_), Some(_)) => (None, Some(theme.exec_fg)),
                _ => (None, None),
            },
            None => (None, None),
        };

        let left_no = row.and_then(|rr| rr.left);
        let right_no = row.and_then(|rr| rr.right);
        let left_text = left_no.and_then(|i| dv.left.get(i).map(String::as_str));
        let right_text = right_no.and_then(|i| dv.right.get(i).map(String::as_str));

        draw_cell(f, left_x, y, side_w, left_no, left_text, left_accent, in_delta, is_active, is_cursor, base_bg, absent_bg, theme);
        draw_cell(f, right_x, y, right_w, right_no, right_text, right_accent, in_delta, is_active, is_cursor, base_bg, absent_bg, theme);

        let (mark, style) = gutter(row, is_cursor, is_active, theme, base_bg);
        f.buffer_mut()
            .set_string(gutter_x, y, format!("{mark:^w$}", w = GUTTER as usize), style);
    }

    render_footer(f, footer, dv, theme);
}

#[allow(clippy::too_many_arguments)]
fn draw_cell(
    f: &mut Frame,
    x: u16,
    y: u16,
    width: u16,
    lineno: Option<usize>,
    text: Option<&str>,
    accent: Option<Color>,
    in_delta: bool,
    active: bool,
    cursor: bool,
    base_bg: Color,
    absent_bg: Color,
    theme: &Theme,
) {
    let w = width as usize;
    if w == 0 {
        return;
    }
    let present = text.is_some();
    let mut bg = match accent {
        Some(a) if present => mix(base_bg, a, if active { 0.42 } else { 0.24 }),
        _ if !present && in_delta => absent_bg, // the empty side of a change
        _ => base_bg,
    };
    if cursor {
        bg = mix(bg, theme.panel_fg, 0.16);
    }
    let num_style = Style::default().fg(theme.panel_border).bg(bg);
    let mut text_style = Style::default().fg(theme.panel_fg).bg(bg);
    if cursor {
        text_style = text_style.add_modifier(Modifier::BOLD);
    }

    let num = match lineno {
        Some(i) => format!("{:>4} ", i + 1),
        None => " ".repeat(LINENO_W),
    };
    f.buffer_mut().set_string(x, y, num, num_style);
    if w > LINENO_W {
        let avail = w - LINENO_W;
        let body = ellipsize(text.unwrap_or(""), avail);
        f.buffer_mut()
            .set_string(x + LINENO_W as u16, y, pad_right(&body, avail), text_style);
    }
}

fn gutter(
    row: Option<super::Row>,
    cursor: bool,
    active: bool,
    theme: &Theme,
    bg: Color,
) -> (char, Style) {
    let Some(row) = row.filter(|r| r.delta.is_some()) else {
        return (' ', Style::default().bg(bg));
    };
    if cursor {
        return (
            '►',
            Style::default().fg(theme.panel_border_active).bg(bg).add_modifier(Modifier::BOLD),
        );
    }
    let color = if active { theme.panel_border_active } else { theme.panel_border };
    let mark = match (row.left, row.right) {
        (Some(_), Some(_)) => '│',
        (Some(_), None) => '<',
        (None, Some(_)) => '>',
        (None, None) => ' ',
    };
    (mark, Style::default().fg(color).bg(bg).add_modifier(Modifier::BOLD))
}

fn render_status(f: &mut Frame, area: Rect, dv: &DiffView, theme: &Theme) {
    let n = dv.deltas.len();
    let pos = dv.active.map(|i| format!("  [{}/{n}]", i + 1)).unwrap_or_default();
    let half = ((area.width as usize).saturating_sub(12) / 2).max(4);
    let l = format!("{}{}", ellipsize(&dv.left_name, half), if dv.left_dirty { " [+]" } else { "" });
    let r = format!("{}{}", ellipsize(&dv.right_name, half), if dv.right_dirty { " [+]" } else { "" });
    let text = format!(" {l}  ⇄  {r}   {n} {}{pos} ", crate::l10n::trd("diff(s)"));
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&text, area.width as usize),
            theme.menubar.add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

fn render_footer(f: &mut Frame, area: Rect, dv: &DiffView, theme: &Theme) {
    let hint = if dv.status.is_empty() {
        "↑↓ move   Ctrl-↑↓ delta   Ctrl-← apply→left   Ctrl-→ apply→right   F2 save   Esc close"
            .to_string()
    } else {
        dv.status.clone()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            pad_right(&format!(" {hint}"), area.width as usize),
            theme.fkey_label,
        ))),
        area,
    );
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
            Color::Rgb(l(ar, br), l(ag, bg), l(ab, bb))
        }
        _ => a,
    }
}
