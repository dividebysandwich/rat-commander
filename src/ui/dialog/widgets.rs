//! Shared dialog helpers and reusable styled widgets.
//!
//! These functions are used across the individual dialog submodules; the
//! `pub(crate) use` re-exports below let each submodule pull in the common
//! external symbols with a single `use super::widgets::*;`.

pub(crate) use crate::ui::theme::Theme;
pub(crate) use ratatui::Frame;
pub(crate) use ratatui::crossterm::event::{KeyCode, KeyEvent};
pub(crate) use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
pub(crate) use ratatui::style::{Modifier, Style};
pub(crate) use ratatui::text::{Line, Span};
pub(crate) use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
pub(crate) use crate::util::bytes::{format_time, human_size};
pub(crate) use crate::util::text::{ellipsize, pad_right};
pub(crate) use crate::vfs::VfsPath;

use super::form::Field;

/// Set a text field's value (and place the cursor at the end).
pub(crate) fn set_text_field(field: &mut Field, val: &str) {
    if let Field::Text { value, cursor, .. } = field {
        *value = val.to_string();
        *cursor = value.chars().count();
    }
}

/// Apply a single editing key to a text buffer + char cursor.
pub(crate) fn edit_text(value: &mut String, cursor: &mut usize, key: KeyEvent) {
    let byte_at = |s: &str, idx: usize| {
        s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len())
    };
    match key.code {
        KeyCode::Char(c) => {
            let b = byte_at(value, *cursor);
            value.insert(b, c);
            *cursor += 1;
        }
        KeyCode::Backspace => {
            if *cursor > 0 {
                let b = byte_at(value, *cursor - 1);
                value.remove(b);
                *cursor -= 1;
            }
        }
        KeyCode::Delete => {
            if *cursor < value.chars().count() {
                let b = byte_at(value, *cursor);
                value.remove(b);
            }
        }
        KeyCode::Left => *cursor = cursor.saturating_sub(1),
        KeyCode::Right => {
            if *cursor < value.chars().count() {
                *cursor += 1;
            }
        }
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = value.chars().count(),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Draw a drop shadow for a dialog box: a dim band one cell below and to the
/// right of `rect`. Out-of-screen cells are clipped by the renderer.
pub(crate) fn draw_shadow(f: &mut Frame, rect: Rect, _theme: &Theme) {
    let shadow = Style::default().bg(ratatui::style::Color::Rgb(8, 8, 12));
    // Bottom edge (offset right by 1 so it sits under the box).
    let bottom = Rect {
        x: rect.x + 1,
        y: rect.y + rect.height,
        width: rect.width,
        height: 1,
    };
    // Right edge (offset down by 1).
    let right = Rect {
        x: rect.x + rect.width,
        y: rect.y + 1,
        width: 1,
        height: rect.height,
    };
    f.render_widget(Block::default().style(shadow), bottom);
    f.render_widget(Block::default().style(shadow), right);
}

/// A progress bar whose filled portion shows a gradient "pulse" sweeping left to
/// right (truecolor only; otherwise a solid fill). `label` is centered over it.
pub(crate) fn pulse_gauge(f: &mut Frame, area: Rect, ratio: f64, label: &str, base: ratatui::style::Color, theme: &Theme) {
    let w = area.width as usize;
    if w == 0 || area.height == 0 {
        return;
    }
    let filled = (ratio.clamp(0.0, 1.0) * w as f64).round() as usize;

    // Center the label over the bar.
    let label: Vec<char> = label.chars().take(w).collect();
    let lstart = (w - label.len()) / 2;

    let empty_fg = theme.panel_border;
    let buf = f.buffer_mut();
    for x in 0..w {
        let in_label = x >= lstart && x < lstart + label.len();
        let lc = if in_label { Some(label[x - lstart]) } else { None };
        if x < filled {
            let color = pulse_fill(theme, base, x, w);
            let (ch, fg, bg) = match lc {
                Some(c) => (c, theme.dialog_bg, color),
                None => ('█', color, theme.dialog_bg),
            };
            buf.set_string(area.x + x as u16, area.y, ch.to_string(), Style::default().fg(fg).bg(bg));
        } else {
            let (ch, fg) = match lc {
                Some(c) => (c, theme.dialog_fg),
                None => ('░', empty_fg),
            };
            buf.set_string(
                area.x + x as u16,
                area.y,
                ch.to_string(),
                Style::default().fg(fg).bg(theme.dialog_bg),
            );
        }
    }
}

/// Linearly blend two RGB colors: `t`=0 → `a`, `t`=1 → `b`. Non-RGB inputs
/// fall back to `b`.
pub(crate) fn mix_rgb(a: ratatui::style::Color, b: ratatui::style::Color, t: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
            Color::Rgb(f(ar, br), f(ag, bg), f(ab, bb))
        }
        _ => b,
    }
}

/// Color of filled cell `x` (of `w`) in an animated pulse bar over `base`: a
/// bright band sweeps left→right as `theme.anim` advances (truecolor only;
/// otherwise the solid `base`). Shared by the copy gauges and the disk scan bar
/// so they pulse identically.
pub(crate) fn pulse_fill(
    theme: &Theme,
    base: ratatui::style::Color,
    x: usize,
    w: usize,
) -> ratatui::style::Color {
    if !theme.truecolor {
        return base;
    }
    let band = (w as f64 * 0.33).max(5.0);
    let period = w as f64 + band;
    let pos = (theme.anim as f64 * 3.2) % period;
    let t = (1.0 - (x as f64 - pos).abs() / (band * 0.5)).clamp(0.0, 1.0);
    pulse_color(base, t)
}

/// Perceived brightness (Rec. 601 luma, 0..255) of an RGB color; 128 for
/// non-RGB colors.
pub(crate) fn luma(c: ratatui::style::Color) -> f32 {
    if let ratatui::style::Color::Rgb(r, g, b) = c {
        0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
    } else {
        128.0
    }
}

/// Brighten `base` toward a white-hot highlight by pulse intensity `t` (0..1).
pub(crate) fn pulse_color(base: ratatui::style::Color, t: f64) -> ratatui::style::Color {
    if let ratatui::style::Color::Rgb(r, g, b) = base {
        let bright = 0.5 + 0.5 * t; // 0.5×..1.0× brightness
        let hl = t * t * 110.0; // white highlight near the pulse center
        let mix = |c: u8| ((c as f64 * bright) + hl).min(255.0) as u8;
        ratatui::style::Color::Rgb(mix(r), mix(g), mix(b))
    } else {
        base
    }
}

/// A rectangle of fixed size centered within `area`.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

pub(crate) fn dialog_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.dialog_title).bg(theme.dialog_bg))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.dialog_title)
                .bg(theme.dialog_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg))
}

/// Like [`dialog_block`] but drawn in a loud red to flag a dangerous action.
pub(crate) fn danger_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.error_fg).bg(theme.dialog_bg))
        .title(Span::styled(
            format!(" ⚠ {title} ⚠ "),
            Style::default()
                .fg(theme.error_fg)
                .bg(theme.dialog_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg))
}

pub(crate) fn button(text: &str, focused: bool, theme: &Theme) -> Span<'static> {
    let style = if focused {
        theme.button_focused
    } else {
        theme.button
    };
    Span::styled(text.to_string(), style)
}

// --- Reusable styled widgets matching the mc dialog look -------------------

/// Draw a turquoise input field with a trailing `[^]` history button. Returns
/// the caret screen position when `focused`.
pub(crate) fn draw_input_field(
    f: &mut Frame,
    area: Rect,
    value: &str,
    cursor: usize,
    focused: bool,
    masked: bool,
    theme: &Theme,
) -> Option<Position> {
    let total = area.width as usize;
    if total < 4 {
        return None;
    }
    let inner_w = total - 3; // leave room for "[^]"
    let field_style = Style::default().fg(theme.input_fg).bg(theme.input_bg);

    // Horizontal scroll so the caret stays visible.
    let char_count = value.chars().count();
    let start = cursor.saturating_sub(inner_w.saturating_sub(1));
    let shown: String = if masked {
        "*".repeat(char_count)
    } else {
        value.chars().collect()
    };
    let shown: String = shown.chars().skip(start).take(inner_w).collect();
    let mut padded = shown.clone();
    while padded.chars().count() < inner_w {
        padded.push(' ');
    }
    let line = Line::from(vec![
        Span::styled(padded, field_style),
        Span::styled(
            "[^]",
            Style::default().fg(theme.dialog_title).bg(theme.input_bg),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);

    if focused {
        let cx = area.x + (cursor - start).min(inner_w.saturating_sub(1)) as u16;
        Some(Position::new(cx, area.y))
    } else {
        None
    }
}

/// A `(*) Label` / `( ) Label` radio span.
pub(crate) fn radio_span(label: &str, selected: bool, focused: bool, theme: &Theme) -> Span<'static> {
    let mark = if selected { "(*) " } else { "( ) " };
    let style = if focused {
        theme.dialog_selection
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
    };
    Span::styled(format!("{mark}{label}"), style)
}

/// A `[x] Label` / `[ ] Label` checkbox span.
pub(crate) fn check_span(label: &str, checked: bool, focused: bool, theme: &Theme) -> Span<'static> {
    let mark = if checked { "[x] " } else { "[ ] " };
    let style = if focused {
        theme.dialog_selection
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
    };
    Span::styled(format!("{mark}{label}"), style)
}

/// The `[< OK >]   [ Cancel ]` button row.
pub(crate) fn ok_cancel_line(focus_ok: bool, theme: &Theme) -> Line<'static> {
    let ok = if focus_ok {
        Span::styled("[< OK >]", theme.button_focused)
    } else {
        Span::styled("[  OK  ]", theme.button)
    };
    let cancel = if focus_ok {
        Span::styled("[ Cancel ]", theme.button)
    } else {
        Span::styled("[< Cancel >]", theme.button_focused)
    };
    Line::from(vec![ok, Span::styled("   ", Style::default().bg(theme.dialog_bg)), cancel])
}

