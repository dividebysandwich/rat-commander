//! Text input dialog.

use super::widgets::*;
use super::{DialogResult, Submit};

// ---------------------------------------------------------------------------
// Input dialog
// ---------------------------------------------------------------------------

/// What an input dialog's submitted text should be used for.
pub enum InputPurpose {
    MkDir,
    CopyDest(Vec<VfsPath>),
    MoveDest(Vec<VfsPath>),
    Compress(Vec<VfsPath>),
    /// Enter a mount point for `device` (disk mounter).
    MountPath(String),
    /// Enter a sudo password to run a queued privileged command.
    SudoPassword,
    /// Enter a sudo password to start a queued image flash.
    FlashPassword,
    /// Enter a sudo password to start a queued device-imaging.
    ImagePassword,
}

pub struct InputDialog {
    pub title: String,
    pub prompt: String,
    pub buffer: String,
    /// Caret position as a char index.
    pub cursor: usize,
    pub purpose: InputPurpose,
    /// Render the buffer masked (password entry).
    pub masked: bool,
}

impl InputDialog {
    pub fn new(
        title: impl Into<String>,
        prompt: impl Into<String>,
        initial: impl Into<String>,
        purpose: InputPurpose,
    ) -> Self {
        let buffer = initial.into();
        let cursor = buffer.chars().count();
        InputDialog {
            title: title.into(),
            prompt: prompt.into(),
            buffer,
            cursor,
            purpose,
            masked: false,
        }
    }

    /// A masked single-field prompt (for a password).
    pub fn password(title: impl Into<String>, prompt: impl Into<String>, purpose: InputPurpose) -> Self {
        InputDialog {
            title: title.into(),
            prompt: prompt.into(),
            buffer: String::new(),
            cursor: 0,
            purpose,
            masked: true,
        }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buffer.len())
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                // A password may legitimately contain anything (and be empty); the
                // path/name fields are trimmed and must be non-empty.
                if let InputPurpose::SudoPassword = self.purpose {
                    return DialogResult::Submit(Submit::SudoPassword(self.buffer.clone()));
                }
                if let InputPurpose::FlashPassword = self.purpose {
                    return DialogResult::Submit(Submit::FlashPassword(self.buffer.clone()));
                }
                if let InputPurpose::ImagePassword = self.purpose {
                    return DialogResult::Submit(Submit::ImagePassword(self.buffer.clone()));
                }
                let text = self.buffer.trim().to_string();
                if text.is_empty() {
                    return DialogResult::Cancel;
                }
                let submit = match &self.purpose {
                    InputPurpose::MkDir => Submit::MkDir(text),
                    InputPurpose::CopyDest(s) => Submit::Copy(s.clone(), text),
                    InputPurpose::MoveDest(s) => Submit::Move(s.clone(), text),
                    InputPurpose::Compress(s) => Submit::Compress(s.clone(), text),
                    InputPurpose::MountPath(device) => Submit::Mount {
                        device: device.clone(),
                        path: text,
                    },
                    InputPurpose::SudoPassword
                    | InputPurpose::FlashPassword
                    | InputPurpose::ImagePassword => unreachable!(),
                };
                DialogResult::Submit(submit)
            }
            KeyCode::Char(c) => {
                let b = self.byte_at(self.cursor);
                self.buffer.insert(b, c);
                self.cursor += 1;
                DialogResult::None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let start = self.byte_at(self.cursor - 1);
                    self.buffer.remove(start);
                    self.cursor -= 1;
                }
                DialogResult::None
            }
            KeyCode::Delete => {
                let len = self.buffer.chars().count();
                if self.cursor < len {
                    let start = self.byte_at(self.cursor);
                    self.buffer.remove(start);
                }
                DialogResult::None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Right => {
                let len = self.buffer.chars().count();
                if self.cursor < len {
                    self.cursor += 1;
                }
                DialogResult::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.cursor = self.buffer.chars().count();
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 7);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        f.render_widget(
            Paragraph::new(Line::from(self.prompt.clone()))
                .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)),
            rows[0],
        );

        let field = Rect {
            height: 1,
            ..rows[1]
        };
        if let Some(pos) =
            draw_input_field(f, field, &self.buffer, self.cursor, true, self.masked, theme)
        {
            f.set_cursor_position(pos);
        }

        let by = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        if !draw_ok_cancel(f, gfx, by, theme) {
            f.render_widget(
                Paragraph::new(ok_cancel_line(true, theme))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().bg(theme.dialog_bg)),
                by,
            );
        }
    }
}

