//! Overwrite-confirmation dialog (shown mid-copy when a destination exists).

use super::widgets::*;
use super::DialogResult;
use crate::ops::progress::{ConflictInfo, OverwriteDecision, OverwriteRule};

// ---------------------------------------------------------------------------
// Overwrite-confirmation dialog (shown mid-copy when a destination exists)
// ---------------------------------------------------------------------------

/// The interactive controls of the overwrite dialog, in focus order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwControl {
    Yes,
    No,
    Append,
    SkipEmpty,
    All,
    Older,
    NoneRule,
    Smaller,
    SizeDiffers,
    Abort,
}

const OW_ORDER: [OwControl; 10] = [
    OwControl::Yes,
    OwControl::No,
    OwControl::Append,
    OwControl::SkipEmpty,
    OwControl::All,
    OwControl::Older,
    OwControl::NoneRule,
    OwControl::Smaller,
    OwControl::SizeDiffers,
    OwControl::Abort,
];

/// A red "File exists" prompt offering per-file (Yes/No/Append) and global
/// (All/Older/None/Smaller/Size differs) overwrite choices, plus Abort.
pub struct OverwriteDialog {
    info: ConflictInfo,
    focus: usize,
    skip_empty: bool,
    /// Clickable control regions, recorded during render.
    zones: Vec<(Rect, OwControl)>,
}

impl OverwriteDialog {
    pub fn new(info: ConflictInfo) -> Self {
        OverwriteDialog {
            info,
            focus: 0,
            skip_empty: false,
            zones: Vec::new(),
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let len = OW_ORDER.len();
        match key.code {
            KeyCode::Esc => self.activate(OwControl::Abort),
            KeyCode::Enter => self.activate(OW_ORDER[self.focus]),
            KeyCode::Char(' ') => {
                if OW_ORDER[self.focus] == OwControl::SkipEmpty {
                    self.skip_empty = !self.skip_empty;
                }
                DialogResult::None
            }
            KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                self.focus = (self.focus + len - 1) % len;
                DialogResult::None
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                self.focus = (self.focus + 1) % len;
                DialogResult::None
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => self.activate(OwControl::Yes),
            KeyCode::Char('n') | KeyCode::Char('N') => self.activate(OwControl::No),
            KeyCode::Char('p') | KeyCode::Char('P') => self.activate(OwControl::Append),
            _ => DialogResult::None,
        }
    }

    /// Hit-test a mouse click against the recorded control zones.
    pub fn handle_click(&mut self, col: u16, row: u16) -> DialogResult {
        if let Some(&(_, ctrl)) = self
            .zones
            .iter()
            .find(|(r, _)| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
        {
            // Move focus to the clicked control, then activate it.
            if let Some(i) = OW_ORDER.iter().position(|c| *c == ctrl) {
                self.focus = i;
            }
            return self.activate(ctrl);
        }
        DialogResult::None
    }

    fn activate(&mut self, ctrl: OwControl) -> DialogResult {
        let id = self.info.id;
        let decision = |d: OverwriteDecision| DialogResult::Overwrite(id, d);
        let policy = |rule: OverwriteRule, skip_empty: bool| {
            DialogResult::Overwrite(id, OverwriteDecision::Policy { rule, skip_empty })
        };
        match ctrl {
            OwControl::Yes => decision(OverwriteDecision::OverwriteOnce),
            OwControl::No => decision(OverwriteDecision::SkipOnce),
            OwControl::Append => decision(OverwriteDecision::AppendOnce),
            OwControl::SkipEmpty => {
                self.skip_empty = !self.skip_empty;
                DialogResult::None
            }
            OwControl::All => policy(OverwriteRule::All, self.skip_empty),
            OwControl::Older => policy(OverwriteRule::Older, self.skip_empty),
            OwControl::NoneRule => policy(OverwriteRule::None, self.skip_empty),
            OwControl::Smaller => policy(OverwriteRule::Smaller, self.skip_empty),
            OwControl::SizeDiffers => policy(OverwriteRule::SizeDiffers, self.skip_empty),
            OwControl::Abort => decision(OverwriteDecision::Abort),
        }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.zones.clear();

        // A red (warning) box: white text on the theme's error color.
        let bg = theme.error_fg;
        let fg = ratatui::style::Color::White;
        let base = Style::default().fg(fg).bg(bg);

        let w = 60u16.min(area.width.saturating_sub(2));
        let h = 15u16.min(area.height.saturating_sub(2));
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(base.add_modifier(Modifier::BOLD))
            .title(Span::styled(
                " File exists ",
                base.add_modifier(Modifier::BOLD),
            ))
            .title_alignment(ratatui::layout::Alignment::Center)
            .style(base);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        if inner.width < 10 || inner.height < 10 {
            return;
        }

        let mut y = inner.y;
        let name_w = inner.width as usize;
        ow_meta_line(f, inner, y, &format!("New     : {}", crate::util::text::ellipsize(&self.info.new_path, name_w.saturating_sub(10))), base);
        y += 1;
        ow_meta_line(f, inner, y, &ow_meta(self.info.new_size, self.info.new_mtime), base);
        y += 1;
        ow_meta_line(f, inner, y, &format!("Existing: {}", crate::util::text::ellipsize(&self.info.old_path, name_w.saturating_sub(10))), base);
        y += 1;
        ow_meta_line(f, inner, y, &ow_meta(self.info.old_size, self.info.old_mtime), base);
        y += 1;
        self.rule(f, inner, y, bg);
        y += 1;

        // Per-file row.
        ow_center(f, inner, y, "Overwrite this file?", base.add_modifier(Modifier::BOLD));
        y += 1;
        self.button_row(
            f,
            inner,
            y,
            &[
                (" Yes ", OwControl::Yes),
                (" No ", OwControl::No),
                (" Append ", OwControl::Append),
            ],
            theme,
        );
        y += 1;
        self.rule(f, inner, y, bg);
        y += 1;

        // Global row.
        ow_center(f, inner, y, "Overwrite all files?", base.add_modifier(Modifier::BOLD));
        y += 1;
        self.checkbox_row(f, inner, y, "Don't overwrite with zero length file", theme);
        y += 1;
        self.button_row(
            f,
            inner,
            y,
            &[
                (" All ", OwControl::All),
                (" Older ", OwControl::Older),
                (" None ", OwControl::NoneRule),
                (" Smaller ", OwControl::Smaller),
                (" Size differs ", OwControl::SizeDiffers),
            ],
            theme,
        );
        y += 1;
        self.rule(f, inner, y, bg);
        y += 1;

        self.button_row(f, inner, y, &[(" Abort ", OwControl::Abort)], theme);
    }

    fn rule(&self, f: &mut Frame, inner: Rect, y: u16, bg: ratatui::style::Color) {
        if y >= inner.y + inner.height {
            return;
        }
        let style = Style::default().fg(ratatui::style::Color::White).bg(bg);
        f.buffer_mut()
            .set_string(inner.x, y, "─".repeat(inner.width as usize), style);
    }

    /// Render a centered row of bracketed buttons and record their click zones.
    fn button_row(
        &mut self,
        f: &mut Frame,
        inner: Rect,
        y: u16,
        buttons: &[(&str, OwControl)],
        theme: &Theme,
    ) {
        if y >= inner.y + inner.height {
            return;
        }
        let bg = theme.error_fg;
        // Each label is wrapped as "[label]"; buttons separated by one space.
        let labels: Vec<String> = buttons.iter().map(|(l, _)| format!("[{l}]")).collect();
        let total: usize = labels.iter().map(|l| l.chars().count()).sum::<usize>() + labels.len().saturating_sub(1);
        let mut x = inner.x + (inner.width.saturating_sub(total as u16)) / 2;
        for (label, (_, ctrl)) in labels.iter().zip(buttons.iter()) {
            let focused = OW_ORDER[self.focus] == *ctrl;
            let style = if focused {
                Style::default()
                    .fg(bg)
                    .bg(ratatui::style::Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(ratatui::style::Color::White)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            };
            let wlen = label.chars().count() as u16;
            f.buffer_mut().set_string(x, y, label, style);
            self.zones.push((Rect { x, y, width: wlen, height: 1 }, *ctrl));
            x += wlen + 1;
        }
    }

    fn checkbox_row(&mut self, f: &mut Frame, inner: Rect, y: u16, label: &str, theme: &Theme) {
        if y >= inner.y + inner.height {
            return;
        }
        let bg = theme.error_fg;
        let focused = OW_ORDER[self.focus] == OwControl::SkipEmpty;
        let mark = if self.skip_empty { "[x] " } else { "[ ] " };
        let text = format!("{mark}{label}");
        let style = if focused {
            Style::default().fg(bg).bg(ratatui::style::Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ratatui::style::Color::White).bg(bg)
        };
        let wlen = text.chars().count() as u16;
        let x = inner.x + (inner.width.saturating_sub(wlen)) / 2;
        f.buffer_mut().set_string(x, y, &text, style);
        self.zones.push((Rect { x, y, width: wlen, height: 1 }, OwControl::SkipEmpty));
    }
}

/// Format a "size + date" detail line for the overwrite dialog.
fn ow_meta(size: u64, mtime: Option<std::time::SystemTime>) -> String {
    let date = mtime.map(format_time).unwrap_or_default();
    format!("{size:>14}      {date}")
}

/// Render a left-aligned detail line within the dialog interior.
fn ow_meta_line(f: &mut Frame, inner: Rect, y: u16, text: &str, style: Style) {
    if y >= inner.y + inner.height {
        return;
    }
    let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
    f.render_widget(Paragraph::new(Span::styled(text.to_string(), style)), row);
}

/// Render a centered label line within the dialog interior.
fn ow_center(f: &mut Frame, inner: Rect, y: u16, text: &str, style: Style) {
    if y >= inner.y + inner.height {
        return;
    }
    let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
    f.render_widget(
        Paragraph::new(Span::styled(text.to_string(), style))
            .alignment(ratatui::layout::Alignment::Center),
        row,
    );
}





