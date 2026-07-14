//! The command-line **console backdrop**.
//!
//! Commands run from the command line have their output captured (through a
//! pseudo-terminal, see `app::run_command`) and fed here, into a headless
//! terminal emulator. Its screen is then drawn *behind* the panels, so hiding a
//! panel (`Ctrl-F1`/`Ctrl-F2`) or switching to half-height (`Ctrl-F3`) reveals
//! the shell output underneath — the Norton-Commander console.
//!
//! The emulator ([`vt100`]) interprets the full ANSI/VT stream, so colours and
//! cursor motion render faithfully; the console keeps the most recent screenful
//! (the results of previous runs) as its scrollback.

use vt100::Parser;

/// A headless terminal emulator holding the captured command-line output.
pub struct Console {
    parser: Parser,
    /// Set once any output has been captured, so the backdrop stays blank (and
    /// rendering is skipped entirely) until the first command runs.
    used: bool,
}

impl Console {
    /// A blank console sized to `rows` x `cols`. The size is kept in step with
    /// the exposed backdrop area by [`Console::resize`].
    pub fn new(rows: u16, cols: u16) -> Self {
        Console { parser: Parser::new(rows.max(1), cols.max(1), 0), used: false }
    }

    /// Resize the emulated screen to match the backdrop area (no-op when already
    /// that size). Content reflows as a real terminal would.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if self.parser.screen().size() != (rows, cols) {
            self.parser.screen_mut().set_size(rows, cols);
        }
    }

    /// Feed captured terminal bytes (a command's raw PTY output) to the emulator.
    pub fn feed(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.used = true;
            self.parser.process(bytes);
        }
    }

    /// Echo a command onto the console before its output, so the backdrop reads
    /// like a real session (`<cwd>$ <cmd>`). A leading newline separates it from
    /// the previous command's output.
    pub fn banner(&mut self, cwd: &str, cmd: &str) {
        // Mark used unconditionally: even an output-less command should leave its
        // prompt line on the console.
        self.used = true;
        self.parser.process(format!("\r\n{cwd}$ {cmd}\r\n").as_bytes());
    }

    /// Whether anything has been captured yet (rendering is skipped until so).
    pub fn is_used(&self) -> bool {
        self.used
    }

    /// The current emulated screen, for rendering the backdrop.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_and_exposes_text() {
        let mut c = Console::new(24, 80);
        assert!(!c.is_used(), "blank console is unused");
        c.feed(b"hello world\r\n");
        assert!(c.is_used());
        // The text lands on the first row of the emulated screen.
        let screen = c.screen();
        let row0: String =
            (0..11).filter_map(|col| screen.cell(0, col)).map(|cell| cell.contents()).collect();
        assert_eq!(row0, "hello world");
    }

    #[test]
    fn banner_marks_used_even_without_output() {
        let mut c = Console::new(24, 80);
        c.banner("/tmp", "true");
        assert!(c.is_used());
        let screen = c.screen();
        let text = (0..2)
            .flat_map(|r| (0..80).filter_map(move |col| screen.cell(r, col)))
            .map(|cell| cell.contents())
            .collect::<String>();
        assert!(text.contains("/tmp$ true"), "banner echoes the command: {text:?}");
    }

    #[test]
    fn resize_changes_the_screen_size() {
        let mut c = Console::new(24, 80);
        c.resize(10, 40);
        assert_eq!(c.screen().size(), (10, 40));
        // Zero dimensions are clamped to at least one row/column.
        c.resize(0, 0);
        assert_eq!(c.screen().size(), (1, 1));
    }
}
