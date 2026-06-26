//! The command line at the bottom of the screen (above the F-key row).

use crate::ui::theme::Theme;
use crate::util::text::ellipsize;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// State for the persistent command line, including edit buffer and history.
#[derive(Default)]
pub struct CommandLine {
    pub buffer: String,
    /// Caret position as a char index.
    pub cursor: usize,
    pub history: Vec<String>,
    history_pos: Option<usize>,
}

impl CommandLine {
    pub fn new() -> Self {
        CommandLine::default()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.trim().is_empty()
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buffer.len())
    }

    pub fn insert(&mut self, c: char) {
        let b = self.byte_at(self.cursor);
        self.buffer.insert(b, c);
        self.cursor += 1;
        self.history_pos = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let start = self.byte_at(self.cursor - 1);
            self.buffer.remove(start);
            self.cursor -= 1;
        }
    }

    pub fn delete(&mut self) {
        let len = self.buffer.chars().count();
        if self.cursor < len {
            let start = self.byte_at(self.cursor);
            self.buffer.remove(start);
        }
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        let len = self.buffer.chars().count();
        if self.cursor < len {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buffer.chars().count();
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.history_pos = None;
    }

    /// Take the current command, push it to history, and clear the buffer.
    pub fn take(&mut self) -> String {
        let cmd = std::mem::take(&mut self.buffer);
        self.cursor = 0;
        self.history_pos = None;
        if !cmd.trim().is_empty() {
            self.history.push(cmd.clone());
        }
        cmd
    }

    /// Recall the previous history entry into the buffer.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = match self.history_pos {
            Some(0) => 0,
            Some(p) => p - 1,
            None => self.history.len() - 1,
        };
        self.history_pos = Some(pos);
        self.buffer = self.history[pos].clone();
        self.cursor = self.buffer.chars().count();
    }

    /// Recall the next history entry (toward the present).
    pub fn history_next(&mut self) {
        match self.history_pos {
            Some(p) if p + 1 < self.history.len() => {
                self.history_pos = Some(p + 1);
                self.buffer = self.history[p + 1].clone();
                self.cursor = self.buffer.chars().count();
            }
            _ => {
                self.history_pos = None;
                self.clear();
            }
        }
    }
}

/// Render the command line. `cwd` is shown as the prompt; returns the caret
/// screen position so the caller can show the cursor when the panel has focus.
pub fn render(
    f: &mut Frame,
    area: Rect,
    cmd: &CommandLine,
    cwd: &str,
    theme: &Theme,
) -> Position {
    let prompt = format!("{}$ ", ellipsize(cwd, (area.width as usize).saturating_sub(12)));
    let line = Line::from(vec![
        Span::styled(prompt.clone(), Style::default().fg(theme.panel_border_active)),
        Span::raw(cmd.buffer.clone()),
    ]);
    f.render_widget(Paragraph::new(line), area);
    let prompt_w = prompt.chars().count() as u16;
    Position::new(area.x + prompt_w + cmd.cursor as u16, area.y)
}
