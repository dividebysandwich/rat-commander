//! Internal file viewer with text and hex modes, wrap toggle, and search.
//!
//! The whole file is read into memory (capped) via the VFS, so the viewer
//! works over any backend. Scrolling is by logical line (text) or 16-byte row
//! (hex).

pub mod render;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// Maximum bytes read into the viewer (larger files are truncated with a note).
pub const MAX_VIEW_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Text,
    Hex,
}

/// Result of handling a key: whether the viewer should stay open.
pub enum ViewerSignal {
    Stay,
    Close,
}

pub struct ViewerState {
    pub name: String,
    data: Vec<u8>,
    truncated: bool,
    /// Byte offset of the start of each text line.
    line_starts: Vec<usize>,
    pub mode: ViewMode,
    pub wrap: bool,
    /// Top visible logical line (text) or top 16-byte row (hex).
    top: usize,
    /// Horizontal scroll (text, non-wrap).
    h_offset: usize,
    /// Current search query.
    query: String,
    /// When `Some`, the F7 search prompt is capturing input.
    search_input: Option<String>,
    /// Byte offset of the last match (for "find next").
    last_match: Option<usize>,
    /// Viewport size, updated by the renderer each frame.
    view_rows: usize,
    view_cols: usize,
}

impl ViewerState {
    pub fn new(name: String, mut data: Vec<u8>) -> Self {
        let truncated = data.len() > MAX_VIEW_BYTES;
        if truncated {
            data.truncate(MAX_VIEW_BYTES);
        }
        let line_starts = compute_line_starts(&data);
        ViewerState {
            name,
            data,
            truncated,
            line_starts,
            mode: ViewMode::Text,
            wrap: false,
            top: 0,
            h_offset: 0,
            query: String::new(),
            search_input: None,
            last_match: None,
            view_rows: 1,
            view_cols: 1,
        }
    }

    pub fn is_searching(&self) -> bool {
        self.search_input.is_some()
    }

    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn hex_rows(&self) -> usize {
        self.data.len().div_ceil(16)
    }

    fn max_top(&self) -> usize {
        let total = match self.mode {
            ViewMode::Text => self.line_count(),
            ViewMode::Hex => self.hex_rows(),
        };
        total.saturating_sub(1)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ViewerSignal {
        // Search prompt captures input first.
        if self.search_input.is_some() {
            self.handle_search_key(key);
            return ViewerSignal::Stay;
        }

        match key.code {
            KeyCode::F(10) | KeyCode::Esc | KeyCode::Char('q') => return ViewerSignal::Close,
            KeyCode::F(2) => self.wrap = !self.wrap,
            KeyCode::F(4) => {
                self.mode = match self.mode {
                    ViewMode::Text => ViewMode::Hex,
                    ViewMode::Hex => ViewMode::Text,
                };
                self.top = self.top.min(self.max_top());
            }
            KeyCode::F(7) => self.search_input = Some(String::new()),
            KeyCode::Char('n') => self.find_next(),
            KeyCode::Down => self.scroll(1),
            KeyCode::Up => self.scroll(-1),
            KeyCode::PageDown => self.scroll(self.view_rows as isize - 1),
            KeyCode::PageUp => self.scroll(-(self.view_rows as isize - 1)),
            KeyCode::Home => self.top = 0,
            KeyCode::End => self.top = self.max_top(),
            KeyCode::Left => self.h_offset = self.h_offset.saturating_sub(8),
            KeyCode::Right => self.h_offset += 8,
            _ => {}
        }
        ViewerSignal::Stay
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.search_input = None,
            KeyCode::Enter => {
                if let Some(q) = self.search_input.take() {
                    self.query = q;
                    self.last_match = None;
                    self.find_next();
                }
            }
            KeyCode::Backspace => {
                if let Some(q) = self.search_input.as_mut() {
                    q.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(q) = self.search_input.as_mut() {
                    q.push(c);
                }
            }
            _ => {}
        }
    }

    fn scroll(&mut self, delta: isize) {
        let max = self.max_top() as isize;
        self.top = (self.top as isize + delta).clamp(0, max.max(0)) as usize;
    }

    /// Find the next occurrence of the query after the last match.
    fn find_next(&mut self) {
        if self.query.is_empty() {
            return;
        }
        let start = self.last_match.map(|m| m + 1).unwrap_or(0);
        let found = self.find_from(start).or_else(|| self.find_from(0));
        if let Some(off) = found {
            self.last_match = Some(off);
            match self.mode {
                ViewMode::Text => self.top = self.byte_to_line(off),
                ViewMode::Hex => self.top = off / 16,
            }
        }
    }

    fn find_from(&self, start: usize) -> Option<usize> {
        let q = self.query.as_bytes();
        if q.is_empty() || q.len() > self.data.len() {
            return None;
        }
        let ql: Vec<u8> = q.iter().map(|b| b.to_ascii_lowercase()).collect();
        let last = self.data.len() - ql.len();
        (start..=last).find(|&i| {
            self.data[i..i + ql.len()]
                .iter()
                .zip(&ql)
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
        })
    }

    fn byte_to_line(&self, off: usize) -> usize {
        match self.line_starts.binary_search(&off) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// Text of logical line `i`, with tabs expanded and CR stripped.
    fn line_str(&self, i: usize) -> String {
        let start = self.line_starts[i];
        let end = self
            .line_starts
            .get(i + 1)
            .map(|&s| s.saturating_sub(1)) // drop the '\n'
            .unwrap_or(self.data.len());
        let mut bytes = &self.data[start..end.max(start)];
        if bytes.last() == Some(&b'\r') {
            bytes = &bytes[..bytes.len() - 1];
        }
        String::from_utf8_lossy(bytes).replace('\t', "    ")
    }
}

/// Byte offsets where each text line begins (line 0 always starts at 0).
fn compute_line_starts(data: &[u8]) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    if data.is_empty() {
        starts = vec![0];
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_indexing_and_content() {
        let v = ViewerState::new("t".into(), b"alpha\nbeta\r\ngamma".to_vec());
        assert_eq!(v.line_count(), 3);
        assert_eq!(v.line_str(0), "alpha");
        assert_eq!(v.line_str(1), "beta"); // CR stripped
        assert_eq!(v.line_str(2), "gamma");
    }

    #[test]
    fn search_finds_and_maps_to_line() {
        let mut v = ViewerState::new("t".into(), b"one\ntwo\nthree\nTWO".to_vec());
        v.query = "two".into();
        v.find_next();
        // Case-insensitive: first match is on line 1 ("two").
        assert_eq!(v.top, 1);
        // Next wraps forward to the uppercase TWO on line 3.
        v.find_next();
        assert_eq!(v.top, 3);
    }

    #[test]
    fn hex_rows_count() {
        let v = ViewerState::new("t".into(), vec![0u8; 33]);
        assert_eq!(v.hex_rows(), 3); // ceil(33/16)
    }
}
