//! The editor text buffer: a `ropey` rope with an op-log undo/redo stack.
//!
//! Every mutation flows through [`EditorBuffer::replace_range`], which records
//! the removed and inserted text so undo/redo are exact inverses. Higher-level
//! operations (insert char, delete, block ops, paste) are expressed in terms of
//! it. All indices are *character* indices.

use ropey::Rope;

/// A single reversible edit: at char `at`, `removed` text was replaced by
/// `inserted` text.
#[derive(Debug, Clone)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
}

pub struct EditorBuffer {
    rope: Rope,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl EditorBuffer {
    pub fn from_str(text: &str) -> Self {
        EditorBuffer {
            rope: Rope::from_str(text),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// The full buffer contents as a string.
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx.min(self.rope.len_chars()))
    }

    pub fn line_to_char(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.rope.len_lines().saturating_sub(1)))
    }

    /// Character at `idx`, or `None` at/after the end.
    pub fn char_at(&self, idx: usize) -> Option<char> {
        if idx < self.rope.len_chars() {
            Some(self.rope.char(idx))
        } else {
            None
        }
    }

    /// Number of characters on `line`, excluding the trailing newline.
    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        // Strip a trailing '\n' (and '\r' before it).
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        len
    }

    /// Text of `line` without its trailing newline.
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let start = self.rope.line_to_char(line);
        let end = start + self.line_len(line);
        self.rope.slice(start..end).to_string()
    }

    /// Slice `[start, end)` as an owned string.
    pub fn slice(&self, start: usize, end: usize) -> String {
        let len = self.rope.len_chars();
        let (s, e) = (start.min(len), end.min(len));
        self.rope.slice(s..e).to_string()
    }

    /// Replace `[start, end)` with `text`, recording undo. Returns the char
    /// index just past the inserted text (a natural new cursor position).
    pub fn replace_range(&mut self, start: usize, end: usize, text: &str) -> usize {
        let len = self.rope.len_chars();
        let (start, end) = (start.min(len), end.min(len));
        let removed = self.rope.slice(start..end).to_string();
        if start != end {
            self.rope.remove(start..end);
        }
        if !text.is_empty() {
            self.rope.insert(start, text);
        }
        self.undo.push(Edit {
            at: start,
            removed,
            inserted: text.to_string(),
        });
        self.redo.clear();
        start + text.chars().count()
    }

    /// Insert `text` at `at`. Returns the new cursor position.
    pub fn insert(&mut self, at: usize, text: &str) -> usize {
        self.replace_range(at, at, text)
    }

    /// Delete `[start, end)`. Returns `start`.
    pub fn delete(&mut self, start: usize, end: usize) -> usize {
        self.replace_range(start, end, "");
        start
    }

    /// Undo the last edit. Returns the cursor position to restore, if any.
    pub fn undo(&mut self) -> Option<usize> {
        let e = self.undo.pop()?;
        let ins_len = e.inserted.chars().count();
        self.rope.remove(e.at..e.at + ins_len);
        if !e.removed.is_empty() {
            self.rope.insert(e.at, &e.removed);
        }
        let cursor = e.at + e.removed.chars().count();
        self.redo.push(e);
        Some(cursor)
    }

    /// Redo the last undone edit. Returns the cursor position, if any.
    pub fn redo(&mut self) -> Option<usize> {
        let e = self.redo.pop()?;
        let rem_len = e.removed.chars().count();
        self.rope.remove(e.at..e.at + rem_len);
        if !e.inserted.is_empty() {
            self.rope.insert(e.at, &e.inserted);
        }
        let cursor = e.at + e.inserted.chars().count();
        self.undo.push(e);
        Some(cursor)
    }

    /// Find `needle` (case-sensitive) at or after `from`; returns char index.
    pub fn find(&self, needle: &str, from: usize) -> Option<usize> {
        if needle.is_empty() {
            return None;
        }
        // Rope -> String search is simple and fine for typical edit sizes.
        let text = self.rope.to_string();
        let chars: Vec<char> = text.chars().collect();
        let needle: Vec<char> = needle.chars().collect();
        if needle.len() > chars.len() {
            return None;
        }
        let last = chars.len() - needle.len();
        (from.min(last + 1)..=last).find(|&i| chars[i..i + needle.len()] == needle[..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_delete_and_undo_redo() {
        let mut b = EditorBuffer::from_str("hello");
        let c = b.insert(5, " world");
        assert_eq!(b.text(), "hello world");
        assert_eq!(c, 11);

        b.delete(0, 6); // remove "hello "
        assert_eq!(b.text(), "world");

        assert_eq!(b.undo(), Some(6)); // restores "hello "
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.undo(), Some(5)); // removes " world"
        assert_eq!(b.text(), "hello");

        assert_eq!(b.redo(), Some(11));
        assert_eq!(b.text(), "hello world");
    }

    #[test]
    fn line_metrics() {
        let b = EditorBuffer::from_str("ab\ncde\nf");
        assert_eq!(b.len_lines(), 3);
        assert_eq!(b.line_len(0), 2);
        assert_eq!(b.line_len(1), 3);
        assert_eq!(b.line_text(1), "cde");
        assert_eq!(b.char_to_line(4), 1);
    }

    #[test]
    fn find_substring() {
        let b = EditorBuffer::from_str("the cat sat on the mat");
        assert_eq!(b.find("the", 0), Some(0));
        assert_eq!(b.find("the", 1), Some(15));
        assert_eq!(b.find("dog", 0), None);
    }
}
