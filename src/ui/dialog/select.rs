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

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => {
                if self.pattern.trim().is_empty() {
                    return DialogResult::Cancel;
                }
                return DialogResult::Submit(Submit::Select {
                    select: self.select,
                    pattern: self.pattern.clone(),
                    files_only: self.files_only,
                    case_sensitive: self.case_sensitive,
                    shell: self.shell,
                });
            }
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

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = if self.select { "Select" } else { "Unselect" };
        let rect = centered(area, 54u16.min(area.width.saturating_sub(2)), 7);
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

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

