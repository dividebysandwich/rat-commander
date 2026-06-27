//! Internal `mcedit`-style text editor.
//!
//! The editor is a full-screen overlay (like the viewer). It owns an
//! [`EditorBuffer`] and all cursor/selection state; the app handles only the
//! async file save when the editor asks for it.

pub mod buffer;
pub mod hex;
pub mod render;

use crate::vfs::VfsPath;
use buffer::EditorBuffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;

/// What the app should do after the editor handles a key.
pub enum EditorSignal {
    Stay,
    Close,
    /// Persist the buffer; close the editor afterwards if `close_after`.
    Save { close_after: bool },
    /// The buffer is modified and the user asked to quit: the app should show a
    /// modal save/discard/cancel confirmation.
    ConfirmQuit,
    /// Open the modal search dialog (F7).
    OpenSearch,
    /// Open the modal search & replace dialog (F4).
    OpenReplace,
}

/// Remembered search options for "find next" (`n`).
#[derive(Default, Clone)]
struct LastSearch {
    pattern: String,
    regex: bool,
    case_sensitive: bool,
    whole_words: bool,
    backwards: bool,
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
    last_search: LastSearch,
    status: String,
    view_rows: usize,
    view_cols: usize,
    /// When `Some`, the editor is in (in-place, file-backed) hex mode.
    hex: Option<hex::HexEditor>,
}

/// Above this size a file is opened straight into hex mode (text mode loads the
/// whole file, so it's reserved for reasonably sized files).
pub const MAX_TEXT_EDIT: u64 = crate::viewer::MAX_VIEW_BYTES as u64;

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
            last_search: LastSearch::default(),
            status: String::new(),
            view_rows: 1,
            view_cols: 1,
            hex: None,
        }
    }

    /// Open a (local) file directly in hex mode without loading it into memory —
    /// used for files too large to load as text.
    pub fn new_hex(name: String, path: VfsPath) -> std::io::Result<Self> {
        let hex = hex::HexEditor::open(&path.path)?;
        let mut s = Self::new(name, path, "");
        s.hex = Some(hex);
        Ok(s)
    }

    pub fn is_hex(&self) -> bool {
        self.hex.is_some()
    }

    /// Flush pending in-place hex edits to the file (the app's save path calls
    /// this instead of writing the text buffer when in hex mode).
    pub fn flush_hex(&mut self) -> std::io::Result<()> {
        if let Some(h) = self.hex.as_mut() {
            h.save()?;
        }
        self.dirty = false;
        Ok(())
    }

    pub fn contents(&self) -> String {
        self.buf.text()
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.status = "Saved".to_string();
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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.status.clear();

        // F9 toggles hex mode in either direction.
        if key.code == KeyCode::F(9) {
            self.toggle_hex();
            return EditorSignal::Stay;
        }
        if self.hex.is_some() {
            return self.handle_hex_key(key);
        }

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
            KeyCode::F(7) => return EditorSignal::OpenSearch,
            KeyCode::F(4) => return EditorSignal::OpenReplace,
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

    /// Toggle between text and hex modes. Switching is only allowed when the
    /// current mode has no unsaved changes, so the two backing stores can't
    /// diverge (and the in-place file is never clobbered by stale text).
    fn toggle_hex(&mut self) {
        if let Some(h) = self.hex.as_ref() {
            // Leaving hex mode → text mode.
            if h.dirty {
                self.status = "Save (F2) before leaving hex mode".to_string();
                return;
            }
            if h.len > MAX_TEXT_EDIT {
                self.status = "File too large for text mode".to_string();
                return;
            }
            let reload = h.saved_any;
            let path = self.path.path.clone();
            self.hex = None;
            // Re-read the file so the text view reflects any saved hex edits.
            if reload && let Ok(data) = std::fs::read(&path) {
                self.buf = EditorBuffer::from_str(&String::from_utf8_lossy(&data));
                self.cursor = 0;
                self.top_line = 0;
                self.left_col = 0;
                self.clear_marks();
            }
            self.status = "Text mode".to_string();
        } else {
            // Entering hex mode (local files only).
            if self.path.scheme != "file" {
                self.status = "Hex mode requires a local file".to_string();
                return;
            }
            if self.dirty {
                self.status = "Save (F2) before switching to hex mode".to_string();
                return;
            }
            match hex::HexEditor::open(Path::new(&self.path.path)) {
                Ok(h) => {
                    let ro = h.readonly;
                    self.hex = Some(h);
                    self.status = if ro { "Hex mode (read-only)" } else { "Hex mode" }.to_string();
                }
                Err(e) => self.status = format!("cannot open for hex: {e}"),
            }
        }
    }

    /// Key handling while in hex mode.
    fn handle_hex_key(&mut self, key: KeyEvent) -> EditorSignal {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::F(10) | KeyCode::Esc => {
                return if self.dirty {
                    EditorSignal::ConfirmQuit
                } else {
                    EditorSignal::Close
                };
            }
            // Saving routes through the app, which flushes the overlay in place.
            KeyCode::F(2) => return EditorSignal::Save { close_after: false },
            _ => {}
        }

        let rows = self.view_rows.max(1) as i64;
        let readonly = self.hex.as_ref().map(|h| h.readonly).unwrap_or(true);
        let mut typed = false;
        if let Some(h) = self.hex.as_mut() {
            match key.code {
                KeyCode::Up => h.move_rows(-1),
                KeyCode::Down => h.move_rows(1),
                KeyCode::Left => h.move_by(-1),
                KeyCode::Right => h.move_by(1),
                KeyCode::Home if ctrl => h.goto_start(),
                KeyCode::End if ctrl => h.goto_end(),
                KeyCode::Home => h.row_start(),
                KeyCode::End => h.row_end(),
                KeyCode::PageUp => h.move_rows(-(rows - 1).max(1)),
                KeyCode::PageDown => h.move_rows((rows - 1).max(1)),
                KeyCode::Tab => h.toggle_pane(),
                KeyCode::Backspace => h.move_by(-1),
                KeyCode::Char(c) => {
                    typed = true;
                    if !readonly {
                        if h.ascii_pane {
                            h.input_ascii(c);
                        } else {
                            let _ = h.input_hex(c);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(h) = &self.hex {
            self.dirty = h.dirty;
        }
        if typed && readonly {
            self.status = "read-only file".to_string();
        }
        EditorSignal::Stay
    }

    /// Apply the result of the modal search / search-and-replace dialog.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_search_replace(
        &mut self,
        replace: bool,
        search: &str,
        replacement: &str,
        regex: bool,
        case_sensitive: bool,
        whole_words: bool,
        backwards: bool,
    ) {
        self.last_search = LastSearch {
            pattern: search.to_string(),
            regex,
            case_sensitive,
            whole_words,
            backwards,
        };
        if replace {
            let n = self.replace_all(search, replacement, regex, case_sensitive, whole_words);
            self.status = format!("Replaced {n} occurrence(s)");
        } else {
            self.search_next();
        }
    }

    /// Build a regex from the given options.
    fn build_regex(
        pattern: &str,
        regex: bool,
        case_sensitive: bool,
        whole_words: bool,
    ) -> Option<regex::Regex> {
        let mut pat = if regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        if whole_words {
            pat = format!(r"\b(?:{pat})\b");
        }
        regex::RegexBuilder::new(&pat)
            .case_insensitive(!case_sensitive)
            .build()
            .ok()
    }

    /// Find the next (or previous) match of the remembered search.
    fn search_next(&mut self) {
        let ls = self.last_search.clone();
        if ls.pattern.is_empty() {
            return;
        }
        let Some(re) = Self::build_regex(&ls.pattern, ls.regex, ls.case_sensitive, ls.whole_words)
        else {
            self.status = "Invalid search pattern".to_string();
            return;
        };
        let text = self.buf.text();
        // Work in byte offsets, then convert to a char index.
        let cur_byte = char_to_byte(&text, self.cursor);
        let found = if ls.backwards {
            re.find_iter(&text)
                .filter(|m| m.start() < cur_byte)
                .last()
                .or_else(|| re.find_iter(&text).last())
        } else {
            re.find_at(&text, (cur_byte + 1).min(text.len()))
                .or_else(|| re.find(&text))
        };
        match found {
            Some(m) => {
                self.cursor = text[..m.start()].chars().count();
                self.goal_col = None;
            }
            None => self.status = format!("Not found: {}", ls.pattern),
        }
    }

    /// Replace all matches; returns the count. Done as a single buffer edit so
    /// it is one undo step.
    fn replace_all(
        &mut self,
        search: &str,
        replacement: &str,
        regex: bool,
        case_sensitive: bool,
        whole_words: bool,
    ) -> usize {
        if search.is_empty() {
            return 0;
        }
        let Some(re) = Self::build_regex(search, regex, case_sensitive, whole_words) else {
            self.status = "Invalid search pattern".to_string();
            return 0;
        };
        let text = self.buf.text();
        let count = re.find_iter(&text).count();
        if count == 0 {
            return 0;
        }
        // In literal mode replacement is verbatim; in regex mode allow $1 refs.
        let new_text = if regex {
            re.replace_all(&text, replacement).into_owned()
        } else {
            re.replace_all(&text, regex::NoExpand(replacement)).into_owned()
        };
        let len = self.buf.len_chars();
        self.cursor = self.buf.replace_range(0, len, &new_text).min(new_text.chars().count());
        self.cursor = self.cursor.min(self.buf.len_chars());
        self.dirty = true;
        self.clear_marks();
        count
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

}

fn order(a: usize, b: usize) -> (usize, usize) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Convert a char index into a byte offset within `text`.
fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
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

    fn tmpfile(bytes: &[u8]) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("rc_edhex_{}_{nanos}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn hex_mode_edits_file_in_place() {
        let p = tmpfile(b"hello");
        let mut e = EditorState::new("h".into(), VfsPath::local(&p), "hello");
        e.handle_key(key(KeyCode::F(9))); // enter hex
        assert!(e.is_hex());
        // Overwrite first byte 'h' (0x68) with 'H' (0x48).
        e.handle_key(key(KeyCode::Char('4')));
        e.handle_key(key(KeyCode::Char('8')));
        assert!(e.dirty, "byte edit marks dirty");
        assert_eq!(std::fs::read(&p).unwrap(), b"hello", "not written until save");

        e.flush_hex().unwrap(); // the app's save path calls this
        assert!(!e.dirty);
        assert_eq!(std::fs::read(&p).unwrap(), b"Hello", "in-place byte write");

        // Toggle back to text reflects the saved change.
        e.handle_key(key(KeyCode::F(9)));
        assert!(!e.is_hex());
        assert_eq!(e.contents(), "Hello");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn hex_toggle_blocked_with_unsaved_text() {
        let p = tmpfile(b"abc");
        let mut e = EditorState::new("h".into(), VfsPath::local(&p), "abc");
        e.handle_key(key(KeyCode::Char('x'))); // dirty text
        e.handle_key(key(KeyCode::F(9)));
        assert!(!e.is_hex(), "can't enter hex with unsaved text edits");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn hex_view_renders_offset_and_ascii() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let p = tmpfile(b"hello world example bytes 0123456789ABCDEF");
        let mut e = EditorState::new("h".into(), VfsPath::local(&p), "x");
        e.handle_key(key(KeyCode::F(9)));
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(90, 12)).unwrap();
        t.draw(|f| crate::editor::render::render(f, f.area(), &mut e, &theme))
            .unwrap();
        let b = t.backend().buffer();
        let mut s = String::new();
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                s.push_str(b[(x, y)].symbol());
            }
        }
        assert!(s.contains("00000000"), "offset column");
        assert!(s.contains("HEX"), "hex status indicator");
        assert!(s.contains("hello world"), "ascii pane shows content");
        std::fs::remove_file(&p).ok();
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
    fn literal_replace_all() {
        let mut e = ed("a b a b");
        e.apply_search_replace(true, "a", "X", false, false, false, false);
        assert_eq!(e.contents(), "X b X b");
    }

    #[test]
    fn regex_replace_with_groups() {
        let mut e = ed("name: bob");
        e.apply_search_replace(true, r"(\w+): (\w+)", "$2=$1", true, true, false, false);
        assert_eq!(e.contents(), "bob=name");
    }

    #[test]
    fn case_insensitive_search_moves_cursor() {
        let mut e = ed("one TWO three");
        e.apply_search_replace(false, "two", "", false, false, false, false);
        // Cursor should land on "TWO" (char index 4).
        assert_eq!(e.cur_line(), 0);
        assert_eq!(e.cur_col(), 4);
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
