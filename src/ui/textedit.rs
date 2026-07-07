//! Shared Emacs/readline-style editing for every single-line text input — the
//! command line and every dialog field. [`edit_key`] applies one keystroke to a
//! `(value, cursor)` pair, using a process-wide kill buffer and mark so text can
//! be cut in one field and yanked in another (like Emacs' kill ring).
//!
//! The mark and kill live in [`EditState`]; the app shares a single global one
//! (see [`edit_key`]), while tests drive [`edit_key_with`] with a private state
//! for isolation.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::RwLock;

/// The kill buffer and mark shared by all inputs.
#[derive(Default)]
pub struct EditState {
    /// Char index of the mark set by C-@, or `None`. Cleared on any edit.
    pub mark: Option<usize>,
    /// The last killed/copied text, yanked back by C-y.
    pub kill: String,
}

/// The one shared editing state used by the live UI.
static GLOBAL: RwLock<EditState> = RwLock::new(EditState { mark: None, kill: String::new() });

/// What a keystroke did, so callers can react (e.g. the command line resets its
/// history cursor only on an actual edit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    /// The text changed.
    Modified,
    /// Only the cursor/mark moved.
    Moved,
    /// Not an editing key (the caller should handle it).
    Ignored,
}

/// Apply one editing key to `(value, cursor)` using the shared global state.
pub fn edit_key(value: &mut String, cursor: &mut usize, key: KeyEvent) -> Edit {
    let mut st = GLOBAL.write().unwrap();
    edit_key_with(value, cursor, &mut st, key)
}

/// Clear the shared mark. Call after modifying an input outside [`edit_key`]
/// (e.g. the command line's own `insert`/`backspace`), so a stale mark can't
/// later cut the wrong region.
pub fn mark_clear() {
    GLOBAL.write().unwrap().mark = None;
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// First index at or after `i` past the end of the next word.
fn word_forward(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && !is_word(chars[i]) {
        i += 1;
    }
    while i < chars.len() && is_word(chars[i]) {
        i += 1;
    }
    i
}

/// First index at or before `i` at the start of the previous word.
fn word_backward(chars: &[char], mut i: usize) -> usize {
    while i > 0 && !is_word(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// Apply one editing key against an explicit [`EditState`]. See the module docs.
pub fn edit_key_with(
    value: &mut String,
    cursor: &mut usize,
    st: &mut EditState,
    key: KeyEvent,
) -> Edit {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut chars: Vec<char> = value.chars().collect();
    let len = chars.len();
    // A stale mark from another (shorter) field must never index out of bounds.
    let mark = st.mark.map(|m| m.min(len));
    let rebuild = |chars: &[char], value: &mut String| {
        *value = chars.iter().collect();
    };

    match key.code {
        // -- Cursor movement (mark preserved) --
        KeyCode::Home => {
            *cursor = 0;
            Edit::Moved
        }
        KeyCode::End => {
            *cursor = len;
            Edit::Moved
        }
        KeyCode::Char('a') if ctrl && !alt => {
            *cursor = 0;
            Edit::Moved
        }
        KeyCode::Char('e') if ctrl && !alt => {
            *cursor = len;
            Edit::Moved
        }
        KeyCode::Left if !ctrl && !alt => {
            *cursor = cursor.saturating_sub(1);
            Edit::Moved
        }
        KeyCode::Char('b') if ctrl && !alt => {
            *cursor = cursor.saturating_sub(1);
            Edit::Moved
        }
        KeyCode::Right if !ctrl && !alt => {
            if *cursor < len {
                *cursor += 1;
            }
            Edit::Moved
        }
        KeyCode::Char('f') if ctrl && !alt => {
            if *cursor < len {
                *cursor += 1;
            }
            Edit::Moved
        }
        KeyCode::Char('b') if alt && !ctrl => {
            *cursor = word_backward(&chars, *cursor);
            Edit::Moved
        }
        KeyCode::Char('f') if alt && !ctrl => {
            *cursor = word_forward(&chars, *cursor);
            Edit::Moved
        }

        // -- Deletion (clears the mark) --
        // Delete a word backward: Alt-Backspace or Alt-C-h.
        KeyCode::Backspace if alt => {
            let start = word_backward(&chars, *cursor);
            chars.drain(start..*cursor);
            *cursor = start;
            rebuild(&chars, value);
            st.mark = None;
            Edit::Modified
        }
        KeyCode::Char('h') if ctrl && alt => {
            let start = word_backward(&chars, *cursor);
            chars.drain(start..*cursor);
            *cursor = start;
            rebuild(&chars, value);
            st.mark = None;
            Edit::Modified
        }
        // Delete the previous character: Backspace or C-h.
        KeyCode::Backspace => {
            if *cursor > 0 {
                chars.remove(*cursor - 1);
                *cursor -= 1;
                rebuild(&chars, value);
            }
            st.mark = None;
            Edit::Modified
        }
        KeyCode::Char('h') if ctrl && !alt => {
            if *cursor > 0 {
                chars.remove(*cursor - 1);
                *cursor -= 1;
                rebuild(&chars, value);
            }
            st.mark = None;
            Edit::Modified
        }
        // Delete the character at the cursor: Delete or C-d.
        KeyCode::Delete => {
            if *cursor < len {
                chars.remove(*cursor);
                rebuild(&chars, value);
            }
            st.mark = None;
            Edit::Modified
        }
        KeyCode::Char('d') if ctrl && !alt => {
            if *cursor < len {
                chars.remove(*cursor);
                rebuild(&chars, value);
            }
            st.mark = None;
            Edit::Modified
        }
        // Kill from the cursor to the end of the line (into the kill buffer).
        KeyCode::Char('k') if ctrl && !alt => {
            if *cursor < len {
                st.kill = chars[*cursor..].iter().collect();
                chars.truncate(*cursor);
                rebuild(&chars, value);
            }
            st.mark = None;
            Edit::Modified
        }

        // -- Mark, kill/copy region, yank --
        // Set the mark (C-@ / C-Space).
        KeyCode::Char('@' | ' ') if ctrl && !alt => {
            st.mark = Some(*cursor);
            Edit::Moved
        }
        // Kill the region between mark and cursor into the kill buffer (C-w).
        KeyCode::Char('w') if ctrl && !alt => {
            if let Some(m) = mark {
                let (a, b) = (m.min(*cursor), m.max(*cursor));
                st.kill = chars[a..b].iter().collect();
                chars.drain(a..b);
                *cursor = a;
                rebuild(&chars, value);
            }
            st.mark = None;
            Edit::Modified
        }
        // Copy the region to the kill buffer without removing it (Alt-w).
        KeyCode::Char('w') if alt && !ctrl => {
            if let Some(m) = mark {
                let (a, b) = (m.min(*cursor), m.max(*cursor));
                st.kill = chars[a..b].iter().collect();
            }
            Edit::Moved
        }
        // Yank the kill buffer at the cursor (C-y).
        KeyCode::Char('y') if ctrl && !alt => {
            if !st.kill.is_empty() {
                let ins: Vec<char> = st.kill.chars().collect();
                let n = ins.len();
                chars.splice(*cursor..*cursor, ins);
                *cursor += n;
                rebuild(&chars, value);
            }
            st.mark = None;
            Edit::Modified
        }

        // -- Plain text (also AltGr, i.e. Ctrl+Alt, for composed characters) --
        KeyCode::Char(c) if ctrl == alt => {
            chars.insert(*cursor, c);
            *cursor += 1;
            rebuild(&chars, value);
            st.mark = None;
            Edit::Modified
        }
        _ => Edit::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    /// Drive a sequence against a private state, returning (value, cursor).
    fn run(initial: &str, cursor: usize, keys: &[KeyEvent]) -> (String, usize) {
        let mut v = initial.to_string();
        let mut c = cursor;
        let mut st = EditState::default();
        for key in keys {
            edit_key_with(&mut v, &mut c, &mut st, *key);
        }
        (v, c)
    }

    #[test]
    fn movement_bindings() {
        // C-a home, C-e end, C-b/C-f by char.
        let (_, c) = run("hello", 5, &[ctrl('a')]);
        assert_eq!(c, 0);
        let (_, c) = run("hello", 0, &[ctrl('e')]);
        assert_eq!(c, 5);
        let (_, c) = run("hello", 3, &[ctrl('b'), ctrl('b')]);
        assert_eq!(c, 1);
        let (_, c) = run("hello", 1, &[ctrl('f')]);
        assert_eq!(c, 2);
    }

    #[test]
    fn word_movement() {
        // Alt-b / Alt-f jump over words (non-alphanumeric runs are skipped).
        let (_, c) = run("foo bar baz", 11, &[alt('b')]);
        assert_eq!(c, 8, "back to start of 'baz'");
        let (_, c) = run("foo bar baz", 8, &[alt('b')]);
        assert_eq!(c, 4, "back to start of 'bar'");
        let (_, c) = run("foo bar", 0, &[alt('f')]);
        assert_eq!(c, 3, "forward past 'foo'");
        let (_, c) = run("foo bar", 3, &[alt('f')]);
        assert_eq!(c, 7, "forward past 'bar'");
    }

    #[test]
    fn deletion_bindings() {
        // C-h / Backspace delete the previous char; C-d / Delete the one at point.
        let (v, c) = run("hello", 3, &[ctrl('h')]);
        assert_eq!((v.as_str(), c), ("helo", 2));
        let (v, c) = run("hello", 3, &[k(KeyCode::Backspace)]);
        assert_eq!((v.as_str(), c), ("helo", 2));
        let (v, c) = run("hello", 1, &[ctrl('d')]);
        assert_eq!((v.as_str(), c), ("hllo", 1));
        let (v, c) = run("hello", 1, &[k(KeyCode::Delete)]);
        assert_eq!((v.as_str(), c), ("hllo", 1));
    }

    #[test]
    fn delete_word_backward() {
        let backspace_alt = KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT);
        let (v, c) = run("foo bar baz", 11, &[backspace_alt]);
        assert_eq!((v.as_str(), c), ("foo bar ", 8));
        // Alt-C-h does the same.
        let alt_ctrl_h = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::ALT | KeyModifiers::CONTROL);
        let (v, c) = run("foo bar baz", 11, &[alt_ctrl_h]);
        assert_eq!((v.as_str(), c), ("foo bar ", 8));
    }

    #[test]
    fn kill_to_end_and_yank() {
        // C-k kills to end of line; C-y yanks it back elsewhere.
        let mut v = "hello world".to_string();
        let mut c = 5;
        let mut st = EditState::default();
        edit_key_with(&mut v, &mut c, &mut st, ctrl('k'));
        assert_eq!(v, "hello");
        assert_eq!(st.kill, " world");
        // Move home and yank: the kill buffer is inserted at the cursor.
        edit_key_with(&mut v, &mut c, &mut st, ctrl('a'));
        edit_key_with(&mut v, &mut c, &mut st, ctrl('y'));
        assert_eq!(v, " worldhello");
        assert_eq!(c, 6);
    }

    #[test]
    fn mark_kill_region_and_copy() {
        // Set mark at 0, move to 5, C-w cuts "hello" into the kill buffer.
        let mut v = "hello world".to_string();
        let mut c = 0;
        let mut st = EditState::default();
        edit_key_with(&mut v, &mut c, &mut st, ctrl('@')); // set mark at 0
        edit_key_with(&mut v, &mut c, &mut st, ctrl('e')); // cursor to end (11)
        // move back to just after "hello" (index 5) via word-back then adjust:
        c = 5;
        edit_key_with(&mut v, &mut c, &mut st, ctrl('w')); // kill [0,5)
        assert_eq!(v, " world");
        assert_eq!(st.kill, "hello");
        assert_eq!(c, 0);

        // Alt-w copies without removing and keeps the text.
        let mut v = "abcdef".to_string();
        let mut c = 6;
        let mut st = EditState { mark: Some(2), ..Default::default() };
        edit_key_with(&mut v, &mut c, &mut st, alt('w'));
        assert_eq!(v, "abcdef", "copy leaves the text intact");
        assert_eq!(st.kill, "cdef");
    }

    #[test]
    fn typing_clears_the_mark() {
        let mut v = "abc".to_string();
        let mut c = 0;
        let mut st = EditState::default();
        edit_key_with(&mut v, &mut c, &mut st, ctrl('@'));
        assert_eq!(st.mark, Some(0));
        edit_key_with(&mut v, &mut c, &mut st, k(KeyCode::Char('x')));
        assert_eq!(v, "xabc");
        assert_eq!(st.mark, None, "an edit drops the mark");
    }
}
