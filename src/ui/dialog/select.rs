//! Select / unselect-group dialog.

use super::widgets::*;
use super::{DialogResult, Submit};

// ---------------------------------------------------------------------------
// Select / unselect-group dialog
// ---------------------------------------------------------------------------

pub struct SelectDialog {
    select: bool,
    pattern: String,
    cursor: usize,
    files_only: bool,
    case_sensitive: bool,
    shell: bool,
    focus: usize, // 0 pattern, 1 files_only, 2 case, 3 shell
}

impl SelectDialog {
    pub fn new(select: bool) -> Self {
        SelectDialog {
            select,
            pattern: "*".to_string(),
            cursor: 1,
            files_only: false,
            case_sensitive: true,
            shell: true,
            focus: 0,
        }
    }

    /// Build the submit result, or `Cancel` when the pattern is blank.
    fn submit(&self) -> DialogResult {
        if self.pattern.trim().is_empty() {
            return DialogResult::Cancel;
        }
        DialogResult::Submit(Submit::Select {
            select: self.select,
            pattern: self.pattern.clone(),
            files_only: self.files_only,
            case_sensitive: self.case_sensitive,
            shell: self.shell,
        })
    }

    /// The centered outer box (kept in sync with `render`), for click geometry.
    /// Height 8 leaves a blank row between the checkboxes and the button row.
    fn box_rect(&self, area: Rect) -> Rect {
        centered(area, 54u16.min(area.width.saturating_sub(2)), 8)
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => return self.submit(),
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % 4,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + 3) % 4,
            KeyCode::Char(' ') if self.focus > 0 => match self.focus {
                1 => self.files_only = !self.files_only,
                2 => self.case_sensitive = !self.case_sensitive,
                3 => self.shell = !self.shell,
                _ => {}
            },
            _ if self.focus == 0 => edit_text(&mut self.pattern, &mut self.cursor, key),
            _ => {}
        }
        DialogResult::None
    }

    /// Route a left click: focus/position the pattern field, tick a checkbox, or
    /// press OK/Cancel. The layout mirrors `render`.
    pub(crate) fn handle_click(&mut self, area: Rect, col: u16, row: u16) -> DialogResult {
        let rect = self.box_rect(area);
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        let in_x = col >= inner.x && col < inner.x + inner.width;
        // Pattern field (top interior row): focus it and place the caret.
        if row == inner.y && in_x {
            self.focus = 0;
            let inner_w = (inner.width as usize).saturating_sub(3);
            let start = self.cursor.saturating_sub(inner_w.saturating_sub(1));
            let char_count = self.pattern.chars().count();
            self.cursor = (start + (col - inner.x) as usize).min(char_count);
            return DialogResult::None;
        }
        // Checkbox rows: "Files only" | "Case sensitive" on one row, "Using shell
        // patterns" on the next.
        let half = inner.width / 2;
        if row == inner.y + 2 && in_x {
            if col < inner.x + half {
                self.files_only = !self.files_only;
                self.focus = 1;
            } else {
                self.case_sensitive = !self.case_sensitive;
                self.focus = 2;
            }
            return DialogResult::None;
        }
        if row == inner.y + 3 && in_x {
            self.shell = !self.shell;
            self.focus = 3;
            return DialogResult::None;
        }
        // OK / Cancel on the last interior row (left half OK, right half Cancel).
        if row == inner.y + inner.height.saturating_sub(1) && in_x {
            return if col < inner.x + inner.width / 2 { self.submit() } else { DialogResult::Cancel };
        }
        DialogResult::None
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let title = if self.select { "Select" } else { "Unselect" };
        let rect = self.box_rect(area);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd(title), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let mut caret = None;
        let field = Rect { height: 1, ..inner };
        if let Some(p) =
            draw_input_field(f, field, &self.pattern, self.cursor, self.focus == 0, false, theme)
        {
            caret = Some(p);
        }

        let half = inner.width / 2;
        let r1 = Rect { y: inner.y + 2, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(Line::from(check_span(&crate::l10n::trd("Files only"), self.files_only, self.focus == 1, theme)))
                .style(Style::default().bg(theme.dialog_bg)),
            Rect { width: half, ..r1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span(
                &crate::l10n::trd("Case sensitive"),
                self.case_sensitive,
                self.focus == 2,
                theme,
            )))
            .style(Style::default().bg(theme.dialog_bg)),
            Rect { x: inner.x + half, width: inner.width - half, ..r1 },
        );
        let r2 = Rect { y: inner.y + 3, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(Line::from(check_span(
                &crate::l10n::trd("Using shell patterns"),
                self.shell,
                self.focus == 3,
                theme,
            )))
            .style(Style::default().bg(theme.dialog_bg)),
            r2,
        );

        // OK / Cancel on the last interior row (a blank row sits above it), as
        // graphical buttons when available, else a text button line.
        let by = Rect { y: inner.y + inner.height.saturating_sub(1), height: 1, ..inner };
        if !draw_ok_cancel(f, gfx, by, theme) {
            f.render_widget(
                Paragraph::new(ok_cancel_line(true, theme))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().bg(theme.dialog_bg)),
                by,
            );
        }

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

