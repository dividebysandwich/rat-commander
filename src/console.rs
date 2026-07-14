//! The command-line **console backdrop**.
//!
//! A single persistent shell (the same one `Ctrl-O` drops into — see
//! [`crate::shell::Subshell`]) runs in a pseudo-terminal for the life of the
//! program. Its output is fed into a headless terminal emulator ([`vt100`]) held
//! here, whose screen is drawn *behind* the panels: hide a panel (`Ctrl-F1` /
//! `Ctrl-F2`) or switch to half-height (`Ctrl-F4`) and the live shell output
//! shows through the exposed area, Norton-Commander style.
//!
//! The emulator is shared with the subshell's reader thread through an
//! `Arc<Mutex<…>>`, so command output and interactive `Ctrl-O` activity land on
//! the same console — one terminal session, always in sync.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use vt100::Parser;

/// The shared console emulator plus a "has produced output yet" flag (so the
/// backdrop stays blank until the shell is first used).
pub struct Console {
    parser: Arc<Mutex<Parser>>,
    used: Arc<AtomicBool>,
}

impl Console {
    /// A blank console sized to `rows` x `cols` (kept in step with the terminal
    /// by [`Console::resize`]).
    pub fn new(rows: u16, cols: u16) -> Self {
        Console {
            parser: Arc::new(Mutex::new(Parser::new(rows.max(1), cols.max(1), 0))),
            used: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Handles for the subshell reader thread: the shared emulator it feeds, and
    /// the flag it raises once any output has arrived.
    pub fn shared(&self) -> (Arc<Mutex<Parser>>, Arc<AtomicBool>) {
        (Arc::clone(&self.parser), Arc::clone(&self.used))
    }

    /// Whether the shell has produced any output (rendering is skipped until so).
    pub fn is_used(&self) -> bool {
        self.used.load(Ordering::Relaxed)
    }

    /// Resize the emulated screen (no-op when already that size). Content reflows
    /// as a real terminal would; kept equal to the subshell PTY's size.
    pub fn resize(&self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if let Ok(mut p) = self.parser.lock()
            && p.screen().size() != (rows, cols)
        {
            p.screen_mut().set_size(rows, cols);
        }
    }

    /// Feed bytes straight into the emulator. The live console is driven by the
    /// subshell reader thread through [`Console::shared`]; this is used by tests
    /// and to seed the screen.
    pub fn feed(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Ok(mut p) = self.parser.lock() {
            p.process(bytes);
            self.used.store(true, Ordering::Relaxed);
        }
    }

    /// Lock the emulator for rendering the backdrop.
    pub fn lock(&self) -> Option<MutexGuard<'_, Parser>> {
        self.parser.lock().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_exposes_text() {
        let c = Console::new(24, 80);
        assert!(!c.is_used(), "blank console is unused");
        c.feed(b"hello world\r\n");
        assert!(c.is_used());
        let p = c.lock().unwrap();
        let screen = p.screen();
        let row0: String =
            (0..11).filter_map(|col| screen.cell(0, col)).map(|cell| cell.contents()).collect();
        assert_eq!(row0, "hello world");
    }

    #[test]
    fn resize_changes_the_screen_size() {
        let c = Console::new(24, 80);
        c.resize(10, 40);
        assert_eq!(c.lock().unwrap().screen().size(), (10, 40));
        // Zero dimensions are clamped to at least one row/column.
        c.resize(0, 0);
        assert_eq!(c.lock().unwrap().screen().size(), (1, 1));
    }

    #[test]
    fn shared_handle_feeds_the_same_screen() {
        let c = Console::new(24, 80);
        let (parser, used) = c.shared();
        // Simulate the reader thread writing through the shared handle.
        parser.lock().unwrap().process(b"from-thread\r\n");
        used.store(true, Ordering::Relaxed);
        assert!(c.is_used());
        let p = c.lock().unwrap();
        let row0: String =
            (0..11).filter_map(|col| p.screen().cell(0, col)).map(|cell| cell.contents()).collect();
        assert_eq!(row0, "from-thread");
    }
}
