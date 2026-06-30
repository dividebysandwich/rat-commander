//! Internal file viewer with text and hex modes, wrap toggle, and search.
//!
//! The content is exposed through a [`Source`] that is either a small in-memory
//! buffer or a **paged file on disk** — the latter never loads the whole file
//! into memory, reading only the bytes needed to render the current page or to
//! advance a search. Only a per-line offset index is kept (8 bytes per line).
//! Scrolling is by logical line (text) or 16-byte row (hex).

pub mod render;

use crate::syntax::{ColorRun, Highlighter};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

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

/// How the "Goto" dialog interprets its entered value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoMode {
    /// 1-based line number (text) or 16-byte row (hex).
    Line,
    /// Percentage through the file.
    Percent,
    /// Byte offset, entered in decimal.
    DecimalOffset,
    /// Byte offset, entered in hexadecimal.
    HexOffset,
}

/// Result of handling a key: whether the viewer should stay open.
pub enum ViewerSignal {
    Stay,
    Close,
    /// Ask the app to open the modal "Goto" dialog (F5).
    OpenGoto,
}

pub struct ViewerState {
    pub name: String,
    src: Source,
    truncated: bool,
    /// A temp file to delete when the viewer closes (a fetched remote file).
    temp: Option<PathBuf>,
    /// Incremental syntax highlighter (text mode only), when a syntax matched.
    hl: Option<Highlighter>,
    /// Byte offset of the start of each text line. Built lazily: only the lines
    /// within the first [`scanned`](Self::scanned) bytes are present until more
    /// is needed (scrolling, search, goto), so huge files open instantly.
    line_starts: Vec<usize>,
    /// Bytes scanned so far for `line_starts`; every newline in `[0, scanned)`
    /// has been recorded. Equals the file length once fully indexed.
    scanned: usize,
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
    /// Content body and footer (F-key bar) rects, recorded by the renderer for
    /// mouse hit-testing.
    content_area: Rect,
    footer_area: Rect,
}

impl ViewerState {
    /// An in-memory viewer (help text, or already-loaded small content).
    pub fn new(name: String, mut data: Vec<u8>) -> Self {
        let truncated = data.len() > MAX_VIEW_BYTES;
        if truncated {
            data.truncate(MAX_VIEW_BYTES);
        }
        let line_starts = compute_line_starts(&data);
        let scanned = data.len();
        ViewerState {
            name,
            src: Source::Mem(data),
            truncated,
            temp: None,
            hl: None,
            line_starts,
            scanned,
            mode: ViewMode::Text,
            wrap: false,
            top: 0,
            h_offset: 0,
            query: String::new(),
            search_input: None,
            last_match: None,
            view_rows: 1,
            view_cols: 1,
            content_area: Rect::default(),
            footer_area: Rect::default(),
        }
    }

    /// A file-backed (paged) viewer from an already-scanned file (built on the
    /// main thread so the blocking scan can run off-thread). When `temp` is set,
    /// that file is deleted on close.
    pub fn from_scanned(
        name: String,
        file: File,
        len: usize,
        line_starts: Vec<usize>,
        scanned: usize,
        temp: Option<PathBuf>,
    ) -> Self {
        ViewerState {
            name,
            src: Source::File { file: RefCell::new(file), len },
            truncated: false,
            temp,
            hl: None,
            line_starts,
            scanned,
            mode: ViewMode::Text,
            wrap: false,
            top: 0,
            h_offset: 0,
            query: String::new(),
            search_input: None,
            last_match: None,
            view_rows: 1,
            view_cols: 1,
            content_area: Rect::default(),
            footer_area: Rect::default(),
        }
    }

    /// Convenience: open + scan a file in one call (used by tests).
    pub fn open_file(name: String, path: PathBuf, temp: Option<PathBuf>) -> std::io::Result<Self> {
        let (file, len, line_starts, scanned) = scan_file(&path)?;
        Ok(Self::from_scanned(name, file, len, line_starts, scanned, temp))
    }

    /// Turn on syntax highlighting if a syntax matches the file name and the
    /// content is within the size cap. `dark` selects a fitting bundled theme.
    pub fn enable_syntax(&mut self, dark: bool) {
        if self.src.len() <= crate::syntax::HL_MAX_BYTES {
            self.hl = Highlighter::for_file(&self.name, dark);
        }
    }

    fn has_syntax(&self) -> bool {
        self.hl.is_some()
    }

    /// Color runs for line `li` (computing highlight up to it on demand). Empty
    /// when highlighting is off. Returns owned runs so the caller can also read
    /// the line text without a borrow conflict.
    fn line_runs(&mut self, li: usize) -> Vec<ColorRun> {
        let total = self.line_starts.len();
        let Some(hl) = self.hl.as_mut() else {
            return Vec::new();
        };
        // Disjoint field borrows: `hl` (self.hl) vs. self.src / self.line_starts.
        while hl.processed() <= li && hl.processed() < total {
            let i = hl.processed();
            let start = self.line_starts[i];
            let end = self
                .line_starts
                .get(i + 1)
                .map(|&s| s.saturating_sub(1))
                .unwrap_or_else(|| self.src.len());
            let mut bytes = self.src.read_range(start, end.max(start));
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            let display = String::from_utf8_lossy(&bytes).replace('\t', "    ");
            hl.process_next(&display);
        }
        hl.line(li).to_vec()
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

    /// Whether the whole file's line index has been built (so `line_count` is
    /// exact and the last line's extent is known).
    fn fully_indexed(&self) -> bool {
        self.scanned >= self.data_len()
    }

    /// Index one more chunk of the file, appending newline offsets. Returns
    /// `false` once fully indexed (a read error is treated as EOF so callers
    /// that loop can't spin).
    fn scan_one_chunk(&mut self) -> bool {
        let len = self.data_len();
        if self.scanned >= len {
            return false;
        }
        const CHUNK: usize = 256 * 1024;
        let end = (self.scanned + CHUNK).min(len);
        let buf = self.src.read_range(self.scanned, end);
        if buf.is_empty() {
            self.scanned = len; // give up rather than loop forever on a bad read
            return false;
        }
        for i in memchr::memchr_iter(b'\n', &buf) {
            self.line_starts.push(self.scanned + i + 1);
        }
        self.scanned += buf.len();
        true
    }

    /// Extend the index until at least `target` bytes have been scanned (or EOF).
    fn extend_to_byte(&mut self, target: usize) {
        let target = target.min(self.data_len());
        while self.scanned < target && self.scan_one_chunk() {}
    }

    /// Extend the index until logical line `target` is known (or EOF). Indexing
    /// one past the last visible line lets `line_str` find that line's end.
    fn extend_to_line(&mut self, target: usize) {
        while self.line_starts.len() <= target && self.scan_one_chunk() {}
    }

    /// Build the rest of the line index (for "go to end" / percentage jumps).
    fn index_fully(&mut self) {
        while self.scan_one_chunk() {}
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
            KeyCode::F(5) => return ViewerSignal::OpenGoto,
            KeyCode::F(7) => self.search_input = Some(String::new()),
            KeyCode::Char('n') => self.find_next(),
            KeyCode::Down => self.scroll(1),
            KeyCode::Up => self.scroll(-1),
            KeyCode::PageDown => self.scroll(self.view_rows as isize - 1),
            KeyCode::PageUp => self.scroll(-(self.view_rows as isize - 1)),
            KeyCode::Home => self.top = 0,
            KeyCode::End => {
                // The true last line is only known once the whole file is indexed.
                if self.mode == ViewMode::Text {
                    self.index_fully();
                }
                self.top = self.max_top();
            }
            KeyCode::Left => self.h_offset = self.h_offset.saturating_sub(8),
            KeyCode::Right => self.h_offset += 8,
            _ => {}
        }
        ViewerSignal::Stay
    }

    /// Route a mouse event. The wheel scrolls; a click in the lower half of the
    /// body scrolls down a page and the upper half scrolls up; the F-key bar
    /// acts as buttons.
    pub fn handle_mouse(&mut self, ev: MouseEvent) -> ViewerSignal {
        let (col, row) = (ev.column, ev.row);

        // F-key bar clicks (when not capturing a search query).
        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left))
            && row == self.footer_area.y
            && self.search_input.is_none()
        {
            let labels = self.footer_labels();
            return match crate::ui::fkeys::index_at(self.footer_area, &labels, col, row) {
                Some(i) => self.activate_fkey(i),
                None => ViewerSignal::Stay,
            };
        }

        match ev.kind {
            MouseEventKind::ScrollDown => self.scroll(3),
            MouseEventKind::ScrollUp => self.scroll(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                let a = self.content_area;
                let inside = a.height > 0
                    && row >= a.y
                    && row < a.y + a.height
                    && col >= a.x
                    && col < a.x + a.width;
                if inside {
                    // Below the vertical center pages down; above it pages up.
                    let mid = a.y + a.height / 2;
                    let page = (self.view_rows as isize - 1).max(1);
                    self.scroll(if row >= mid { page } else { -page });
                }
            }
            _ => {}
        }
        ViewerSignal::Stay
    }

    /// The F-key bar labels for the current mode (kept in sync with the footer
    /// renderer, which calls this).
    pub(crate) fn footer_labels(&self) -> [&'static str; 10] {
        let wrap = if self.wrap { "Unwrap" } else { "Wrap" };
        let mode = if self.mode == ViewMode::Hex { "Text" } else { "Hex" };
        ["Help", wrap, "Quit", mode, "Goto", "", "Search", "", "Next", "Quit"]
    }

    /// Perform the action of F-key index `i` (0-based) from a bar click.
    fn activate_fkey(&mut self, i: usize) -> ViewerSignal {
        let code = match i {
            1 => KeyCode::F(2),               // Wrap / Unwrap
            2 | 9 => return ViewerSignal::Close, // Quit
            3 => KeyCode::F(4),               // Text / Hex
            4 => return ViewerSignal::OpenGoto, // Goto
            6 => KeyCode::F(7),               // Search
            8 => KeyCode::Char('n'),          // Next match
            _ => return ViewerSignal::Stay,   // Help / empty: no-op
        };
        self.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
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
        let target = (self.top as isize + delta).max(0) as usize;
        // Index far enough that `target` is reachable (and a page is renderable).
        if self.mode == ViewMode::Text {
            self.extend_to_line(target + self.view_rows);
        }
        self.top = target.min(self.max_top());
    }

    /// Jump to a position given by the Goto dialog. Returns whether the input
    /// parsed (so the caller can flag bad input). In text mode positions are
    /// logical lines; in hex mode they are 16-byte rows.
    pub fn goto(&mut self, value: &str, mode: GotoMode) -> bool {
        let v = value.trim();
        let text = self.mode == ViewMode::Text;
        // Parse first, then extend the index as far as the target needs before
        // computing the top row.
        let target = match mode {
            GotoMode::Line => {
                let Ok(n) = v.parse::<usize>() else { return false };
                let line = n.saturating_sub(1);
                if text {
                    self.extend_to_line(line);
                }
                line
            }
            GotoMode::Percent => {
                let Ok(p) = v.parse::<f64>() else { return false };
                // A percentage needs the exact total, so finish indexing.
                if text {
                    self.index_fully();
                }
                let total = match self.mode {
                    ViewMode::Text => self.line_count(),
                    ViewMode::Hex => self.hex_rows(),
                };
                ((total.saturating_sub(1)) as f64 * p.clamp(0.0, 100.0) / 100.0).round() as usize
            }
            GotoMode::DecimalOffset => {
                let Ok(off) = v.parse::<usize>() else { return false };
                if text {
                    self.extend_to_byte(off + 1);
                }
                self.offset_to_top(off)
            }
            GotoMode::HexOffset => {
                let hex = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).unwrap_or(v);
                let Ok(off) = usize::from_str_radix(hex, 16) else { return false };
                if text {
                    self.extend_to_byte(off + 1);
                }
                self.offset_to_top(off)
            }
        };
        self.top = target.min(self.max_top());
        true
    }

    /// Map a byte offset to the top index for the current mode.
    fn offset_to_top(&self, off: usize) -> usize {
        match self.mode {
            ViewMode::Text => self.byte_to_line(off),
            ViewMode::Hex => off / 16,
        }
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
                ViewMode::Text => {
                    // The match may lie beyond the indexed region; index up to it.
                    self.extend_to_byte(off + 1);
                    self.top = self.byte_to_line(off);
                }
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
    for i in memchr::memchr_iter(b'\n', data) {
        starts.push(i + 1);
    }
    starts
}

/// Bytes scanned up-front when a file is opened. The rest of the line index is
/// built lazily as the user scrolls/searches, so even multi-gigabyte files open
/// instantly instead of waiting for a full newline scan.
const INITIAL_SCAN: usize = 1024 * 1024;

/// Open a file and build the *initial* slice of its line-start index (offsets
/// only). Returns the open handle, byte length, the partial index, and how many
/// bytes were scanned — all `Send`, so it can run in `spawn_blocking` and the
/// (non-`Send`) [`ViewerState`] is then assembled on the main thread. The viewer
/// extends the index on demand from there.
pub fn scan_file(path: &Path) -> std::io::Result<(File, usize, Vec<usize>, usize)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len() as usize;
    let (line_starts, scanned) = scan_line_starts(&mut file, INITIAL_SCAN.min(len))?;
    Ok((file, len, line_starts, scanned))
}

/// Scan up to `budget` bytes from the start of `file`, recording newline offsets.
/// Returns the partial index and the number of bytes actually scanned. Only
/// newline offsets are recorded — the file content is never held in memory.
fn scan_line_starts(file: &mut File, budget: usize) -> std::io::Result<(Vec<usize>, usize)> {
    let mut starts = vec![0usize];
    if budget == 0 {
        return Ok((starts, 0));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut off = 0usize;
    while off < budget {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for i in memchr::memchr_iter(b'\n', &buf[..n]) {
            starts.push(off + i + 1);
        }
        off += n;
    }
    Ok((starts, off))
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
    fn lazy_index_builds_on_demand() {
        // A file large enough that a single read can't swallow it, so a small
        // initial budget leaves the index genuinely partial. Fixed-width lines
        // ("00000000\n" = 9 bytes) make offsets predictable.
        const N: usize = 80_000; // ~720 KB
        let mut bytes = Vec::with_capacity(N * 9);
        for i in 0..N {
            bytes.extend_from_slice(format!("{i:08}\n").as_bytes());
        }
        let path = std::env::temp_dir().join(format!("rc_viewer_lazy_{}.txt", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();

        let make = |budget: usize| {
            let mut f = File::open(&path).unwrap();
            let len = f.metadata().unwrap().len() as usize;
            let (starts, scanned) = scan_line_starts(&mut f, budget).unwrap();
            ViewerState::from_scanned("t".into(), f, len, starts, scanned, None)
        };

        // Open with only a tiny budget: the index starts as a short prefix.
        let mut v = make(100 * 1024);
        assert!(!v.fully_indexed(), "opens without scanning the whole file");
        assert!(v.line_count() < N, "only a prefix is indexed ({})", v.line_count());
        assert_eq!(v.line_str(0), "00000000");

        // Scrolling/extension reveals deeper lines with correct content.
        v.extend_to_line(70_000);
        assert_eq!(v.line_str(70_000), format!("{:08}", 70_000));

        // Goto by line extends as far as needed.
        assert!(v.goto("75000", GotoMode::Line));
        assert_eq!(v.top, 74_999);
        assert_eq!(v.line_str(74_999), format!("{:08}", 74_999));

        // A byte-offset goto maps to the right line after extending.
        assert!(v.goto(&format!("{}", 9 * 60_000), GotoMode::DecimalOffset));
        assert_eq!(v.top, 60_000);

        // Finishing the index yields the exact total (N lines + the empty line
        // after the trailing newline).
        v.index_fully();
        assert!(v.fully_indexed());
        assert_eq!(v.line_count(), N + 1);
        assert_eq!(v.line_str(N - 1), format!("{:08}", N - 1));

        // A percentage jump triggers a full scan and lands on the last line.
        let mut v2 = make(100 * 1024);
        assert!(!v2.fully_indexed());
        assert!(v2.goto("100", GotoMode::Percent));
        assert!(v2.fully_indexed(), "percent jump finishes indexing");
        assert_eq!(v2.top, N);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn small_file_is_fully_indexed_on_open() {
        let path = std::env::temp_dir().join(format!("rc_viewer_small_{}.txt", std::process::id()));
        std::fs::write(&path, b"a\nb\nc\n").unwrap();
        let v = ViewerState::open_file("t".into(), path.clone(), None).unwrap();
        assert!(v.fully_indexed());
        assert_eq!(v.line_count(), 4); // a, b, c, trailing empty
        assert_eq!(v.line_str(1), "b");
        std::fs::remove_file(&path).ok();
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE }
    }

    /// Body at rows 1..11, F-key bar at row 12, as the renderer would record.
    fn with_layout(v: &mut ViewerState) {
        v.content_area = Rect::new(0, 1, 40, 10);
        v.footer_area = Rect::new(0, 12, 40, 1);
        v.view_rows = 10;
        v.view_cols = 40;
    }

    fn many_lines(n: usize) -> Vec<u8> {
        (0..n).map(|i| format!("line{i}\n")).collect::<String>().into_bytes()
    }

    #[test]
    fn click_below_center_pages_down_above_pages_up() {
        let mut v = ViewerState::new("t".into(), many_lines(100));
        with_layout(&mut v);
        assert_eq!(v.top, 0);
        // Center of the body is row 6; a click below it pages down (view_rows-1).
        v.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 8));
        assert_eq!(v.top, 9, "click below center pages down");
        // A click above center pages back up.
        v.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 2));
        assert_eq!(v.top, 0, "click above center pages up");
    }

    #[test]
    fn wheel_scrolls_the_view() {
        let mut v = ViewerState::new("t".into(), many_lines(100));
        with_layout(&mut v);
        v.handle_mouse(mouse(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(v.top, 3, "wheel down scrolls three lines");
        v.handle_mouse(mouse(MouseEventKind::ScrollUp, 5, 5));
        assert_eq!(v.top, 0, "wheel up scrolls three lines back");
    }

    #[test]
    fn fkey_bar_click_runs_the_function() {
        let mut v = ViewerState::new("t".into(), many_lines(10));
        with_layout(&mut v);
        // Footer width 40, 10 labels → 4 cells each; F4 (Hex) spans cols 12-15.
        assert_eq!(v.mode, ViewMode::Text);
        v.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 12, 12));
        assert_eq!(v.mode, ViewMode::Hex, "clicking F4 toggles hex mode");
        // F10 (Quit) spans cols 36-39 → closes.
        let sig = v.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 36, 12));
        assert!(matches!(sig, ViewerSignal::Close), "clicking F10 quits");
    }

    /// `n` lines of exactly 5 bytes each ("aaaa\n"), so byte offsets are tidy.
    fn fixed_lines(n: usize) -> Vec<u8> {
        (0..n).map(|_| "aaaa\n").collect::<String>().into_bytes()
    }

    #[test]
    fn goto_text_mode_line_percent_and_offsets() {
        let mut v = ViewerState::new("t".into(), fixed_lines(20)); // 21 line starts
        assert!(v.goto("10", GotoMode::Line));
        assert_eq!(v.top, 9, "1-based line number");
        assert!(v.goto("50", GotoMode::Percent));
        assert_eq!(v.top, 10, "50% of 20 = line 10");
        assert!(v.goto("5", GotoMode::DecimalOffset));
        assert_eq!(v.top, 1, "byte 5 is the start of line 1");
        assert!(v.goto("a", GotoMode::HexOffset));
        assert_eq!(v.top, 2, "0x0a = byte 10 → line 2");
        assert!(v.goto("0x0F", GotoMode::HexOffset));
        assert_eq!(v.top, 3, "0x0f = byte 15 → line 3");
        // Out-of-range clamps; garbage is rejected.
        assert!(v.goto("9999", GotoMode::Line));
        assert_eq!(v.top, v.max_top());
        assert!(!v.goto("nope", GotoMode::DecimalOffset));
    }

    #[test]
    fn goto_hex_mode_uses_rows() {
        let mut v = ViewerState::new("t".into(), vec![0u8; 100]); // 7 rows of 16
        v.mode = ViewMode::Hex;
        assert!(v.goto("3", GotoMode::Line));
        assert_eq!(v.top, 2, "line number is a 16-byte row in hex mode");
        assert!(v.goto("32", GotoMode::DecimalOffset));
        assert_eq!(v.top, 2, "byte 32 is row 2");
        assert!(v.goto("20", GotoMode::HexOffset));
        assert_eq!(v.top, 2, "0x20 = 32 → row 2");
    }

    #[test]
    fn f5_and_goto_label_click_request_the_dialog() {
        let mut v = ViewerState::new("t".into(), fixed_lines(10));
        with_layout(&mut v);
        assert!(matches!(v.handle_key(super::KeyEvent::new(super::KeyCode::F(5), super::KeyModifiers::NONE)), ViewerSignal::OpenGoto));
        // The "Goto" label is F5 (index 4): cols 16-19 on a 40-wide bar.
        assert!(matches!(
            v.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 16, 12)),
            ViewerSignal::OpenGoto
        ));
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

    #[test]
    fn hex_color_tints_the_hash() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut v = ViewerState::new("t".into(), b"x #ff501a y".to_vec());
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(40, 6)).unwrap();
        t.draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme)).unwrap();
        let b = t.backend().buffer();
        let hash = (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .find(|&(x, y)| b[(x, y)].symbol() == "#")
            .expect("'#' rendered");
        assert_eq!(
            b[hash].fg,
            ratatui::style::Color::Rgb(0xff, 0x50, 0x1a),
            "hash tinted with its color"
        );
    }

    #[test]
    fn syntax_highlight_colors_the_body() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rc_hl_{}_{nanos}.rs", std::process::id()));
        std::fs::write(&path, b"fn main() { let x = 1; }\n").unwrap();

        let mut v = ViewerState::open_file("a.rs".into(), path.clone(), Some(path.clone())).unwrap();
        v.enable_syntax(true);
        assert!(v.has_syntax(), "rust syntax should be detected");

        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(80, 6)).unwrap();
        t.draw(|f| render::render(f, f.area(), &mut v, &theme)).unwrap();
        let b = t.backend().buffer();

        // Body is on row 1 (row 0 is the header). Collect text + distinct colors.
        let mut text = String::new();
        let mut colors = std::collections::HashSet::new();
        for x in 0..b.area.width {
            let cell = &b[(x, 1)];
            text.push_str(cell.symbol());
            colors.insert(format!("{:?}", cell.fg));
        }
        assert!(text.contains("fn main"), "code is rendered");
        assert!(colors.len() > 1, "highlighting uses more than one color");

        drop(v);
    }
}
