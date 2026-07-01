//! Find-file dialog.

use super::widgets::*;
use super::{DialogResult, Submit};

// ---------------------------------------------------------------------------
// Find-file dialog
// ---------------------------------------------------------------------------

/// Result of the find-file dialog.
#[derive(Debug, Clone)]
pub struct FindParams {
    pub start_at: String,
    pub file_name: String,
    pub content: String,
    pub recursive: bool,
    pub case_sensitive: bool,
    pub skip_hidden: bool,
    pub shell: bool,
}

pub struct FindDialog {
    start_at: String,
    start_cursor: usize,
    file_name: String,
    name_cursor: usize,
    content: String,
    content_cursor: usize,
    recursive: bool,
    case_sensitive: bool,
    skip_hidden: bool,
    shell: bool,
    focus: usize, // 0 start, 1 name, 2 content, 3..6 checks
}

impl FindDialog {
    pub fn new(start_at: String) -> Self {
        let start_cursor = start_at.chars().count();
        FindDialog {
            start_at,
            start_cursor,
            file_name: "*".to_string(),
            name_cursor: 1,
            content: String::new(),
            content_cursor: 0,
            recursive: true,
            case_sensitive: false,
            skip_hidden: true,
            shell: true,
            focus: 1,
        }
    }

    const FOCUS_COUNT: usize = 7;

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => {
                if self.file_name.trim().is_empty() {
                    return DialogResult::Cancel;
                }
                return DialogResult::Submit(Submit::Find(FindParams {
                    start_at: self.start_at.clone(),
                    file_name: self.file_name.clone(),
                    content: self.content.clone(),
                    recursive: self.recursive,
                    case_sensitive: self.case_sensitive,
                    skip_hidden: self.skip_hidden,
                    shell: self.shell,
                }));
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % Self::FOCUS_COUNT,
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = (self.focus + Self::FOCUS_COUNT - 1) % Self::FOCUS_COUNT
            }
            KeyCode::Char(' ') if self.focus >= 3 => match self.focus {
                3 => self.recursive = !self.recursive,
                4 => self.case_sensitive = !self.case_sensitive,
                5 => self.skip_hidden = !self.skip_hidden,
                6 => self.shell = !self.shell,
                _ => {}
            },
            _ => match self.focus {
                0 => edit_text(&mut self.start_at, &mut self.start_cursor, key),
                1 => edit_text(&mut self.file_name, &mut self.name_cursor, key),
                2 => edit_text(&mut self.content, &mut self.content_cursor, key),
                _ => {}
            },
        }
        DialogResult::None
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let rect = centered(area, 66u16.min(area.width.saturating_sub(2)), 13);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("Find File", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let line_at = |yy: u16| Rect { x: inner.x, y: yy, width: inner.width, height: 1 };
        let mut caret = None;
        let mut y = inner.y;

        f.render_widget(Paragraph::new(Span::styled("Start at:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.start_at, self.start_cursor, self.focus == 0, false, theme,
        ) {
            caret = Some(p);
        }
        y += 2;

        f.render_widget(Paragraph::new(Span::styled("File name:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.file_name, self.name_cursor, self.focus == 1, false, theme,
        ) {
            caret = Some(p);
        }
        y += 1;
        f.render_widget(Paragraph::new(Span::styled("Content:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.content, self.content_cursor, self.focus == 2, false, theme,
        ) {
            caret = Some(p);
        }
        y += 2;

        // Checkboxes in two columns.
        let half = inner.width / 2;
        f.render_widget(
            Paragraph::new(Line::from(check_span("Find recursively", self.recursive, self.focus == 3, theme))).style(base),
            Rect { x: inner.x, y, width: half, height: 1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span("Case sensitive", self.case_sensitive, self.focus == 4, theme))).style(base),
            Rect { x: inner.x + half, y, width: inner.width - half, height: 1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span("Skip hidden", self.skip_hidden, self.focus == 5, theme))).style(base),
            Rect { x: inner.x, y: y + 1, width: half, height: 1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span("Using shell patterns", self.shell, self.focus == 6, theme))).style(base),
            Rect { x: inner.x + half, y: y + 1, width: inner.width - half, height: 1 },
        );

        let by = inner.y + inner.height - 1;
        if !draw_ok_cancel(f, gfx, line_at(by), theme) {
            f.render_widget(
                Paragraph::new(ok_cancel_line(true, theme))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(base),
                line_at(by),
            );
        }

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}
