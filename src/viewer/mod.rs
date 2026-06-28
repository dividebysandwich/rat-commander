//! Internal file viewer with text and hex modes, wrap toggle, and search.
//!
//! The content is exposed through a [`Source`] that is either a small in-memory
//! buffer or a **paged file on disk** — the latter never loads the whole file
//! into memory, reading only the bytes needed to render the current page or to
//! advance a search. Only a per-line offset index is kept (8 bytes per line).
//! Scrolling is by logical line (text) or 16-byte row (hex).

pub mod render;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Maximum bytes read into an *in-memory* viewer (larger in-memory buffers are
/// truncated with a note). File-backed sources are paged and never truncated.
pub const MAX_VIEW_BYTES: usize = 64 * 1024 * 1024;

/// Where the viewer reads bytes from.
enum Source {
    /// Small content held in memory (help text, remote-less small files).
    Mem(Vec<u8>),
    /// A seekable local file, read on demand (never fully loaded).
    File { file: RefCell<File>, len: usize },
}

impl Source {
    fn len(&self) -> usize {
        match self {
            Source::Mem(d) => d.len(),
            Source::File { len, .. } => *len,
        }
    }

    /// Read bytes `[start, end)` (clamped to the source length). Short/failed
    /// reads return what was obtained; callers tolerate partial results.
    fn read_range(&self, start: usize, end: usize) -> Vec<u8> {
        let end = end.min(self.len());
        if start >= end {
            return Vec::new();
        }
        match self {
            Source::Mem(d) => d[start..end].to_vec(),
            Source::File { file, .. } => {
                let mut f = file.borrow_mut();
                if f.seek(SeekFrom::Start(start as u64)).is_err() {
                    return Vec::new();
                }
                let mut buf = vec![0u8; end - start];
                let mut read = 0;
                while read < buf.len() {
                    match f.read(&mut buf[read..]) {
                        Ok(0) => break,
                        Ok(n) => read += n,
                        Err(_) => break,
                    }
                }
                buf.truncate(read);
                buf
            }
        }
    }
}

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
    src: Source,
    truncated: bool,
    /// A temp file to delete when the viewer closes (a fetched remote file).
    temp: Option<PathBuf>,
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
    /// An in-memory viewer (help text, or already-loaded small content).
    pub fn new(name: String, mut data: Vec<u8>) -> Self {
        let truncated = data.len() > MAX_VIEW_BYTES;
        if truncated {
            data.truncate(MAX_VIEW_BYTES);
        }
        let line_starts = compute_line_starts(&data);
        ViewerState {
            name,
            src: Source::Mem(data),
            truncated,
            temp: None,
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

    /// A file-backed (paged) viewer. The file is opened and its line-start index
    /// built by a single sequential scan (offsets only — the content is never
    /// held in memory). When `temp` is set, that file is deleted on close.
    pub fn open_file(name: String, path: PathBuf, temp: Option<PathBuf>) -> std::io::Result<Self> {
        let mut file = File::open(&path)?;
        let len = file.metadata()?.len() as usize;
        let line_starts = scan_line_starts(&mut file, len)?;
        Ok(ViewerState {
            name,
            src: Source::File { file: RefCell::new(file), len },
            truncated: false,
            temp,
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
        })
    }

    /// 16 bytes (or fewer at EOF) of the hex row starting at byte `off`.
    pub(crate) fn hex_row(&self, off: usize) -> Vec<u8> {
        self.src.read_range(off, off + 16)
    }

    fn data_len(&self) -> usize {
        self.src.len()
    }

    pub fn is_searching(&self) -> bool {
        self.search_input.is_some()
    }

    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn hex_rows(&self) -> usize {
        self.data_len().div_ceil(16)
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

    /// Case-insensitive search from byte `start`, reading the source in
    /// overlapping windows so file-backed sources never load fully into memory.
    fn find_from(&self, start: usize) -> Option<usize> {
        let ql: Vec<u8> = self.query.bytes().map(|b| b.to_ascii_lowercase()).collect();
        let len = self.data_len();
        if ql.is_empty() || ql.len() > len {
            return None;
        }
        const WINDOW: usize = 256 * 1024;
        let overlap = ql.len() - 1;
        let mut pos = start.min(len);
        while pos + ql.len() <= len {
            let end = (pos + WINDOW).min(len);
            let buf = self.src.read_range(pos, end);
            if buf.len() >= ql.len() {
                let last = buf.len() - ql.len();
                if let Some(i) = (0..=last).find(|&i| {
                    buf[i..i + ql.len()]
                        .iter()
                        .zip(&ql)
                        .all(|(a, b)| a.to_ascii_lowercase() == *b)
                }) {
                    return Some(pos + i);
                }
            }
            if end == len {
                break;
            }
            pos = end - overlap; // overlap so matches spanning a window boundary are found
        }
        None
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
            .unwrap_or_else(|| self.data_len());
        let mut bytes = self.src.read_range(start, end.max(start));
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        String::from_utf8_lossy(&bytes).replace('\t', "    ")
    }
}

impl Drop for ViewerState {
    fn drop(&mut self) {
        if let Some(path) = self.temp.take() {
            let _ = std::fs::remove_file(path);
        }
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

/// Build the line-start index for a file by scanning it sequentially in chunks.
/// Only newline offsets are recorded — the file content is never held in memory.
fn scan_line_starts(file: &mut File, len: usize) -> std::io::Result<Vec<usize>> {
    let mut starts = vec![0usize];
    if len == 0 {
        return Ok(starts);
    }
    file.seek(SeekFrom::Start(0))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut off = 0usize;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for (i, &b) in buf[..n].iter().enumerate() {
            if b == b'\n' {
                starts.push(off + i + 1);
            }
        }
        off += n;
    }
    Ok(starts)
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

    #[test]
    fn file_backed_viewer_pages_without_loading_all() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rc_view_{}_{nanos}", std::process::id()));
        std::fs::write(&path, b"alpha\nbeta\r\nNEEDLE here\ngamma").unwrap();

        let mut v = ViewerState::open_file("t".into(), path.clone(), Some(path.clone())).unwrap();
        // Index and on-demand reads work the same as the in-memory viewer.
        assert_eq!(v.line_count(), 4);
        assert_eq!(v.line_str(0), "alpha");
        assert_eq!(v.line_str(1), "beta"); // CR stripped
        assert!(matches!(v.src, Source::File { .. }), "uses a paged file source");

        // Search reads through the file (windowed), not a memory copy.
        v.query = "needle".into();
        v.find_next();
        assert_eq!(v.top, 2, "case-insensitive match maps to its line");

        // Hex row reads the requested 16-byte window on demand.
        assert_eq!(&v.hex_row(0)[..5], b"alpha");

        // The temp file is removed when the viewer is dropped.
        drop(v);
        assert!(!path.exists(), "temp file cleaned up on close");
    }
}
