//! File-backed hex editor used by the editor's hex mode (F9).
//!
//! Editing happens **in place**: only a small window of the file is read for
//! display, pending byte changes are kept in a sparse overlay, and saving seeks
//! to each changed offset and overwrites just that byte. Nothing loads the whole
//! file, so arbitrarily large files can be hex-edited. The length is fixed —
//! this is overwrite-only editing (no insert/delete), which is what allows the
//! in-place writes.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Bytes shown per row.
pub const BYTES_PER_ROW: u64 = 16;

pub struct HexEditor {
    file: File,
    pub path: PathBuf,
    pub len: u64,
    pub readonly: bool,
    /// Cursor byte offset, in `0..len` (or 0 for an empty file).
    pub cursor: u64,
    /// First byte of the top visible row (a multiple of `BYTES_PER_ROW`).
    pub top: u64,
    /// Pending byte overwrites, flushed to the file on save.
    overlay: BTreeMap<u64, u8>,
    /// Editing the ASCII column instead of the hex column.
    pub ascii_pane: bool,
    /// In the hex column, whether the low nibble is next to be typed.
    pub nibble_low: bool,
    pub dirty: bool,
    /// Whether a save has actually altered the file (⇒ text view must reload).
    pub saved_any: bool,
    pub view_rows: usize,
}

impl HexEditor {
    /// Open `path` for in-place hex editing (read-write, falling back to
    /// read-only if write access is denied).
    pub fn open(path: &Path) -> io::Result<HexEditor> {
        let (file, readonly) = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(f) => (f, false),
            Err(_) => (OpenOptions::new().read(true).open(path)?, true),
        };
        let len = file.metadata()?.len();
        Ok(HexEditor {
            file,
            path: path.to_path_buf(),
            len,
            readonly,
            cursor: 0,
            top: 0,
            overlay: BTreeMap::new(),
            ascii_pane: false,
            nibble_low: false,
            dirty: false,
            saved_any: false,
            view_rows: 1,
        })
    }

    /// Read `count` bytes from `start`, applying any pending overlay edits. The
    /// returned vector may be shorter than `count` near end-of-file.
    pub fn window(&mut self, start: u64, count: usize) -> Vec<u8> {
        let mut buf = vec![0u8; count];
        let read = if start < self.len {
            let _ = self.file.seek(SeekFrom::Start(start));
            self.file.read(&mut buf).unwrap_or(0)
        } else {
            0
        };
        buf.truncate(read);
        for (&off, &b) in self.overlay.range(start..start + read as u64) {
            buf[(off - start) as usize] = b;
        }
        buf
    }

    /// The byte at `off`, reading from the overlay or the file.
    pub fn byte_at(&mut self, off: u64) -> Option<u8> {
        if off >= self.len {
            return None;
        }
        if let Some(&b) = self.overlay.get(&off) {
            return Some(b);
        }
        let mut b = [0u8; 1];
        let _ = self.file.seek(SeekFrom::Start(off));
        self.file.read_exact(&mut b).ok().map(|_| b[0])
    }

    fn set_byte(&mut self, off: u64, val: u8) {
        if self.readonly || off >= self.len {
            return;
        }
        self.overlay.insert(off, val);
        self.dirty = true;
    }

    /// Flush pending edits to the file, in place (one seek+write per changed
    /// byte). Cheap even for huge files since only changed bytes are written.
    pub fn save(&mut self) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "read-only file"));
        }
        for (&off, &b) in &self.overlay {
            self.file.seek(SeekFrom::Start(off))?;
            self.file.write_all(&[b])?;
        }
        self.file.flush()?;
        if !self.overlay.is_empty() {
            self.saved_any = true;
        }
        self.overlay.clear();
        self.dirty = false;
        Ok(())
    }

    // -- Navigation --------------------------------------------------------

    pub fn move_by(&mut self, delta: i64) {
        let max = self.len.saturating_sub(1) as i64;
        self.cursor = (self.cursor as i64 + delta).clamp(0, max.max(0)) as u64;
        self.nibble_low = false;
    }

    pub fn move_rows(&mut self, rows: i64) {
        self.move_by(rows * BYTES_PER_ROW as i64);
    }

    pub fn row_start(&mut self) {
        self.cursor -= self.cursor % BYTES_PER_ROW;
        self.nibble_low = false;
    }

    pub fn row_end(&mut self) {
        let end = self.cursor - self.cursor % BYTES_PER_ROW + (BYTES_PER_ROW - 1);
        self.cursor = end.min(self.len.saturating_sub(1));
        self.nibble_low = false;
    }

    pub fn goto_start(&mut self) {
        self.cursor = 0;
        self.nibble_low = false;
    }

    pub fn goto_end(&mut self) {
        self.cursor = self.len.saturating_sub(1);
        self.nibble_low = false;
    }

    pub fn toggle_pane(&mut self) {
        self.ascii_pane = !self.ascii_pane;
        self.nibble_low = false;
    }

    // -- Editing -----------------------------------------------------------

    /// Type a hex digit into the current byte's nibble. Returns false if `c` is
    /// not a hex digit (so the caller can ignore it).
    pub fn input_hex(&mut self, c: char) -> bool {
        let Some(v) = c.to_digit(16) else {
            return false;
        };
        let v = v as u8;
        if self.cursor >= self.len {
            return true;
        }
        let cur = self.byte_at(self.cursor).unwrap_or(0);
        if self.nibble_low {
            self.set_byte(self.cursor, (cur & 0xF0) | v);
            self.move_by(1); // advance to the next byte (clears nibble_low)
        } else {
            self.set_byte(self.cursor, (v << 4) | (cur & 0x0F));
            self.nibble_low = true;
        }
        true
    }

    /// Overwrite the current byte with a typed ASCII character and advance.
    pub fn input_ascii(&mut self, c: char) {
        let code = c as u32;
        if code > 0xFF || self.cursor >= self.len {
            return;
        }
        self.set_byte(self.cursor, code as u8);
        self.move_by(1);
    }

    // -- Search / replace --------------------------------------------------

    /// Find `pattern` at or after `from` (forward) or strictly before `from`
    /// (backward). Streams the file in overlapping chunks, so it works on huge
    /// files; the overlay is honored via [`window`].
    pub fn find(&mut self, pattern: &[u8], from: u64, backwards: bool) -> Option<u64> {
        let plen = pattern.len() as u64;
        if plen == 0 || plen > self.len {
            return None;
        }
        const CHUNK: usize = 64 * 1024;
        let step = CHUNK as u64;
        if backwards {
            // Scan forward over [0, from), keeping the last match found.
            let mut pos = 0u64;
            let mut last = None;
            while pos < from {
                let want = (from - pos).min(step + plen - 1) as usize;
                let buf = self.window(pos, want);
                if buf.len() < pattern.len() {
                    break;
                }
                let mut search_from = 0;
                while let Some(i) = find_sub(&buf[search_from..], pattern) {
                    let off = pos + (search_from + i) as u64;
                    if off >= from {
                        break;
                    }
                    last = Some(off);
                    search_from += i + 1;
                    if search_from >= buf.len() {
                        break;
                    }
                }
                pos += (buf.len() - (pattern.len() - 1)) as u64;
            }
            last
        } else {
            let mut pos = from.min(self.len);
            while pos < self.len {
                let want = ((self.len - pos) as usize).min(CHUNK + pattern.len() - 1);
                let buf = self.window(pos, want);
                if buf.len() < pattern.len() {
                    break;
                }
                if let Some(i) = find_sub(&buf, pattern) {
                    return Some(pos + i as u64);
                }
                pos += (buf.len() - (pattern.len() - 1)) as u64;
            }
            None
        }
    }

    /// Overwrite every non-overlapping occurrence of `search` with `replacement`
    /// (which must be the same length — editing is overwrite-only). Returns the
    /// number of replacements; the changes go to the overlay (flushed on save).
    pub fn replace_all(&mut self, search: &[u8], replacement: &[u8]) -> usize {
        if search.is_empty() || search.len() != replacement.len() || self.readonly {
            return 0;
        }
        let mut count = 0;
        let mut pos = 0u64;
        while let Some(off) = self.find(search, pos, false) {
            for (k, &b) in replacement.iter().enumerate() {
                self.set_byte(off + k as u64, b);
            }
            count += 1;
            pos = off + search.len() as u64;
        }
        count
    }
}

/// Index of the first occurrence of `needle` in `hay` (naive).
fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(bytes: &[u8]) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("rc_hex_{}_{nanos}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn edits_overlay_then_saves_in_place() {
        let p = tmp(b"hello world");
        let mut h = HexEditor::open(&p).unwrap();
        assert_eq!(h.len, 11);

        // Overwrite the first byte 'h' (0x68) with 'H' (0x48) via the hex pane.
        assert!(h.input_hex('4'));
        assert!(h.input_hex('8'));
        // Pending edit visible before save; cursor advanced to byte 1.
        assert_eq!(h.byte_at(0), Some(b'H'));
        assert_eq!(h.cursor, 1);
        // File on disk is still unchanged until save.
        assert_eq!(std::fs::read(&p).unwrap(), b"hello world");

        h.save().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"Hello world");
        assert!(!h.dirty && h.saved_any);

        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn ascii_pane_overwrite_keeps_length() {
        let p = tmp(b"abc");
        let mut h = HexEditor::open(&p).unwrap();
        h.toggle_pane();
        h.input_ascii('X'); // overwrite 'a'
        h.save().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"Xbc");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn finds_and_replaces_bytes() {
        let p = tmp(b"abc abc abc");
        let mut h = HexEditor::open(&p).unwrap();
        // Forward find from offset 1 lands on the second "abc" at offset 4.
        assert_eq!(h.find(b"abc", 1, false), Some(4));
        // Backward find before offset 6 lands on the first "abc".
        assert_eq!(h.find(b"abc", 6, true), Some(0));
        // Equal-length replace-all overwrites in place.
        assert_eq!(h.replace_all(b"abc", b"XYZ"), 3);
        h.save().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"XYZ XYZ XYZ");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn replace_rejects_length_change() {
        let p = tmp(b"abcabc");
        let mut h = HexEditor::open(&p).unwrap();
        assert_eq!(h.replace_all(b"abc", b"ab"), 0, "different length is refused");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn window_reads_with_overlay() {
        let p = tmp(b"0123456789");
        let mut h = HexEditor::open(&p).unwrap();
        h.input_hex('4'); // high nibble of byte 0 -> 0x4?
        h.input_hex('1'); // -> 0x41 = 'A'
        let w = h.window(0, 4);
        assert_eq!(&w, b"A123");
        std::fs::remove_file(&p).ok();
    }
}
