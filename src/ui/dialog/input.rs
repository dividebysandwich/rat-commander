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
    /// Enter a root password for the network explorer (blank ⇒ user mode).
    NetworkPassword,
    /// Set the persistent listing filter on panel `side` (blank clears it).
    PanelFilter(usize),
    /// Answer a `%{…}` prompt of a user-menu command (any text, blank allowed).
    MenuPrompt,
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
    /// The pre-filled text is fully marked: the next inserted character (or a
    /// Backspace/Delete) replaces the whole buffer, mimicking GUI selection.
    /// Any cursor movement clears the mark without deleting.
    pub selected: bool,
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
        let selected = !buffer.is_empty();
        InputDialog {
            title: title.into(),
            prompt: prompt.into(),
            buffer,
            cursor,
            purpose,
            masked: false,
            selected,
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
            selected: false,
        }
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
                // A blank network password is valid (means "user mode").
                if let InputPurpose::NetworkPassword = self.purpose {
                    return DialogResult::Submit(Submit::NetworkPassword(self.buffer.clone()));
                }
                // A blank filter is valid too — it clears the panel filter.
                if let InputPurpose::PanelFilter(side) = self.purpose {
                    return DialogResult::Submit(Submit::PanelFilter {
                        side,
                        pattern: self.buffer.trim().to_string(),
                    });
                }
                // A `%{…}` menu prompt substitutes exactly what was typed (which
                // may legitimately be empty), verbatim — no trimming or quoting.
                if let InputPurpose::MenuPrompt = self.purpose {
                    return DialogResult::Submit(Submit::MenuPrompt(self.buffer.clone()));
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
                    | InputPurpose::ImagePassword
                    | InputPurpose::NetworkPassword
                    | InputPurpose::PanelFilter(_)
                    | InputPurpose::MenuPrompt => unreachable!(),
                };
                DialogResult::Submit(submit)
            }
            // Everything else is full Emacs/readline editing (shared with the
            // command line and every other input), honouring the select-all mark.
            _ => {
                edit_text_marked(&mut self.buffer, &mut self.cursor, &mut self.selected, key);
                DialogResult::None
            }
        }
    }

    #[cfg(test)]
    fn buffer_cursor(&self) -> (&str, usize) {
        (&self.buffer, self.cursor)
    }

    /// Route a click onto the text field: drop the select-all mark and place the
    /// caret under the pointer. The OK/Cancel row is left to the generic dialog
    /// button handler. Returns `Some` when the field row was hit.
    pub(crate) fn click_field(&mut self, area: Rect, col: u16, row: u16) -> Option<DialogResult> {
        let rect = centered(area, 60u16.min(area.width.saturating_sub(4)), 7);
        let inner = Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        };
        // The input sits on the second interior row (below the prompt label).
        if row == inner.y + 1 && col >= inner.x && col < inner.x + inner.width {
            self.selected = false;
            self.cursor = (col.saturating_sub(inner.x) as usize).min(self.buffer.chars().count());
            return Some(DialogResult::None);
        }
        None
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 7);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd(&self.title), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        f.render_widget(
            Paragraph::new(Line::from(crate::l10n::trd(&self.prompt)))
                .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)),
            rows[0],
        );

        let field = Rect {
            height: 1,
            ..rows[1]
        };
        if let Some(pos) = draw_input_field_ex(
            f,
            field,
            &self.buffer,
            self.cursor,
            true,
            self.masked,
            self.selected,
            theme,
        ) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn input_dialog_supports_readline_editing() {
        let mut d = InputDialog::new("t", "p", "hello", InputPurpose::MkDir);
        // Pre-filled text opens fully marked; a cursor motion drops the mark
        // without clearing (readline C-A goes to the start).
        d.handle_key(ctrl('a'));
        assert_eq!(d.buffer_cursor(), ("hello", 0));
        assert!(!d.selected, "a readline motion drops the select-all mark");
        // C-E to the end, then C-K kills to end of line.
        d.handle_key(ctrl('e'));
        assert_eq!(d.buffer_cursor(), ("hello", 5));
        d.handle_key(ctrl('a'));
        d.handle_key(ctrl('k'));
        assert_eq!(d.buffer_cursor(), ("", 0));
        // Yank it back.
        d.handle_key(ctrl('y'));
        assert_eq!(d.buffer_cursor(), ("hello", 5));
    }
}

