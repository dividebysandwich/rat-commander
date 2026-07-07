//! The command line at the bottom of the screen (above the F-key row).

use crate::ui::theme::Theme;
use crate::util::text::ellipsize;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Fallback history cap for a bare `CommandLine` (mirrors `Config::default`);
/// the app overwrites `history_max` from the loaded config.
const DEFAULT_HISTORY_MAX: usize = 100;

/// State for the persistent command line, including edit buffer and history.
#[derive(Default)]
pub struct CommandLine {
    pub buffer: String,
    /// Caret position as a char index.
    pub cursor: usize,
    pub history: Vec<String>,
    /// Maximum number of entries kept in `history` (0 = disabled); set from the
    /// `command_history_max` config value.
    pub history_max: usize,
    history_pos: Option<usize>,
}

impl CommandLine {
    pub fn new() -> Self {
        CommandLine { history_max: DEFAULT_HISTORY_MAX, ..Default::default() }
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

    /// Insert a whole string at the cursor (e.g. a filename from Alt-Enter).
    pub fn insert_str(&mut self, s: &str) {
        let b = self.byte_at(self.cursor);
        self.buffer.insert_str(b, s);
        self.cursor += s.chars().count();
        self.history_pos = None;
    }

    /// Replace the buffer with `s`, placing the caret at its end (used to recall
    /// a command from the Shell History window without running it).
    pub fn set(&mut self, s: String) {
        self.cursor = s.chars().count();
        self.buffer = s;
        self.history_pos = None;
    }

    /// Insert `arg` at the cursor as a command-line argument (Alt-Enter): a
    /// separating space is added before it when the preceding character isn't
    /// already whitespace, and a trailing space is appended so another argument
    /// can follow.
    pub fn insert_arg(&mut self, arg: &str) {
        let needs_lead = self.cursor > 0
            && self.buffer.chars().nth(self.cursor - 1).is_some_and(|c| !c.is_whitespace());
        let text = if needs_lead { format!(" {arg} ") } else { format!("{arg} ") };
        self.insert_str(&text);
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
        // Record it, collapsing an immediate repeat of the previous command so
        // the history (and Alt-P/Alt-N cycling) isn't cluttered with dupes, then
        // cap it to the configured maximum (dropping the oldest entries).
        if !cmd.trim().is_empty() && self.history.last() != Some(&cmd) {
            self.history.push(cmd.clone());
            if self.history.len() > self.history_max {
                let excess = self.history.len() - self.history_max;
                self.history.drain(..excess);
            }
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

#[cfg(test)]
mod tests {
    use super::CommandLine;

    #[test]
    fn take_records_history_and_collapses_repeats() {
        let mut c = CommandLine::new();
        c.set("ls".to_string());
        assert_eq!(c.take(), "ls");
        c.set("ls".to_string());
        c.take(); // immediate repeat is not re-recorded
        c.set("pwd".to_string());
        c.take();
        assert_eq!(c.history, vec!["ls".to_string(), "pwd".to_string()]);
        // Blank commands are never recorded.
        c.set("   ".to_string());
        c.take();
        assert_eq!(c.history.len(), 2);
    }

    #[test]
    fn ctrl_p_and_ctrl_n_cycle_history() {
        let mut c = CommandLine::new();
        for cmd in ["one", "two", "three"] {
            c.set(cmd.to_string());
            c.take();
        }
        // history_prev walks backward from the newest.
        c.history_prev();
        assert_eq!(c.buffer, "three");
        c.history_prev();
        assert_eq!(c.buffer, "two");
        c.history_prev();
        assert_eq!(c.buffer, "one");
        c.history_prev(); // clamped at the oldest
        assert_eq!(c.buffer, "one");
        // history_next walks forward, then clears past the newest.
        c.history_next();
        assert_eq!(c.buffer, "two");
        c.history_next();
        assert_eq!(c.buffer, "three");
        c.history_next();
        assert_eq!(c.buffer, "");
    }

    #[test]
    fn insert_arg_spaces_and_quotes() {
        let mut c = CommandLine::new();
        c.set("cp".to_string()); // caret at end, no trailing space
        c.insert_arg("a.txt");
        assert_eq!(c.buffer, "cp a.txt "); // leading + trailing space
        c.insert_arg("b.txt");
        assert_eq!(c.buffer, "cp a.txt b.txt "); // no double space after a space
        assert_eq!(c.cursor, c.buffer.chars().count());
    }

    #[test]
    fn history_is_capped_at_history_max() {
        let mut c = CommandLine::new();
        c.history_max = 3;
        for cmd in ["a", "b", "c", "d", "e"] {
            c.set(cmd.to_string());
            c.take();
        }
        // Only the 3 most recent survive; the oldest were dropped.
        assert_eq!(c.history, vec!["c".to_string(), "d".to_string(), "e".to_string()]);
        // A max of 0 disables history entirely.
        let mut c = CommandLine::new();
        c.history_max = 0;
        c.set("x".to_string());
        c.take();
        assert!(c.history.is_empty());
    }

    #[test]
    fn set_places_caret_at_end() {
        let mut c = CommandLine::new();
        c.set("echo hi".to_string());
        assert_eq!(c.cursor, 7);
        assert_eq!(c.buffer, "echo hi");
    }
}
