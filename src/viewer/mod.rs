//! Internal file viewer with text and hex modes, wrap toggle, and search.
//!
//! The content is exposed through a [`Source`] that is either a small in-memory
//! buffer or a **paged file on disk** — the latter never loads the whole file
//! into memory, reading only the bytes needed to render the current page or to
//! advance a search. Only a per-line offset index is kept (8 bytes per line).
//! Scrolling is by logical line (text) or 16-byte row (hex).

pub mod markdown;
mod search;
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

/// Most lines a single "Find all" will mark in the viewer. It pages files far
/// larger than the editor ever opens, so the sweep is bounded rather than
/// promising to mark every hit in a multi-gigabyte log.
const FOUND_LINES_MAX: usize = 50_000;

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

/// A decoded image shown fullscreen when F3 opens a supported image file. Falls
/// back to the raw text/hex view when a file can't be decoded, or via F8.
pub struct ViewerImage {
    /// The image scaled down for display (aspect preserved).
    pub img: image::RgbaImage,
    /// Cheap content signature for the graphics cache.
    pub sig: u64,
    /// Original pixel dimensions (before scaling), shown in the header.
    pub orig: (u32, u32),
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
/// A search as the dialog set it up. Repeating an identical one resumes from the
/// last hit; changing any field restarts from the top.
#[derive(Default, Clone, PartialEq, Eq)]
struct ViewSearch {
    query: String,
    regex: bool,
    case_sensitive: bool,
    whole_words: bool,
    backwards: bool,
    hex: bool,
}

pub enum ViewerSignal {
    Stay,
    Close,
    /// Ask the app to open the modal "Goto" dialog (F5).
    OpenGoto,
    /// Ask the app to open the modal search dialog (F7) — the same one the
    /// editor uses, so the viewer offers the same modes and options.
    OpenSearch,
    /// Ask the app to open the embedded user manual (F1), like the panel F1.
    OpenHelp,
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
    /// The file looks like Markdown (by extension), so the Markdown render mode
    /// and its F8 Raw/Render toggle are offered.
    is_markdown: bool,
    /// In text mode, whether to draw the Markdown approximation (true) or the raw
    /// text with syntax highlighting (false). Only meaningful when `is_markdown`.
    markdown_render: bool,
    pub wrap: bool,
    /// Top visible logical line (text) or top 16-byte row (hex).
    top: usize,
    /// Horizontal scroll (text, non-wrap).
    h_offset: usize,
    /// The search being repeated, exactly as the dialog set it up. `find_next`
    /// resumes from `last_match` while this is unchanged, so pressing F7-Enter on
    /// the same term walks the file; changing any option restarts from the top.
    search: ViewSearch,
    /// Term the search dialog opens pre-filled with — the last committed search,
    /// seeded from the app-wide memory so it survives across files/reopenings.
    search_seed: String,
    /// Lines holding a match, from the dialog's "Find all" (the same button the
    /// editor has). Kept until the next Find all or until the viewer closes.
    found_lines: std::collections::HashSet<usize>,
    /// Byte offset of the last match (for "find next").
    last_match: Option<usize>,
    /// Viewport size, updated by the renderer each frame.
    view_rows: usize,
    view_cols: usize,
    /// Content body and footer (F-key bar) rects, recorded by the renderer for
    /// mouse hit-testing.
    content_area: Rect,
    footer_area: Rect,
    /// Cached document outline (headings), built lazily on the first F6 press.
    outline: Option<Vec<markdown::OutlineItem>>,
    /// Whether the F6 outline navigator overlay is currently shown.
    outline_open: bool,
    /// Selected entry in the outline list.
    outline_sel: usize,
    /// First visible entry (scroll offset) of the outline list; kept in sync by
    /// the renderer so the selection stays on screen.
    outline_top: usize,
    /// Interior rect of the outline list, recorded by the renderer for mouse hits.
    outline_area: Rect,
    /// A decoded image, when this file could be shown as one (F3 on an image).
    image: Option<ViewerImage>,
    /// Whether the image (vs. the raw text/hex) is currently displayed — toggled
    /// with F8. Only meaningful when `image` is set.
    show_image: bool,
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
        let is_markdown = is_markdown_name(&name);
        ViewerState {
            name,
            src: Source::Mem(data),
            truncated,
            temp: None,
            hl: None,
            line_starts,
            scanned,
            mode: ViewMode::Text,
            is_markdown,
            markdown_render: is_markdown,
            wrap: false,
            top: 0,
            h_offset: 0,
            search: ViewSearch::default(),
            search_seed: String::new(),
            found_lines: std::collections::HashSet::new(),
            last_match: None,
            view_rows: 1,
            view_cols: 1,
            content_area: Rect::default(),
            footer_area: Rect::default(),
            outline: None,
            outline_open: false,
            outline_sel: 0,
            outline_top: 0,
            outline_area: Rect::default(),
            image: None,
            show_image: false,
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
        let is_markdown = is_markdown_name(&name);
        ViewerState {
            name,
            src: Source::File { file: RefCell::new(file), len },
            truncated: false,
            temp,
            hl: None,
            line_starts,
            scanned,
            mode: ViewMode::Text,
            is_markdown,
            markdown_render: is_markdown,
            wrap: false,
            top: 0,
            h_offset: 0,
            search: ViewSearch::default(),
            search_seed: String::new(),
            found_lines: std::collections::HashSet::new(),
            last_match: None,
            view_rows: 1,
            view_cols: 1,
            content_area: Rect::default(),
            footer_area: Rect::default(),
            outline: None,
            outline_open: false,
            outline_sel: 0,
            outline_top: 0,
            outline_area: Rect::default(),
            image: None,
            show_image: false,
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

    /// Seed the F7 prompt's pre-filled term from the app-wide search memory.
    pub fn set_search_seed(&mut self, seed: String) {
        self.search_seed = seed;
    }

    /// Whether the viewer is showing the hex dump (F4), so the shared search
    /// dialog opens in Hex mode — matching what the editor does.
    pub fn is_hex(&self) -> bool {
        self.mode == ViewMode::Hex
    }

    /// The last search term (for writing back to the app-wide search memory).
    pub fn search_seed(&self) -> &str {
        &self.search_seed
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ViewerSignal {
        // While the outline navigator is open it captures navigation keys.
        if self.outline_open {
            return self.handle_outline_key(key);
        }

        match key.code {
            // F3 toggles the viewer (open in the panels, close here), matching
            // the footer's "Quit" label; F10 / Esc / q also close.
            KeyCode::F(3) | KeyCode::F(10) | KeyCode::Esc | KeyCode::Char('q') => {
                return ViewerSignal::Close
            }
            KeyCode::F(2) => self.wrap = !self.wrap,
            KeyCode::F(4) => {
                self.mode = match self.mode {
                    ViewMode::Text => ViewMode::Hex,
                    ViewMode::Hex => ViewMode::Text,
                };
                self.top = self.top.min(self.max_top());
            }
            KeyCode::F(1) => return ViewerSignal::OpenHelp,
            KeyCode::F(5) => return ViewerSignal::OpenGoto,
            // F6 (Markdown files in text mode): open the document outline.
            KeyCode::F(6) if self.is_markdown && self.mode == ViewMode::Text => self.open_outline(),
            // F8 (image files): toggle between the image and the raw text/hex.
            KeyCode::F(8) if self.image.is_some() => self.show_image = !self.show_image,
            // F8 (Markdown files only): toggle the Markdown render and the raw
            // (syntax-highlighted) text.
            KeyCode::F(8) if self.is_markdown && self.mode == ViewMode::Text => {
                self.markdown_render = !self.markdown_render;
            }
            KeyCode::F(7) => return ViewerSignal::OpenSearch,
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

        // The outline navigator, while open, captures the mouse (wheel scrolls it,
        // a click on an entry jumps there, a click outside dismisses it).
        if self.outline_open {
            return self.handle_outline_mouse(ev);
        }

        // F-key bar clicks.
        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left)) && row == self.footer_area.y {
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
    /// Whether the Markdown approximation should be drawn (a Markdown file in
    /// text mode with the render toggle on).
    pub(crate) fn markdown_active(&self) -> bool {
        self.is_markdown && self.markdown_render && self.mode == ViewMode::Text
    }

    /// Attach a decoded image and switch to showing it (F3 on an image file).
    pub fn set_image(&mut self, iv: ViewerImage) {
        self.image = Some(iv);
        self.show_image = true;
    }

    /// The decoded image, when it is currently being displayed (vs. the raw view).
    pub(crate) fn active_image(&self) -> Option<&ViewerImage> {
        self.show_image.then_some(self.image.as_ref()).flatten()
    }

    pub(crate) fn footer_labels(&self) -> [&'static str; 10] {
        let wrap = if self.wrap { "Unwrap" } else { "Wrap" };
        let mode = if self.mode == ViewMode::Hex { "Text" } else { "Hex" };
        // F8: for an image file, toggle Image/Raw; for a Markdown file in text
        // mode, "Raw" shows the source and "Render" the approximation.
        let f8 = if self.image.is_some() {
            if self.show_image { "Raw" } else { "Image" }
        } else if self.is_markdown && self.mode == ViewMode::Text {
            if self.markdown_render { "Raw" } else { "Render" }
        } else {
            ""
        };
        // F6: the document outline, offered for Markdown files in text mode.
        let outline = if self.is_markdown && self.mode == ViewMode::Text { "Outline" } else { "" };
        ["Help", wrap, "Quit", mode, "Goto", outline, "Search", f8, "Next", "Quit"]
    }

    /// Perform the action of F-key index `i` (0-based) from a bar click.
    fn activate_fkey(&mut self, i: usize) -> ViewerSignal {
        let code = match i {
            1 => KeyCode::F(2),               // Wrap / Unwrap
            2 | 9 => return ViewerSignal::Close, // Quit
            3 => KeyCode::F(4),               // Text / Hex
            4 => return ViewerSignal::OpenGoto, // Goto
            5 => KeyCode::F(6),               // Outline (Markdown)
            6 => KeyCode::F(7),               // Search
            7 => KeyCode::F(8),               // Raw / Render (Markdown)
            8 => KeyCode::Char('n'),          // Next match
            0 => return ViewerSignal::OpenHelp, // Help
            _ => return ViewerSignal::Stay,   // empty slot: no-op
        };
        self.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Whether the F6 document-outline overlay is currently shown.
    pub fn is_outline_open(&self) -> bool {
        self.outline_open
    }

    /// Open the document-outline navigator, building the heading list on first
    /// use and pre-selecting the heading at or before the current position.
    pub fn open_outline(&mut self) {
        if self.outline.is_none() {
            let items = self.build_outline();
            self.outline = Some(items);
        }
        self.outline_sel = self
            .outline
            .as_ref()
            .map_or(0, |o| o.iter().rposition(|it| it.line <= self.top).unwrap_or(0));
        self.outline_open = true;
    }

    /// Scan the whole document for ATX headings, producing the outline entries.
    /// Headings inside fenced code blocks are ignored.
    fn build_outline(&mut self) -> Vec<markdown::OutlineItem> {
        self.index_fully();
        let mut items = Vec::new();
        let mut in_fence = false;
        for li in 0..self.line_count() {
            let line = self.line_str(li);
            if markdown::is_fence(&line) {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            if let Some((level, text)) = markdown::heading_of(&line) {
                items.push(markdown::OutlineItem { level, text, line: li });
            }
        }
        items
    }

    /// Keys while the outline overlay is open: navigate the list, jump on Enter,
    /// dismiss on Esc/F6.
    fn handle_outline_key(&mut self, key: KeyEvent) -> ViewerSignal {
        let len = self.outline.as_ref().map_or(0, |o| o.len());
        // A page is the visible height of the list (set by the renderer).
        let page = (self.outline_area.height as isize).max(1);
        match key.code {
            KeyCode::Esc | KeyCode::F(6) => self.outline_open = false,
            KeyCode::Enter => {
                self.jump_to_outline_sel();
                self.outline_open = false;
            }
            KeyCode::Up => self.outline_move(-1),
            KeyCode::Down => self.outline_move(1),
            KeyCode::PageUp => self.outline_move(-page),
            KeyCode::PageDown => self.outline_move(page),
            KeyCode::Home => self.outline_sel = 0,
            KeyCode::End => self.outline_sel = len.saturating_sub(1),
            _ => {}
        }
        ViewerSignal::Stay
    }

    /// Mouse while the outline overlay is open: wheel scrolls the selection, a
    /// left click on an entry jumps there, a click elsewhere dismisses it.
    fn handle_outline_mouse(&mut self, ev: MouseEvent) -> ViewerSignal {
        match ev.kind {
            MouseEventKind::ScrollDown => self.outline_move(1),
            MouseEventKind::ScrollUp => self.outline_move(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                let a = self.outline_area;
                let inside = a.height > 0
                    && ev.row >= a.y
                    && ev.row < a.y + a.height
                    && ev.column >= a.x
                    && ev.column < a.x + a.width;
                if inside {
                    let idx = self.outline_top + (ev.row - a.y) as usize;
                    if idx < self.outline.as_ref().map_or(0, |o| o.len()) {
                        self.outline_sel = idx;
                        self.jump_to_outline_sel();
                    }
                }
                self.outline_open = false;
            }
            _ => {}
        }
        ViewerSignal::Stay
    }

    /// Move the outline selection by `delta`, clamped to the list bounds.
    fn outline_move(&mut self, delta: isize) {
        let len = self.outline.as_ref().map_or(0, |o| o.len());
        if len == 0 {
            return;
        }
        self.outline_sel =
            (self.outline_sel as isize).saturating_add(delta).clamp(0, len as isize - 1) as usize;
    }

    /// Scroll the view to the currently selected heading's source line.
    fn jump_to_outline_sel(&mut self) {
        let Some(line) =
            self.outline.as_ref().and_then(|o| o.get(self.outline_sel)).map(|it| it.line)
        else {
            return;
        };
        self.extend_to_line(line);
        self.top = line.min(self.max_top());
        self.h_offset = 0;
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
    /// Apply the search dialog's result. An identical repeat continues from the
    /// last hit — which is what makes pressing F7-Enter again walk the file —
    /// while any change of term or option restarts from the top.
    pub fn apply_search(&mut self, p: &crate::ui::dialog::SearchReplaceParams) {
        let want = ViewSearch {
            query: p.search.clone(),
            regex: p.regex,
            case_sensitive: p.case_sensitive,
            whole_words: p.whole_words,
            backwards: p.backwards,
            hex: p.hex,
        };
        if want != self.search {
            self.search = want;
            self.last_match = None;
        }
        self.search_seed = p.search.clone();
        if p.find_all { self.find_all() } else { self.find_next() }
    }

    /// Compile the current search, or `None` when it is unusable (bad regex, bad
    /// hex, empty term).
    fn needle(&self) -> Option<search::Needle> {
        let s = &self.search;
        search::Needle::build(&s.query, s.regex, s.case_sensitive, s.whole_words, s.hex)
    }

    /// Whether `line` holds a "Find all" match (the renderer tints it).
    pub(crate) fn line_found(&self, line: usize) -> bool {
        self.found_lines.contains(&line)
    }

    /// Number of lines the last "Find all" marked.
    pub fn found_count(&self) -> usize {
        self.found_lines.len()
    }

    fn find_next(&mut self) {
        let Some(needle) = self.needle() else { return };
        let found = if self.search.backwards {
            // Wrap to the end when there is nothing before the current hit.
            let before = self.last_match.unwrap_or(0);
            self.scan_back(&needle, before).or_else(|| self.scan_back(&needle, self.data_len()))
        } else {
            let start = self.last_match.map(|m| m + 1).unwrap_or(0);
            self.scan(&needle, start).or_else(|| self.scan(&needle, 0))
        };
        if let Some(off) = found {
            self.last_match = Some(off);
            self.reveal(off);
        }
    }

    /// Scroll so the byte at `off` is on screen.
    fn reveal(&mut self, off: usize) {
        match self.mode {
            ViewMode::Text => {
                // The match may lie beyond the indexed region; index up to it.
                self.extend_to_byte(off + 1);
                self.top = self.byte_to_line(off);
            }
            ViewMode::Hex => self.top = off / 16,
        }
    }

    /// Highlight every line holding a match, replacing any previous set, and jump
    /// to the first. Capped at [`FOUND_LINES_MAX`]: the viewer pages files far too
    /// big to mark exhaustively, and a bounded set keeps this from turning into an
    /// unbounded scan of a multi-gigabyte log.
    fn find_all(&mut self) {
        self.found_lines.clear();
        let Some(needle) = self.needle() else { return };
        let mut at = 0usize;
        let mut first = None;
        while let Some(off) = self.scan(&needle, at) {
            if first.is_none() {
                first = Some(off);
            }
            self.extend_to_byte(off + 1);
            self.found_lines.insert(self.byte_to_line(off));
            if self.found_lines.len() >= FOUND_LINES_MAX {
                break;
            }
            at = off + 1;
        }
        if let Some(off) = first {
            self.last_match = Some(off);
            self.reveal(off);
        }
    }

    /// First match at or after `start`, reading the source in overlapping windows
    /// so a file-backed source is never loaded whole.
    fn scan(&self, needle: &search::Needle, start: usize) -> Option<usize> {
        const WINDOW: usize = 256 * 1024;
        let len = self.data_len();
        let overlap = needle.overlap();
        let mut pos = start.min(len);
        while pos + needle.min_len() <= len {
            let end = (pos + WINDOW.max(overlap + 1)).min(len);
            let buf = self.src.read_range(pos, end);
            if let Some(i) = needle.find(&buf, 0) {
                return Some(pos + i);
            }
            if end == len {
                break;
            }
            // Rewind so a match straddling the seam is still seen.
            pos = end - overlap;
        }
        None
    }

    /// Last match starting strictly before `before`. Scans forward keeping the
    /// most recent hit: a backwards search is the rarer path, and this reuses the
    /// same windowing rather than needing a second, reversed reader.
    fn scan_back(&self, needle: &search::Needle, before: usize) -> Option<usize> {
        let mut best = None;
        let mut at = 0usize;
        while let Some(off) = self.scan(needle, at) {
            if off >= before {
                break;
            }
            best = Some(off);
            at = off + 1;
        }
        best
    }

    fn byte_to_line(&self, off: usize) -> usize {
        match self.line_starts.binary_search(&off) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// Whether logical line `line` begins *inside* a fenced code block — i.e. an
    /// odd number of code-fence lines (` ``` ` / `~~~`) precede it. Lets the
    /// Markdown renderer tell whether the top of the viewport is already within a
    /// code box when the block's opening fence has scrolled off the top. Scans
    /// only the already-indexed prefix `0..line`, so it triggers no extra I/O.
    fn in_code_fence_at(&self, line: usize) -> bool {
        let mut inside = false;
        for li in 0..line.min(self.line_count()) {
            if markdown::is_fence(&self.line_str(li)) {
                inside = !inside;
            }
        }
        inside
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

/// Whether `name` looks like a Markdown file (by extension).
fn is_markdown_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".md", ".markdown", ".mdown", ".mkd", ".mdx"].iter().any(|ext| lower.ends_with(ext))
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
    fn markdown_mode_defaults_on_and_f8_toggles_raw() {
        // A .md file opens in the Markdown approximation; F8 toggles raw text.
        let mut v = ViewerState::new("README.md".into(), b"# Title\n".to_vec());
        assert!(v.markdown_active(), "markdown files render markdown by default");
        assert_eq!(v.footer_labels()[7], "Raw", "F8 shows 'Raw' while rendering markdown");

        v.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE));
        assert!(!v.markdown_active(), "F8 switches to raw text");
        assert_eq!(v.footer_labels()[7], "Render", "F8 shows 'Render' while raw");

        v.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE));
        assert!(v.markdown_active(), "F8 toggles back to markdown");

        // Switching to hex (F4) hides the markdown toggle.
        v.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE));
        assert_eq!(v.mode, ViewMode::Hex);
        assert_eq!(v.footer_labels()[7], "");

        // A non-markdown file never offers the markdown mode.
        let v = ViewerState::new("notes.txt".into(), b"# not a heading\n".to_vec());
        assert!(!v.markdown_active());
        assert_eq!(v.footer_labels()[7], "");
    }

    #[test]
    fn outline_extracts_headings_and_skips_fenced_code() {
        let md = concat!(
            "# Title\n",           // 0
            "intro\n",             // 1
            "## Section A\n",      // 2
            "```\n",               // 3  code fence opens
            "# not a heading\n",   // 4  inside the fence — ignored
            "```\n",               // 5  fence closes
            "## Section B\n",      // 6
            "### Sub `B1`\n",      // 7  inline code stripped
            "text\n",              // 8
            "#### Deep\n",         // 9
        );
        let mut v = ViewerState::new("doc.md".into(), md.as_bytes().to_vec());
        let items = v.build_outline();
        let got: Vec<(usize, &str, usize)> =
            items.iter().map(|it| (it.level, it.text.as_str(), it.line)).collect();
        assert_eq!(
            got,
            vec![
                (1, "Title", 0),
                (2, "Section A", 2),
                (2, "Section B", 6),
                (3, "Sub B1", 7),
                (4, "Deep", 9),
            ]
        );
    }

    #[test]
    fn f6_opens_outline_navigates_and_jumps() {
        let md = concat!(
            "# One\n",   // 0
            "a\n",       // 1
            "## Two\n",  // 2
            "b\n",       // 3
            "## Three\n",// 4
            "c\n",       // 5
        );
        let mut v = ViewerState::new("d.md".into(), md.as_bytes().to_vec());
        let press = |v: &mut ViewerState, c: KeyCode| {
            v.handle_key(KeyEvent::new(c, KeyModifiers::NONE));
        };

        press(&mut v, KeyCode::F(6));
        assert!(v.is_outline_open());
        assert_eq!(v.outline.as_ref().unwrap().len(), 3);
        assert_eq!(v.outline_sel, 0, "starts on the heading at/before the top line");

        // Navigate to "Three" and jump: the view scrolls to its source line.
        press(&mut v, KeyCode::Down);
        press(&mut v, KeyCode::Down);
        assert_eq!(v.outline_sel, 2);
        press(&mut v, KeyCode::Enter);
        assert!(!v.is_outline_open(), "Enter closes the outline");
        assert_eq!(v.top, 4, "jumped to the 'Three' heading line");

        // Reopening reflects the new position; Esc dismisses without moving.
        press(&mut v, KeyCode::F(6));
        assert_eq!(v.outline_sel, 2);
        press(&mut v, KeyCode::Esc);
        assert!(!v.is_outline_open());
        assert_eq!(v.top, 4);
    }

    #[test]
    fn f6_outline_only_for_markdown_text_mode() {
        // A non-markdown file offers no outline and F6 is inert.
        let mut v = ViewerState::new("notes.txt".into(), b"# not markdown\n".to_vec());
        assert_eq!(v.footer_labels()[5], "");
        v.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        assert!(!v.is_outline_open());

        // Markdown in text mode: the label shows and F6 opens the outline.
        let mut v = ViewerState::new("r.md".into(), b"# H\n".to_vec());
        assert_eq!(v.footer_labels()[5], "Outline");
        v.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        assert!(v.is_outline_open());
        v.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        // Switching to hex mode hides the outline affordance.
        v.handle_key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE));
        assert_eq!(v.mode, ViewMode::Hex);
        assert_eq!(v.footer_labels()[5], "");
        v.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        assert!(!v.is_outline_open(), "F6 does nothing in hex mode");
    }

    #[test]
    fn outline_overlay_renders_headings_and_highlights_selection() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let md = concat!("# One\n", "a\n", "## Two\n", "b\n", "### Three\n");
        let mut v = ViewerState::new("d.md".into(), md.as_bytes().to_vec());
        v.open_outline(); // selection starts on "One" (index 0)
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(50, 16)).unwrap();
        t.draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme, None)).unwrap();
        let b = t.backend().buffer();

        // Every heading title is drawn somewhere in the overlay.
        let rows: Vec<String> = (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect())
            .collect();
        for title in ["One", "Two", "Three"] {
            assert!(rows.iter().any(|r| r.contains(title)), "outline shows '{title}'");
        }

        // The selected entry ("One") is drawn with the dialog selection background.
        // (The document's own "One" heading is also visible outside the centered
        // overlay box, so match the row that both shows "One" and is highlighted.)
        let sel_bg = theme.dialog_selection.bg.unwrap();
        let highlighted = rows.iter().enumerate().any(|(y, r)| {
            r.contains("One") && (0..b.area.width).any(|x| b[(x, y as u16)].bg == sel_bg)
        });
        assert!(highlighted, "the selected heading row is highlighted");
    }

    #[test]
    fn markdown_fenced_code_is_boxed_and_literal() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let md = concat!(
            "# Title\n",           // 0
            "```rust\n",           // 1  fence opens (language: rust)
            "fn main() {}\n",      // 2  code content, shown literally
            "# not a heading\n",   // 3  '#' inside the fence stays literal
            "```\n",               // 4  fence closes
            "done\n",              // 5
        );
        let mut v = ViewerState::new("doc.md".into(), md.as_bytes().to_vec());
        assert!(v.markdown_active());
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(40, 12)).unwrap();
        t.draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme, None)).unwrap();
        let b = t.backend().buffer();
        let rows: Vec<String> = (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect())
            .collect();
        let all = rows.join("\n");

        // The block is framed with box-drawing corners and side borders.
        assert!(all.contains('┌') && all.contains('┐'), "top border drawn");
        assert!(all.contains('└') && all.contains('┘'), "bottom border drawn");
        assert!(all.contains('│'), "side borders drawn");
        // The language labels the opening border.
        assert!(rows.iter().any(|r| r.contains("rust")), "language shown on the box");
        // Code content is literal: the '#' line is NOT turned into a heading
        // (which would strip the marker), it is kept verbatim inside the box.
        assert!(rows.iter().any(|r| r.contains("# not a heading")), "'#' kept literally in code");
        assert!(rows.iter().any(|r| r.contains("fn main() {}")), "code body rendered");
    }

    #[test]
    fn markdown_box_still_frames_when_scrolled_into_the_block() {
        // Starting the viewport in the middle of a code block (the opening fence
        // scrolled off the top) still draws the side borders around the content.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let md = concat!(
            "```\n",    // 0  fence opens
            "aaaa\n",   // 1
            "bbbb\n",   // 2
            "cccc\n",   // 3
            "```\n",    // 4  fence closes
        );
        let mut v = ViewerState::new("d.md".into(), md.as_bytes().to_vec());
        v.top = 2; // start on the "bbbb" content line, inside the fence
        assert!(v.in_code_fence_at(v.top), "top of the viewport is inside a fence");
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(30, 8)).unwrap();
        t.draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme, None)).unwrap();
        let b = t.backend().buffer();
        let rows: Vec<String> = (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect())
            .collect();
        // No opening corner is visible (it is above the viewport) but the content
        // is still framed by side borders and closed at the bottom.
        assert!(rows.iter().any(|r| r.contains("bbbb") && r.contains('│')), "content framed");
        assert!(rows.join("\n").contains('└'), "bottom border still drawn");
    }

    #[test]
    fn outline_headings_stay_legible_on_a_bright_dialog() {
        // On a theme with a bright dialog background, the per-level heading colors
        // are contrast-adjusted so every drawn entry remains readable.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let md = concat!("# One\n", "a\n", "## Two\n", "b\n", "### Three\n");
        let mut v = ViewerState::new("d.md".into(), md.as_bytes().to_vec());
        v.open_outline();
        let theme = crate::ui::theme::Theme::by_name("GitHub Light", true);
        let mut t = Terminal::new(TestBackend::new(50, 16)).unwrap();
        t.draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme, None)).unwrap();
        let b = t.backend().buffer();

        // Every non-selected outline entry cell (drawn on the dialog background)
        // must contrast with that background — no illegible headings.
        let dialog_bg = theme.dialog_bg;
        let luma = |c: ratatui::style::Color| match c {
            ratatui::style::Color::Rgb(r, g, b) => {
                0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64
            }
            _ => 128.0,
        };
        let bg_luma = luma(dialog_bg);
        for y in 0..b.area.height {
            for x in 0..b.area.width {
                let cell = &b[(x, y)];
                if cell.bg == dialog_bg && cell.symbol().trim() != "" {
                    assert!(
                        (luma(cell.fg) - bg_luma).abs() >= 96.0,
                        "outline text {:?} must contrast with the dialog bg",
                        cell.symbol()
                    );
                }
            }
        }
    }

    #[test]
    fn outline_click_selects_and_jumps() {
        let md = concat!("# One\n", "a\n", "## Two\n", "b\n", "## Three\n");
        let mut v = ViewerState::new("d.md".into(), md.as_bytes().to_vec());
        v.open_outline();
        // Stand in for the renderer's recorded list rect (rows 2,3,4).
        v.outline_area = Rect::new(0, 2, 20, 3);
        v.outline_top = 0;
        // Click the third row → entry index 2 ("Three" on source line 4).
        v.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 4));
        assert!(!v.is_outline_open(), "a click jumps and closes the outline");
        assert_eq!(v.top, 4);
    }

    /// The dialog's params for a plain (case-insensitive, literal) search.
    fn sp(term: &str) -> crate::ui::dialog::SearchReplaceParams {
        crate::ui::dialog::SearchReplaceParams {
            replace: false,
            search: term.into(),
            replacement: String::new(),
            regex: false,
            case_sensitive: false,
            whole_words: false,
            backwards: false,
            hex: false,
            find_all: false,
        }
    }

    #[test]
    fn f1_asks_the_app_to_open_the_manual() {
        // The viewer's F-key bar advertises "Help" at F1; it must actually do
        // something (open the manual), not sit there as a dead label.
        let mut v = ViewerState::new("t".into(), b"hello".to_vec());
        let sig = v.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert!(matches!(sig, ViewerSignal::OpenHelp));
    }

    #[test]
    fn f7_asks_the_app_for_the_shared_search_dialog() {
        // The viewer no longer owns an inline prompt: F7 hands off to the same
        // modal dialog the editor uses, so both offer the same options.
        let mut v = ViewerState::new("t".into(), b"alpha beta".to_vec());
        let sig = v.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        assert!(matches!(sig, ViewerSignal::OpenSearch));
        // In hex mode the app opens that dialog in Hex mode (see `search_dialog`).
        assert!(!v.is_hex());
        v.mode = ViewMode::Hex;
        assert!(v.is_hex());
    }

    #[test]
    fn repeating_a_search_advances_to_the_next_occurrence() {
        // The bug this guards: re-submitting the same term used to reset the
        // cursor and keep re-finding the first hit.
        let mut v = ViewerState::new("t".into(), b"two\nx\ntwo\ny\ntwo".to_vec());
        v.apply_search(&sp("two"));
        assert_eq!(v.top, 0, "first hit");
        v.apply_search(&sp("two"));
        assert_eq!(v.top, 2, "the same term again moves on");
        v.apply_search(&sp("two"));
        assert_eq!(v.top, 4);
        // Past the last hit it wraps back to the top.
        v.apply_search(&sp("two"));
        assert_eq!(v.top, 0, "wraps around");

        // The `n` key repeats the same search without reopening the dialog.
        v.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(v.top, 2, "'n' advances too");

        // Re-running the same term keeps advancing (we are on line 2, so on to 4).
        v.apply_search(&sp("two"));
        assert_eq!(v.top, 4);
        // A *different* term restarts from the top rather than resuming.
        v.apply_search(&sp("y"));
        assert_eq!(v.top, 3, "a new term searches from the start");
    }

    #[test]
    fn search_honours_the_dialog_options() {
        let mut v = ViewerState::new("t".into(), b"Hit\nhit\nhitting".to_vec());
        // Case-sensitive skips the capitalised line.
        let mut p = sp("hit");
        p.case_sensitive = true;
        v.apply_search(&p);
        assert_eq!(v.top, 1);
        // Whole words skips "hitting" — the next hit wraps back to line 1.
        let mut p = sp("hit");
        p.case_sensitive = true;
        p.whole_words = true;
        v.apply_search(&p);
        assert_eq!(v.top, 1);
        v.apply_search(&p);
        assert_eq!(v.top, 1, "'hitting' is not a whole word, so it wraps to the only hit");

        // Regex.
        let mut v = ViewerState::new("t".into(), b"aaa\nbbb\nabc".to_vec());
        let mut p = sp("a.c");
        p.regex = true;
        v.apply_search(&p);
        assert_eq!(v.top, 2);

        // `^`/`$` anchor per line, exactly as in the editor: the "foo" inside
        // "xfoo" is skipped for the one that starts a line.
        let mut v = ViewerState::new("t".into(), b"xfoo\nbar\nfoo end".to_vec());
        let mut p = sp("^foo");
        p.regex = true;
        v.apply_search(&p);
        assert_eq!(v.top, 2, "the line-initial foo is found, not the one in 'xfoo'");

        // Backwards walks up the file.
        let mut v = ViewerState::new("t".into(), b"hit\nx\nhit\ny\nhit".to_vec());
        v.apply_search(&sp("hit"));
        assert_eq!(v.top, 0);
        let mut p = sp("hit");
        p.backwards = true;
        v.apply_search(&p);
        assert_eq!(v.top, 4, "backwards from the first hit wraps to the last");
        v.apply_search(&p);
        assert_eq!(v.top, 2);
    }

    #[test]
    fn hex_mode_search_matches_raw_bytes() {
        let mut v = ViewerState::new("t".into(), b"one\nHello".to_vec());
        let mut p = sp("48 65"); // "He"
        p.hex = true;
        v.apply_search(&p);
        assert_eq!(v.top, 1, "the hex bytes are found on the second line");
    }

    #[test]
    fn find_all_highlights_matching_lines_until_the_next_one() {
        let mut v = ViewerState::new("t".into(), b"hit\nmiss\nHIT\nmiss\nhit".to_vec());
        let mut p = sp("hit");
        p.find_all = true;
        v.apply_search(&p);
        assert_eq!(v.found_count(), 3);
        for l in [0, 2, 4] {
            assert!(v.line_found(l), "line {l} is highlighted");
        }
        assert!(!v.line_found(1) && !v.line_found(3));

        // A plain search leaves the highlight alone...
        v.apply_search(&sp("miss"));
        assert!(v.line_found(0), "an ordinary search keeps the highlight");
        // ...and the next Find all replaces it.
        let mut p = sp("miss");
        p.find_all = true;
        v.apply_search(&p);
        assert_eq!(v.found_count(), 2);
        assert!(v.line_found(1) && !v.line_found(0));
    }


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
        v.apply_search(&sp("two"));
        // Case-insensitive: first match is on line 1 ("two").
        assert_eq!(v.top, 1);
        // Repeating moves on to the uppercase TWO on line 3.
        v.apply_search(&sp("two"));
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

    #[test]
    fn f3_closes_the_viewer() {
        // F3 opens the viewer from the panels, so it must also close it (the
        // footer labels F3 as "Quit"); F10 and Esc remain close keys too.
        let mut v = ViewerState::new("t".into(), many_lines(10));
        with_layout(&mut v);
        let press = |code: KeyCode| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(
            matches!(v.handle_key(press(KeyCode::F(3))), ViewerSignal::Close),
            "F3 closes the viewer"
        );
        let mut v = ViewerState::new("t".into(), many_lines(10));
        with_layout(&mut v);
        assert!(
            matches!(v.handle_key(press(KeyCode::F(10))), ViewerSignal::Close),
            "F10 still closes the viewer"
        );
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
        v.apply_search(&sp("needle"));
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
        t.draw(|f| crate::viewer::render::render(f, f.area(), &mut v, &theme, None)).unwrap();
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
        t.draw(|f| render::render(f, f.area(), &mut v, &theme, None)).unwrap();
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

    #[test]
    fn image_mode_renders_and_f8_toggles_raw() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut v = ViewerState::new("photo.png".into(), b"\x89PNG not-really".to_vec());
        let img = image::RgbaImage::from_pixel(8, 6, image::Rgba([20, 180, 90, 255]));
        let sig = crate::util::img::image_sig(&img);
        v.set_image(ViewerImage { img, sig, orig: (800, 600) });
        assert!(v.active_image().is_some(), "opens showing the image");

        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(40, 12)).unwrap();
        t.draw(|f| render::render(f, f.area(), &mut v, &theme, None)).unwrap();
        let b = t.backend().buffer();
        let all: String = (0..b.area.height)
            .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
            .map(|(x, y)| b[(x, y)].symbol().to_string())
            .collect();
        // Header names the image and its original dimensions; body has half-blocks.
        assert!(all.contains("Image") && all.contains("800×600"), "header: {all:?}");
        assert!(all.contains('▀'), "ascii image drawn (no graphics)");
        // The F-key bar offers the Image/Raw toggle on F8.
        assert_eq!(v.footer_labels()[7], "Raw");

        // F8 toggles to the raw view and back.
        let f8 = KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE);
        v.handle_key(f8);
        assert!(v.active_image().is_none(), "F8 hides the image");
        assert_eq!(v.footer_labels()[7], "Image");
        v.handle_key(f8);
        assert!(v.active_image().is_some(), "F8 shows it again");
    }
}
