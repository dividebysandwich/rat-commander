//! Internal `mcedit`-style text editor.
//!
//! The editor is a full-screen overlay (like the viewer). It owns an
//! [`EditorBuffer`] and all cursor/selection state; the app handles only the
//! async file save when the editor asks for it.

pub mod buffer;
pub mod render;

use crate::vfs::VfsPath;
use buffer::EditorBuffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the app should do after the editor handles a key.
pub enum EditorSignal {
    Stay,
    Close,
    /// Persist the buffer; close the editor afterwards if `close_after`.
    Save { close_after: bool },
    /// The buffer is modified and the user asked to quit: the app should show a
    /// modal save/discard/cancel confirmation.
    ConfirmQuit,
}

/// An in-editor prompt occupying the bottom line.
enum Prompt {
    Search { buf: String },
    ReplaceFind { buf: String },
    ReplaceWith { find: String, buf: String },
}

pub struct EditorState {
    pub name: String,
    pub path: VfsPath,
    buf: EditorBuffer,
    /// Cursor as an absolute char index.
    cursor: usize,
    /// Preferred column for vertical movement.
    goal_col: Option<usize>,
    pub dirty: bool,
    top_line: usize,
    left_col: usize,
    /// Live marking anchor (block being extended by the cursor).
    anchor: Option<usize>,
    /// Finalized block (start, end) in char indices.
    block: Option<(usize, usize)>,
    clipboard: String,
    last_search: String,
    status: String,
    prompt: Option<Prompt>,
    view_rows: usize,
    view_cols: usize,
}

impl EditorState {
    pub fn new(name: String, path: VfsPath, text: &str) -> Self {
        EditorState {
            name,
            path,
            buf: EditorBuffer::from_str(text),
            cursor: 0,
            goal_col: None,
            dirty: false,
            top_line: 0,
            left_col: 0,
            anchor: None,
            block: None,
            clipboard: String::new(),
            last_search: String::new(),
            status: String::new(),
            prompt: None,
            view_rows: 1,
            view_cols: 1,
        }
    }

    pub fn contents(&self) -> String {
        self.buf.text()
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.status = "Saved".to_string();
    }

    pub fn is_prompting(&self) -> bool {
        self.prompt.is_some()
    }

    // -- Geometry helpers --------------------------------------------------

    fn cur_line(&self) -> usize {
        self.buf.char_to_line(self.cursor)
    }

    fn cur_col(&self) -> usize {
        self.cursor - self.buf.line_to_char(self.cur_line())
    }

    fn line_start_char(&self, line: usize) -> usize {
        self.buf.line_to_char(line)
    }

    // -- Key handling ------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) -> EditorSignal {
        if self.prompt.is_some() {
            return self.handle_prompt_key(key);
        }
        self.handle_edit_key(key)
    }

    fn handle_edit_key(&mut self, key: KeyEvent) -> EditorSignal {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.status.clear();

        match key.code {
            KeyCode::F(10) | KeyCode::Esc => {
                if self.dirty {
                    return EditorSignal::ConfirmQuit;
                }
                return EditorSignal::Close;
            }
            KeyCode::F(2) => return EditorSignal::Save { close_after: false },
            KeyCode::F(3) => self.toggle_mark(),
            KeyCode::F(5) => self.copy_block(),
            KeyCode::F(6) => self.move_block(),
            KeyCode::F(8) => self.delete_block(),
            KeyCode::F(7) => {
                self.prompt = Some(Prompt::Search {
                    buf: self.last_search.clone(),
                })
            }
            KeyCode::F(4) => {
                self.prompt = Some(Prompt::ReplaceFind {
                    buf: self.last_search.clone(),
                })
            }
            KeyCode::Char('z') if ctrl => {
                if let Some(c) = self.buf.undo() {
                    self.cursor = c;
                    self.dirty = true;
                    self.clear_marks();
                }
            }
            KeyCode::Char('y') if ctrl => {
                if let Some(c) = self.buf.redo() {
                    self.cursor = c;
                    self.dirty = true;
                    self.clear_marks();
                }
            }
            KeyCode::Char('v') if ctrl => self.paste(),

            KeyCode::Up => self.move_vertical(-1),
            KeyCode::Down => self.move_vertical(1),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Home => {
                self.cursor = self.line_start_char(self.cur_line());
                self.goal_col = None;
            }
            KeyCode::End => {
                let line = self.cur_line();
                self.cursor = self.line_start_char(line) + self.buf.line_len(line);
                self.goal_col = None;
            }
            KeyCode::PageUp => self.move_vertical(-(self.view_rows as isize - 1)),
            KeyCode::PageDown => self.move_vertical(self.view_rows as isize - 1),

            KeyCode::Enter => self.insert_text("\n"),
            KeyCode::Tab => self.insert_text("    "),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Char(c) => self.insert_text(&c.to_string()),
            _ => {}
        }
        EditorSignal::Stay
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) -> EditorSignal {
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Backspace => {
                if let Some(b) = self.prompt_buf_mut() {
                    b.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(b) = self.prompt_buf_mut() {
                    b.push(c);
                }
            }
            KeyCode::Enter => self.submit_prompt(),
            _ => {}
        }
        EditorSignal::Stay
    }

    fn prompt_buf_mut(&mut self) -> Option<&mut String> {
        match self.prompt.as_mut()? {
            Prompt::Search { buf } => Some(buf),
            Prompt::ReplaceFind { buf } => Some(buf),
            Prompt::ReplaceWith { buf, .. } => Some(buf),
        }
    }

    fn submit_prompt(&mut self) {
        match self.prompt.take() {
            Some(Prompt::Search { buf }) => {
                self.last_search = buf;
                self.search_next();
            }
            Some(Prompt::ReplaceFind { buf }) => {
                if buf.is_empty() {
                    return;
                }
                self.prompt = Some(Prompt::ReplaceWith {
                    find: buf,
                    buf: String::new(),
                });
            }
            Some(Prompt::ReplaceWith { find, buf }) => {
                self.last_search = find.clone();
                let n = self.replace_all(&find, &buf);
                self.status = format!("Replaced {n} occurrence(s)");
            }
            _ => {}
        }
    }

    // -- Movement ----------------------------------------------------------

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.goal_col = None;
    }

    fn move_right(&mut self) {
        if self.cursor < self.buf.len_chars() {
            self.cursor += 1;
        }
        self.goal_col = None;
    }

    fn move_vertical(&mut self, delta: isize) {
        let line = self.cur_line();
        let goal = self.goal_col.unwrap_or_else(|| self.cur_col());
        self.goal_col = Some(goal);
        let max_line = self.buf.len_lines().saturating_sub(1);
        let target = (line as isize + delta).clamp(0, max_line as isize) as usize;
        let col = goal.min(self.buf.line_len(target));
        self.cursor = self.line_start_char(target) + col;
    }

    // -- Editing -----------------------------------------------------------

    fn insert_text(&mut self, text: &str) {
        self.cursor = self.buf.insert(self.cursor, text);
        self.dirty = true;
        self.goal_col = None;
        self.clear_marks();
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.buf.delete(self.cursor - 1, self.cursor);
            self.dirty = true;
        }
        self.goal_col = None;
        self.clear_marks();
    }

    fn delete_forward(&mut self) {
        if self.cursor < self.buf.len_chars() {
            self.buf.delete(self.cursor, self.cursor + 1);
            self.dirty = true;
        }
        self.goal_col = None;
        self.clear_marks();
    }

    fn paste(&mut self) {
        if !self.clipboard.is_empty() {
            let text = self.clipboard.clone();
            self.cursor = self.buf.insert(self.cursor, &text);
            self.dirty = true;
            self.clear_marks();
        }
    }

    // -- Block operations --------------------------------------------------

    fn clear_marks(&mut self) {
        self.anchor = None;
        self.block = None;
    }

    fn toggle_mark(&mut self) {
        if self.anchor.is_some() {
            // Finalize the live block.
            let a = self.anchor.take().unwrap();
            self.block = Some(order(a, self.cursor));
        } else if self.block.is_some() {
            self.block = None;
        } else {
            self.anchor = Some(self.cursor);
            self.block = None;
        }
    }

    /// The current block range (fixed, or live from the anchor).
    fn block_range(&self) -> Option<(usize, usize)> {
        if let Some((s, e)) = self.block {
            Some((s, e))
        } else {
            self.anchor.map(|a| order(a, self.cursor))
        }
    }

    fn copy_block(&mut self) {
        if let Some((s, e)) = self.block_range() {
            self.clipboard = self.buf.slice(s, e);
            self.status = format!("Copied {} chars", e - s);
        }
    }

    fn delete_block(&mut self) {
        if let Some((s, e)) = self.block_range() {
            self.buf.delete(s, e);
            self.cursor = s;
            self.dirty = true;
            self.clear_marks();
        }
    }

    fn move_block(&mut self) {
        let Some((s, e)) = self.block_range() else {
            return;
        };
        if self.cursor >= s && self.cursor <= e {
            self.status = "Move target is inside the block".to_string();
            return;
        }
        let text = self.buf.slice(s, e);
        let block_len = e - s;
        self.buf.delete(s, e);
        // Adjust the insertion point for the removed block.
        let insert_at = if self.cursor > e {
            self.cursor - block_len
        } else {
            self.cursor
        };
        self.cursor = self.buf.insert(insert_at, &text);
        self.dirty = true;
        self.clear_marks();
    }

    // -- Search & replace --------------------------------------------------

    fn search_next(&mut self) {
        if self.last_search.is_empty() {
            return;
        }
        let from = self.cursor + 1;
        let found = self
            .buf
            .find(&self.last_search, from)
            .or_else(|| self.buf.find(&self.last_search, 0));
        match found {
            Some(idx) => {
                self.cursor = idx;
                self.goal_col = None;
            }
            None => self.status = format!("Not found: {}", self.last_search),
        }
    }

    fn replace_all(&mut self, find: &str, with: &str) -> usize {
        if find.is_empty() {
            return 0;
        }
        let mut count = 0;
        let mut pos = 0;
        while let Some(idx) = self.buf.find(find, pos) {
            self.buf.replace_range(idx, idx + find.chars().count(), with);
            pos = idx + with.chars().count();
            count += 1;
        }
        if count > 0 {
            self.dirty = true;
            self.cursor = self.cursor.min(self.buf.len_chars());
            self.clear_marks();
        }
        count
    }
}

fn order(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str) -> EditorState {
        EditorState::new("t".into(), VfsPath::local("/tmp/x"), text)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_and_undo() {
        let mut e = ed("");
        for c in "hi".chars() {
            e.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(e.contents(), "hi");
        assert!(e.dirty);

        e.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(e.contents(), "h");
        e.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert_eq!(e.contents(), "hi");
    }

    #[test]
    fn mark_and_delete_block() {
        let mut e = ed("abcdef");
        e.handle_key(key(KeyCode::F(3))); // start mark at 0
        for _ in 0..3 {
            e.handle_key(key(KeyCode::Right)); // cursor -> 3
        }
        e.handle_key(key(KeyCode::F(8))); // delete block [0,3)
        assert_eq!(e.contents(), "def");
    }

    #[test]
    fn copy_block_and_paste() {
        let mut e = ed("abc");
        e.handle_key(key(KeyCode::F(3)));
        for _ in 0..3 {
            e.handle_key(key(KeyCode::Right));
        }
        e.handle_key(key(KeyCode::F(5))); // copy "abc"
        // cursor at end; paste duplicates.
        e.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert_eq!(e.contents(), "abcabc");
    }

    #[test]
    fn search_and_replace_via_prompts() {
        let mut e = ed("a b a b");
        e.handle_key(key(KeyCode::F(4))); // replace
        e.handle_key(key(KeyCode::Char('a')));
        e.handle_key(key(KeyCode::Enter)); // find = "a"
        e.handle_key(key(KeyCode::Char('X')));
        e.handle_key(key(KeyCode::Enter)); // with = "X"
        assert_eq!(e.contents(), "X b X b");
    }

    #[test]
    fn vertical_movement_keeps_goal_column() {
        let mut e = ed("longline\nx\nshort");
        // Move to col 5 on line 0.
        for _ in 0..5 {
            e.handle_key(key(KeyCode::Right));
        }
        e.handle_key(key(KeyCode::Down)); // line 1 "x" -> clamps to col 1
        assert_eq!(e.cur_line(), 1);
        e.handle_key(key(KeyCode::Down)); // line 2 "short" -> goal col 5 restored
        assert_eq!(e.cur_line(), 2);
        assert_eq!(e.cur_col(), 5);
    }
}
