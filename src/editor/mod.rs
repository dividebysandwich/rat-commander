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
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
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
    /// Text body and footer (F-key bar) rects, recorded by the renderer for
    /// mouse hit-testing.
    text_area: Rect,
    footer_area: Rect,
    /// Where a left-drag selection began (char index), while a drag is active.
    mouse_anchor: Option<usize>,
    /// When `Some`, the editor is in (in-place, file-backed) hex mode.
    hex: Option<hex::HexEditor>,
    /// Last hex-mode search string (prefilled into the search dialog).
    last_hex_search: String,
    /// Incremental syntax highlighter (text mode), when a syntax matched.
    hl: Option<crate::syntax::Highlighter>,
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
            text_area: Rect::default(),
            footer_area: Rect::default(),
            mouse_anchor: None,
            hex: None,
            last_hex_search: String::new(),
            hl: None,
        }
    }

    /// Turn on syntax highlighting if a syntax matches the file name and the
    /// content is within the size cap. `dark` selects a fitting bundled theme.
    pub fn enable_syntax(&mut self, dark: bool) {
        if self.buf.len_chars() <= crate::syntax::HL_MAX_BYTES {
            self.hl = crate::syntax::Highlighter::for_file(&self.name, dark);
        }
    }

    /// Ensure the highlighter has processed lines up to (and including) `upto`.
    fn ensure_hl(&mut self, upto: usize) {
        let total = self.buf.len_lines();
        let Some(hl) = self.hl.as_mut() else {
            return;
        };
        // Disjoint field borrows: `hl` (self.hl) vs. self.buf.
        while hl.processed() < upto && hl.processed() < total {
            let i = hl.processed();
            let display = self.buf.line_text(i);
            hl.process_next(&display);
        }
    }

    /// Per-character foreground colors for `line` (length `len`), or `None` when
    /// highlighting is off.
    fn line_fg(&self, line: usize, len: usize, default: ratatui::style::Color) -> Option<Vec<ratatui::style::Color>> {
        self.hl.as_ref().map(|hl| hl.line_fg(line, len, default))
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

    /// The last hex search string (to prefill the dialog).
    pub fn last_hex_search(&self) -> String {
        self.last_hex_search.clone()
    }

    /// Hex-mode search / replace. `hex` ⇒ the strings are hex bytes (e.g.
    /// "48 65"); otherwise they are literal ASCII bytes. Replace is overwrite-
    /// only, so the replacement must equal the search length.
    pub fn apply_hex_search_replace(
        &mut self,
        replace: bool,
        search: &str,
        replacement: &str,
        hex: bool,
        backwards: bool,
    ) {
        let parse = |s: &str| -> Option<Vec<u8>> {
            if hex {
                parse_hex_bytes(s)
            } else {
                Some(s.as_bytes().to_vec())
            }
        };
        let pat = match parse(search) {
            Some(v) if !v.is_empty() => v,
            _ => {
                self.status = "Invalid search bytes".to_string();
                return;
            }
        };
        self.last_hex_search = search.to_string();
        let Some(h) = self.hex.as_mut() else {
            return;
        };
        if replace {
            let rep = match parse(replacement) {
                Some(v) => v,
                None => {
                    self.status = "Invalid replacement bytes".to_string();
                    return;
                }
            };
            if rep.len() != pat.len() {
                self.status = "Replacement must be the same length (overwrite-only)".to_string();
                return;
            }
            let n = h.replace_all(&pat, &rep);
            self.dirty = self.hex.as_ref().unwrap().dirty;
            self.status = format!("Replaced {n} occurrence(s)");
        } else {
            let from = if backwards { h.cursor } else { (h.cursor + 1).min(h.len) };
            let found = h.find(&pat, from, backwards);
            match found {
                Some(off) => {
                    h.cursor = off;
                    h.nibble_low = false;
                }
                None => self.status = "Not found".to_string(),
            }
        }
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

        // Run the edit, then — if the buffer changed — invalidate the syntax
        // highlight from the first affected line so only the suffix re-highlights.
        let pre_rev = self.buf.revision();
        let pre_line = self.cur_line();
        let sig = self.handle_text_key(key, ctrl);
        if self.buf.revision() != pre_rev {
            let from = pre_line.min(self.cur_line());
            if let Some(hl) = self.hl.as_mut() {
                hl.invalidate(from);
            }
        }
        sig
    }

    /// Route a mouse event: a left click positions the cursor, a left-drag marks
    /// a block (like F3), the wheel scrolls, and the F-key bar acts as buttons.
    pub fn handle_mouse(&mut self, ev: MouseEvent) -> EditorSignal {
        let (col, row) = (ev.column, ev.row);

        // A click on the F-key bar acts as that function key (when the bar is
        // actually showing — a status message replaces it).
        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left))
            && row == self.footer_area.y
            && self.status.is_empty()
        {
            let labels: &[&str] = if self.is_hex() {
                &crate::ui::fkeys::HEX_LABELS
            } else {
                &crate::ui::fkeys::EDITOR_LABELS
            };
            return match crate::ui::fkeys::index_at(self.footer_area, labels, col, row) {
                Some(i) => self.handle_key(KeyEvent::new(KeyCode::F(i as u8 + 1), KeyModifiers::NONE)),
                None => EditorSignal::Stay,
            };
        }

        if self.is_hex() {
            return self.handle_hex_mouse(ev);
        }

        match ev.kind {
            MouseEventKind::ScrollUp => {
                self.status.clear();
                self.move_vertical(-3);
            }
            MouseEventKind::ScrollDown => {
                self.status.clear();
                self.move_vertical(3);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.status.clear();
                if let Some(c) = self.char_at_screen(col, row) {
                    self.cursor = c;
                    self.goal_col = None;
                    self.clear_marks();
                    self.mouse_anchor = Some(c);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(c) = self.char_at_screen(col, row) {
                    // Begin marking on the first drag away from the press point.
                    if self.anchor.is_none()
                        && let Some(a) = self.mouse_anchor
                        && a != c
                    {
                        self.anchor = Some(a);
                    }
                    self.cursor = c;
                    self.goal_col = None;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // Finalize a drag selection so it sticks (like a second F3).
                if let Some(a) = self.anchor.take() {
                    self.block = Some(order(a, self.cursor));
                }
                self.mouse_anchor = None;
            }
            _ => {}
        }
        EditorSignal::Stay
    }

    /// Mouse handling in hex mode: the wheel scrolls and a click places the byte
    /// cursor on the clicked hex/ASCII cell.
    fn handle_hex_mouse(&mut self, ev: MouseEvent) -> EditorSignal {
        match ev.kind {
            MouseEventKind::ScrollUp => {
                if let Some(h) = self.hex.as_mut() {
                    h.move_rows(-3);
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(h) = self.hex.as_mut() {
                    h.move_rows(3);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((off, ascii)) = self.hex_cell_at(ev.column, ev.row)
                    && let Some(h) = self.hex.as_mut()
                {
                    h.cursor = off;
                    h.ascii_pane = ascii;
                    h.nibble_low = false;
                }
            }
            _ => {}
        }
        EditorSignal::Stay
    }

    /// Map a screen point to a char index in text mode (clamped to the buffer).
    fn char_at_screen(&self, col: u16, row: u16) -> Option<usize> {
        let a = self.text_area;
        if a.width == 0
            || a.height == 0
            || col < a.x
            || col >= a.x + a.width
            || row < a.y
            || row >= a.y + a.height
        {
            return None;
        }
        let lines = self.buf.len_lines();
        if lines == 0 {
            return Some(0);
        }
        let line = (self.top_line + (row - a.y) as usize).min(lines - 1);
        let col_in = (self.left_col + (col - a.x) as usize).min(self.buf.line_len(line));
        Some(self.line_start_char(line) + col_in)
    }

    /// Map a screen point in hex mode to `(byte offset, ascii_pane)`, mirroring
    /// the column layout in `render_hex` (offset col + hex cells at x=10, ASCII
    /// at x=60). `None` when the click misses a real byte.
    fn hex_cell_at(&self, col: u16, row: u16) -> Option<(u64, bool)> {
        let a = self.text_area;
        let h = self.hex.as_ref()?;
        if row < a.y || row >= a.y + a.height || col < a.x {
            return None;
        }
        let bpr = hex::BYTES_PER_ROW as usize;
        let base = h.top + (row - a.y) as u64 * hex::BYTES_PER_ROW;
        let x = (col - a.x) as usize;
        let (j, ascii) = if x >= 60 && x < 60 + bpr {
            (x - 60, true)
        } else if x >= 10 {
            // Hex cells: cell j starts at 10 + 3*j (+1 once past the 8-byte gap).
            let rel = x - 10;
            let mut hit = None;
            for j in 0..bpr {
                let start = 3 * j + usize::from(j >= 8);
                if rel >= start && rel < start + 2 {
                    hit = Some(j);
                    break;
                }
            }
            (hit?, false)
        } else {
            return None;
        };
        let off = base + j as u64;
        (off < h.len).then_some((off, ascii))
    }

    fn handle_text_key(&mut self, key: KeyEvent, ctrl: bool) -> EditorSignal {
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
                    // No persistent banner; only note the read-only case.
                    if ro {
                        self.status = "read-only file".to_string();
                    }
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
            KeyCode::F(7) => return EditorSignal::OpenSearch,
            KeyCode::F(4) => return EditorSignal::OpenReplace,
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
        // The whole buffer was rewritten — re-highlight from the top.
        if let Some(hl) = self.hl.as_mut() {
            hl.invalidate(0);
        }
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

/// Parse a hex-byte string like "48 65 6c" or "48656c" into bytes.
fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() || !cleaned.len().is_multiple_of(2) {
        return None;
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cleaned[i..i + 2], 16).ok())
        .collect()
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
    fn hex_search_and_replace() {
        let p = tmpfile(b"hello hello hello");
        let mut e = EditorState::new("h".into(), VfsPath::local(&p), "x");
        e.handle_key(key(KeyCode::F(9)));
        // ASCII search moves the cursor to the next match.
        e.apply_hex_search_replace(false, "hello", "", false, false);
        // Cursor was at 0; next match starts at offset 6.
        // (find searches from cursor+1)
        // Replace-all (equal length) overwrites every occurrence.
        e.apply_hex_search_replace(true, "hello", "HELLO", false, false);
        e.flush_hex().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"HELLO HELLO HELLO");

        // Hex-byte search input ("68 65" = "he") parses and finds.
        e.apply_hex_search_replace(false, "48 45 4C 4C 4F", "", true, false);
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
        // The F-key bar (not a mode banner) is shown, with supported functions.
        assert!(s.contains("Save") && s.contains("Text"), "F-key bar in hex mode");
        assert!(!s.contains("Hex mode"), "no persistent mode banner");
        std::fs::remove_file(&p).ok();
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE }
    }

    /// Place the text body at (0,1) and the F-key bar at row 7, as the renderer
    /// would, so mouse hit-testing has geometry to work with.
    fn with_layout(e: &mut EditorState) {
        e.text_area = Rect::new(0, 1, 20, 5);
        e.footer_area = Rect::new(0, 7, 20, 1);
        e.view_rows = 5;
        e.view_cols = 20;
    }

    #[test]
    fn click_moves_cursor() {
        let mut e = ed("abcdef\nghijkl\nmnopqr");
        with_layout(&mut e);
        // Row 1 is the first text line; column 3 → char index 3 on line 0.
        e.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 3, 1));
        assert_eq!(e.cur_line(), 0);
        assert_eq!(e.cur_col(), 3);
        // Second text line, column 2.
        e.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));
        assert_eq!(e.cur_line(), 1);
        assert_eq!(e.cur_col(), 2);
    }

    #[test]
    fn drag_marks_a_block_but_a_click_does_not() {
        let mut e = ed("abcdef\nghijkl");
        with_layout(&mut e);
        // Press at col 0, drag to col 3, release → block [0,3) like F3.
        e.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0, 1));
        assert_eq!(e.block_range(), None, "a bare press starts no selection");
        e.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3, 1));
        assert_eq!(e.block_range(), Some((0, 3)), "dragging extends a live block");
        e.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 3, 1));
        assert_eq!(e.block_range(), Some((0, 3)), "release finalizes the block");
        e.handle_key(key(KeyCode::F(5))); // copy
        assert_eq!(e.clipboard, "abc");

        // A plain click (down then up, no drag) leaves no selection and the
        // arrow keys do not extend one.
        e.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 1, 1));
        e.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 1, 1));
        e.handle_key(key(KeyCode::Right));
        assert_eq!(e.block_range(), None, "a click leaves no anchor to extend");
    }

    #[test]
    fn wheel_scrolls_the_cursor() {
        let mut e = ed("l0\nl1\nl2\nl3\nl4\nl5");
        with_layout(&mut e);
        assert_eq!(e.cur_line(), 0);
        e.handle_mouse(mouse(MouseEventKind::ScrollDown, 1, 3));
        assert_eq!(e.cur_line(), 3, "wheel down advances three lines");
        e.handle_mouse(mouse(MouseEventKind::ScrollUp, 1, 3));
        assert_eq!(e.cur_line(), 0, "wheel up rewinds three lines");
    }

    #[test]
    fn fkey_bar_click_acts_as_that_key() {
        let mut e = ed("abcdef");
        with_layout(&mut e);
        // Footer width 20, 10 labels → 2 cells each; F3 ("Mark") spans cols 4-5.
        e.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 4, 7));
        assert!(e.anchor.is_some(), "clicking F3 starts a mark");
        // F10 ("Quit") spans cols 18-19; with no unsaved changes it closes.
        let sig = e.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 18, 7));
        assert!(matches!(sig, EditorSignal::Close), "clicking F10 quits");
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
