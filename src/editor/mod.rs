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
    /// Whether the live `anchor` was started by a Shift+move (so a plain move
    /// collapses it), as opposed to an explicit F3 mark (which a move extends).
    shift_marking: bool,
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
            shift_marking: false,
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
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
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

            KeyCode::Up => {
                self.pre_move(shift);
                self.move_vertical(-1);
            }
            KeyCode::Down => {
                self.pre_move(shift);
                self.move_vertical(1);
            }
            // Ctrl-←/→ jump by word; plain ←/→ move one character.
            KeyCode::Left if ctrl => {
                self.pre_move(shift);
                self.word_left();
            }
            KeyCode::Right if ctrl => {
                self.pre_move(shift);
                self.word_right();
            }
            KeyCode::Left => {
                self.pre_move(shift);
                self.move_left();
            }
            KeyCode::Right => {
                self.pre_move(shift);
                self.move_right();
            }
            // Ctrl-Home/End jump to the start/end of the document.
            KeyCode::Home if ctrl => {
                self.pre_move(shift);
                self.cursor = 0;
                self.goal_col = None;
            }
            KeyCode::End if ctrl => {
                self.pre_move(shift);
                self.cursor = self.buf.len_chars();
                self.goal_col = None;
            }
            KeyCode::Home => {
                self.pre_move(shift);
                self.cursor = self.line_start_char(self.cur_line());
                self.goal_col = None;
            }
            KeyCode::End => {
                self.pre_move(shift);
                let line = self.cur_line();
                self.cursor = self.line_start_char(line) + self.buf.line_len(line);
                self.goal_col = None;
            }
            KeyCode::PageUp => {
                self.pre_move(shift);
                self.move_vertical(-(self.view_rows as isize - 1));
            }
            KeyCode::PageDown => {
                self.pre_move(shift);
                self.move_vertical(self.view_rows as isize - 1);
            }

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

    /// Selection bookkeeping run before a cursor movement: with Shift held, start
    /// (or keep extending) a live selection. Releasing Shift *finalizes* the
    /// live selection into a fixed block so it stays marked while the cursor
    /// moves around (rather than collapsing). An F3 mark keeps extending.
    fn pre_move(&mut self, shift: bool) {
        if shift {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
                self.block = None;
                self.shift_marking = true;
            }
        } else if self.shift_marking {
            self.finalize_marks();
        }
    }

    /// Whether `c` is part of a word (letters, digits, underscore).
    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// Move to the start of the next word (skipping the current word, then any
    /// separators), crossing line breaks like most editors.
    fn word_right(&mut self) {
        let n = self.buf.len_chars();
        let is_word = |i: usize| self.buf.char_at(i).map(Self::is_word_char).unwrap_or(false);
        while self.cursor < n && is_word(self.cursor) {
            self.cursor += 1;
        }
        while self.cursor < n && !is_word(self.cursor) {
            self.cursor += 1;
        }
        self.goal_col = None;
    }

    /// Move to the start of the current or previous word.
    fn word_left(&mut self) {
        let is_word = |i: usize| self.buf.char_at(i).map(Self::is_word_char).unwrap_or(false);
        while self.cursor > 0 && !is_word(self.cursor - 1) {
            self.cursor -= 1;
        }
        while self.cursor > 0 && is_word(self.cursor - 1) {
            self.cursor -= 1;
        }
        self.goal_col = None;
    }

    // -- Editing -----------------------------------------------------------

    fn insert_text(&mut self, text: &str) {
        self.finalize_marks();
        let pos = self.cursor;
        let len = text.chars().count();
        self.cursor = self.buf.insert(pos, text);
        self.adjust_block_insert(pos, len);
        self.dirty = true;
        self.goal_col = None;
    }

    fn backspace(&mut self) {
        self.finalize_marks();
        if self.cursor > 0 {
            let (d0, d1) = (self.cursor - 1, self.cursor);
            self.cursor = self.buf.delete(d0, d1);
            self.adjust_block_delete(d0, d1);
            self.dirty = true;
        }
        self.goal_col = None;
    }

    fn delete_forward(&mut self) {
        self.finalize_marks();
        if self.cursor < self.buf.len_chars() {
            let (d0, d1) = (self.cursor, self.cursor + 1);
            self.buf.delete(d0, d1);
            self.adjust_block_delete(d0, d1);
            self.dirty = true;
        }
        self.goal_col = None;
    }

    fn paste(&mut self) {
        if !self.clipboard.is_empty() {
            self.finalize_marks();
            let text = self.clipboard.clone();
            let pos = self.cursor;
            let len = text.chars().count();
            self.cursor = self.buf.insert(pos, &text);
            self.adjust_block_insert(pos, len);
            self.dirty = true;
        }
    }

    // -- Block operations --------------------------------------------------

    fn clear_marks(&mut self) {
        self.anchor = None;
        self.block = None;
        self.shift_marking = false;
    }

    /// Turn a live (anchor-based) selection into a fixed block so it survives
    /// cursor moves and edits. A zero-length selection is dropped. No-op when
    /// there's no live anchor (a fixed block is left as-is).
    fn finalize_marks(&mut self) {
        if let Some(a) = self.anchor.take() {
            let (s, e) = order(a, self.cursor);
            self.block = (s != e).then_some((s, e));
        }
        self.shift_marking = false;
    }

    /// Shift the fixed block to keep the *same text* marked after inserting `len`
    /// chars at `pos`: text before the block moves it; text inside grows it; text
    /// after it is unaffected. (Live anchors are finalized before any edit.)
    fn adjust_block_insert(&mut self, pos: usize, len: usize) {
        if let Some((s, e)) = self.block {
            let s2 = if pos <= s { s + len } else { s };
            let e2 = if pos < e { e + len } else { e };
            self.block = Some((s2, e2));
        }
    }

    /// Shift the fixed block to keep the same text marked after deleting
    /// `[d0, d1)`. The block shrinks by whatever overlap was removed and is
    /// dropped if the whole marked range is gone.
    fn adjust_block_delete(&mut self, d0: usize, d1: usize) {
        if let Some((s, e)) = self.block {
            let len = d1 - d0;
            let map = |x: usize| {
                if x <= d0 {
                    x
                } else if x >= d1 {
                    x - len
                } else {
                    d0 // a marker inside the removed range collapses to its start
                }
            };
            let (s2, e2) = (map(s), map(e));
            self.block = (s2 < e2).then_some((s2, e2));
        }
    }

    fn toggle_mark(&mut self) {
        // An explicit F3 mark is never a Shift-selection (plain moves extend it).
        self.shift_marking = false;
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

    /// F5: insert a copy of the marked block at the cursor (mc-editor "Copy
    /// block"). The original block stays marked.
    fn copy_block(&mut self) {
        let Some((s, e)) = self.block_range() else {
            self.status = "No block is marked".to_string();
            return;
        };
        let text = self.buf.slice(s, e);
        let len = text.chars().count();
        // Finalize a live selection so it tracks the insertion that follows.
        self.finalize_marks();
        let pos = self.cursor;
        self.cursor = self.buf.insert(pos, &text);
        self.adjust_block_insert(pos, len);
        self.dirty = true;
        self.status = format!("Copied {len} chars to the cursor");
    }

    /// Ctrl-C: copy the marked block to the internal clipboard (paste with Ctrl-V).
    fn copy_to_clipboard(&mut self) {
        if let Some((s, e)) = self.block_range() {
            self.clipboard = self.buf.slice(s, e);
            self.status = format!("Copied {} chars to clipboard", e - s);
        } else {
            self.status = "No block is marked".to_string();
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

    fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn ctrl_home_end_jump_to_document_ends() {
        let mut e = ed("line0\nline1\nline2");
        e.handle_key(key(KeyCode::Down));
        e.handle_key(key(KeyCode::Right));
        assert_ne!(e.cursor, 0);
        e.handle_key(key_mod(KeyCode::Home, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, 0);
        assert_eq!(e.cur_line(), 0);
        e.handle_key(key_mod(KeyCode::End, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, e.buf.len_chars());
        assert_eq!(e.cur_line(), 2);
    }

    #[test]
    fn ctrl_arrows_jump_by_word() {
        // "foo bar  baz": words start at 0, 4, 9 (double space before "baz").
        let mut e = ed("foo bar  baz");
        e.handle_key(key_mod(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, 4, "start of \"bar\"");
        e.handle_key(key_mod(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, 9, "start of \"baz\"");
        e.handle_key(key_mod(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, 4, "back to start of \"bar\"");
        e.handle_key(key_mod(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(e.cursor, 0, "back to start of \"foo\"");
        // Word movement crosses line breaks.
        let mut e = ed("a\nbb");
        e.handle_key(key_mod(KeyCode::Right, KeyModifiers::CONTROL)); // past "a"
        e.handle_key(key_mod(KeyCode::Right, KeyModifiers::CONTROL)); // onto "bb"
        assert_eq!(e.cur_line(), 1);
    }

    #[test]
    fn shift_arrows_select_without_f3() {
        let mut e = ed("abcdef");
        e.handle_key(key_mod(KeyCode::Right, KeyModifiers::SHIFT));
        e.handle_key(key_mod(KeyCode::Right, KeyModifiers::SHIFT));
        assert_eq!(e.block_range(), Some((0, 2)), "Shift+Right marks a block");
        e.handle_key(key(KeyCode::F(5))); // copy works on the selection
        assert_eq!(e.clipboard, "ab");
        // A plain move now *keeps* the selection (it finalizes to a fixed block).
        e.handle_key(key(KeyCode::Right));
        assert_eq!(e.block_range(), Some((0, 2)), "Shift-selection persists across plain moves");

        // Shift+Ctrl-Right selects a whole word ("foo " up to the next word).
        let mut e = ed("foo bar");
        e.handle_key(key_mod(KeyCode::Right, KeyModifiers::SHIFT | KeyModifiers::CONTROL));
        assert_eq!(e.block_range(), Some((0, 4)));
    }

    #[test]
    fn shift_selection_persists_and_f5_copies_after_moving() {
        let mut e = ed("hello world");
        // Shift-select "hello".
        for _ in 0..5 {
            e.handle_key(key_mod(KeyCode::Right, KeyModifiers::SHIFT));
        }
        assert_eq!(e.block_range(), Some((0, 5)));
        // Move the cursor away with plain arrows — the selection must stay.
        for _ in 0..3 {
            e.handle_key(key(KeyCode::Right));
        }
        assert_eq!(e.block_range(), Some((0, 5)), "selection persists across plain moves");
        // F5 still copies the marked text.
        e.handle_key(key(KeyCode::F(5)));
        assert_eq!(e.clipboard, "hello");
        // …and it pastes at the cursor.
        e.handle_key(key(KeyCode::End));
        e.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert_eq!(e.contents(), "hello worldhello");
    }

    #[test]
    fn block_stays_anchored_to_text_across_edits() {
        let marked = |e: &EditorState| -> Option<String> {
            e.block_range().map(|(s, end)| e.buf.slice(s, end))
        };
        // Mark "cde" in "abcdefg" → fixed block [2,5).
        let mut e = ed("abcdefg");
        for _ in 0..2 {
            e.handle_key(key(KeyCode::Right));
        }
        e.handle_key(key(KeyCode::F(3))); // anchor at 2
        for _ in 0..3 {
            e.handle_key(key(KeyCode::Right)); // cursor → 5
        }
        e.handle_key(key(KeyCode::F(3))); // finalize block [2,5)
        assert_eq!(marked(&e).as_deref(), Some("cde"));

        // Insert before the block — the same text stays marked.
        e.handle_key(key_mod(KeyCode::Home, KeyModifiers::CONTROL));
        e.handle_key(key(KeyCode::Char('X')));
        e.handle_key(key(KeyCode::Char('Y')));
        assert_eq!(e.contents(), "XYabcdefg");
        assert_eq!(marked(&e).as_deref(), Some("cde"), "tracks an insert before the block");

        // Delete before the block — still the same text.
        e.handle_key(key_mod(KeyCode::Home, KeyModifiers::CONTROL));
        e.handle_key(key(KeyCode::Delete)); // remove 'X'
        assert_eq!(e.contents(), "Yabcdefg");
        assert_eq!(marked(&e).as_deref(), Some("cde"), "tracks a delete before the block");

        // Editing *inside* the block keeps the selection (it grows to stay
        // contiguous), it does not clear it. Block is [3,6) ("cde"); insert 'Z'
        // between 'c' and 'd' (index 4).
        e.handle_key(key_mod(KeyCode::Home, KeyModifiers::CONTROL));
        for _ in 0..4 {
            e.handle_key(key(KeyCode::Right));
        }
        e.handle_key(key(KeyCode::Char('Z')));
        assert_eq!(e.contents(), "YabcZdefg");
        assert_eq!(
            marked(&e).as_deref(),
            Some("cZde"),
            "an edit inside the block keeps it marked (and contiguous)"
        );
    }

    #[test]
    fn f3_marking_still_extends_with_plain_arrows() {
        // Regression: an F3 mark must keep extending on *plain* arrows.
        let mut e = ed("abcdef");
        e.handle_key(key(KeyCode::F(3)));
        e.handle_key(key(KeyCode::Right));
        e.handle_key(key(KeyCode::Right));
        assert_eq!(e.block_range(), Some((0, 2)));
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
    fn hex_color_tints_the_hash_in_editor() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let p = tmpfile(b"");
        let mut e = EditorState::new("c.css".into(), VfsPath::local(&p), "a: #00ff80;");
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(40, 8)).unwrap();
        t.draw(|f| crate::editor::render::render(f, f.area(), &mut e, &theme)).unwrap();
        let b = t.backend().buffer();
        let hash = (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .find(|&(x, y)| b[(x, y)].symbol() == "#")
            .expect("'#' rendered");
        assert_eq!(
            b[hash].fg,
            ratatui::style::Color::Rgb(0x00, 0xff, 0x80),
            "hash tinted with its color"
        );
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
