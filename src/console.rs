//! The command-line **console backdrop**.
//!
//! A shell (the same one `Ctrl-O` drops into — see [`crate::shell`]) runs in a
//! pseudo-terminal for the life of the program, feeding a headless terminal
//! emulator ([`vt100`]) whose screen is drawn *behind* the panels: hide a panel
//! (`Ctrl-F1` / `Ctrl-F2`) or switch to half-height (`Ctrl-F4`) and the live
//! shell output shows through, Norton-Commander style.
//!
//! There can be **several** shells at once — the local subshell plus one per open
//! SFTP/SCP session. Each owns its own emulator; the backdrop shows whichever is
//! *current* (the shell you last ran a command in / dropped into), swapped with
//! [`Console::set_current`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vt100::Parser;

/// A shell's emulator plus a "has produced output" flag (so the backdrop stays
/// blank until the shell is first used).
#[derive(Clone)]
pub struct ConsoleFeed {
    pub parser: Arc<Mutex<Parser>>,
    pub used: Arc<AtomicBool>,
}

impl ConsoleFeed {
    /// A fresh blank emulator of `rows` x `cols`.
    pub fn new(rows: u16, cols: u16) -> Self {
        ConsoleFeed {
            parser: Arc::new(Mutex::new(Parser::new(rows.max(1), cols.max(1), 0))),
            used: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// The console backdrop: a handle to whichever shell's emulator is currently
/// shown behind the panels.
pub struct Console {
    current: Mutex<ConsoleFeed>,
}

impl Console {
    /// A blank console sized to `rows` x `cols` (kept in step with the terminal
    /// by [`Console::resize`]).
    pub fn new(rows: u16, cols: u16) -> Self {
        Console { current: Mutex::new(ConsoleFeed::new(rows, cols)) }
    }

    /// Make `feed` the shell whose screen the backdrop shows (called when the
    /// active shell changes — local ↔ a remote session).
    pub fn set_current(&self, feed: ConsoleFeed) {
        if let Ok(mut cur) = self.current.lock() {
            *cur = feed;
        }
    }

    /// The current shell's emulator, for rendering the backdrop.
    pub fn parser(&self) -> Arc<Mutex<Parser>> {
        self.current
            .lock()
            .map(|c| c.parser.clone())
            .unwrap_or_else(|_| Arc::new(Mutex::new(Parser::new(1, 1, 0))))
    }

    /// Whether the current shell has produced any output (rendering is skipped
    /// until so).
    pub fn is_used(&self) -> bool {
        self.current.lock().map(|c| c.used.load(Ordering::Relaxed)).unwrap_or(false)
    }

    /// Resize the current emulated screen (no-op when already that size). Kept
    /// equal to the subshell PTY's size.
    pub fn resize(&self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if let Ok(cur) = self.current.lock()
            && let Ok(mut p) = cur.parser.lock()
            && p.screen().size() != (rows, cols)
        {
            p.screen_mut().set_size(rows, cols);
        }
    }

    /// Feed bytes straight into the current emulator (used by tests and to seed
    /// the screen; the live console is driven by each shell's reader).
    pub fn feed(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Ok(cur) = self.current.lock() {
            if let Ok(mut p) = cur.parser.lock() {
                p.process(bytes);
            }
            cur.used.store(true, Ordering::Relaxed);
        }
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
        let parser = c.parser();
        let p = parser.lock().unwrap();
        let screen = p.screen();
        let row0: String =
            (0..11).filter_map(|col| screen.cell(0, col)).map(|cell| cell.contents()).collect();
        assert_eq!(row0, "hello world");
    }

    #[test]
    fn resize_changes_the_screen_size() {
        let c = Console::new(24, 80);
        c.resize(10, 40);
        assert_eq!(c.parser().lock().unwrap().screen().size(), (10, 40));
        // Zero dimensions are clamped to at least one row/column.
        c.resize(0, 0);
        assert_eq!(c.parser().lock().unwrap().screen().size(), (1, 1));
    }

    #[test]
    fn set_current_swaps_the_shown_shell() {
        let c = Console::new(24, 80);
        c.feed(b"local-shell\r\n");
        // Switch to a second (remote) shell's blank emulator.
        let remote = ConsoleFeed::new(24, 80);
        remote.parser.lock().unwrap().process(b"remote-shell\r\n");
        remote.used.store(true, Ordering::Relaxed);
        c.set_current(remote.clone());
        let parser = c.parser();
        let p = parser.lock().unwrap();
        let row0: String =
            (0..12).filter_map(|col| p.screen().cell(0, col)).map(|cell| cell.contents()).collect();
        assert_eq!(row0, "remote-shell", "backdrop now shows the current shell");
    }
}
