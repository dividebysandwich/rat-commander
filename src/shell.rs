//! Persistent Ctrl-O subshell, Midnight-Commander style.
//!
//! A single shell process is kept alive in a pseudo-terminal for the life of
//! the app. Ctrl-O *toggles* into it (forwarding the real terminal to the PTY)
//! and Ctrl-O again toggles back to the panels — the shell keeps running, so
//! its working directory, environment, history and jobs are preserved between
//! visits.

use crate::app::event::AppEvent;
use crate::util::async_bridge::AppSender;
use crate::util::{Error, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Environment variable set in the Ctrl-O subshell so a Rat Commander launched
/// from within it can tell it is nested (and disable its own subshell). Mirrors
/// Midnight Commander's `MC_SID` marker.
pub const SUBSHELL_ENV: &str = "RC_SUBSHELL";

/// Whether *this* process was started inside a Rat Commander Ctrl-O subshell
/// (i.e. the marker env var is present). Read once at startup.
pub fn in_subshell() -> bool {
    std::env::var_os(SUBSHELL_ENV).is_some_and(|v| !v.is_empty())
}

/// Byte sent by Ctrl-O in the legacy (raw) keyboard encoding.
const CTRL_O: u8 = 0x0F;
/// Unicode key code of the toggle key (`o`) in the kitty/xterm CSI encodings.
const CTRL_O_KEYCODE: u16 = b'o' as u16;
/// Ctrl bit in the kitty/xterm modifier encoding (parameter value minus one).
const CTRL_MOD: u16 = 4;
/// Longest unfinished escape sequence held back while waiting for the read
/// that completes it; anything longer is treated as garbage and forwarded so
/// a malformed flood can't stall input.
const MAX_HOLD: usize = 24;

/// What a scan of buffered input found.
enum Scan {
    /// Ctrl-O found: forward the bytes before `start`, swallow the toggle
    /// sequence itself, and return to the panels.
    Toggle { start: usize },
    /// No toggle. `hold` trailing bytes look like an unfinished escape
    /// sequence and should be kept back until the next read completes it.
    None { hold: usize },
}

/// Scan `buf` for a Ctrl-O keypress in any encoding a terminal may use while
/// the subshell owns the screen. The shell running inside the PTY can switch
/// the *real* terminal's keyboard encoding out from under us — fish 4.x, for
/// example, enables the kitty keyboard protocol (`CSI = 5 u`) at every prompt,
/// after which Ctrl-O arrives as `ESC[111;5u` rather than the raw 0x0F byte.
/// Recognized encodings:
///   - raw byte 0x0F (legacy)
///   - kitty CSI-u: `ESC [ 111 <:alternates>? ; <mods> <:event>? u`
///   - xterm modifyOtherKeys: `ESC [ 27 ; <mods> ; 111 ~`
fn scan_for_ctrl_o(buf: &[u8]) -> Scan {
    let mut i = 0;
    while i < buf.len() {
        match buf[i] {
            CTRL_O => return Scan::Toggle { start: i },
            0x1B if i + 1 < buf.len() && buf[i + 1] == b'[' => {
                // A CSI sequence: params are 0x30..=0x3F bytes, then an
                // optional intermediate, then a final byte in 0x40..=0x7E.
                let params_start = i + 2;
                let mut j = params_start;
                while j < buf.len() && (0x20..=0x3F).contains(&buf[j]) {
                    j += 1;
                }
                if j >= buf.len() {
                    // Unfinished sequence at the end of the chunk: hold it back.
                    let len = buf.len() - i;
                    return Scan::None { hold: if len <= MAX_HOLD { len } else { 0 } };
                }
                let terminator = buf[j];
                let params = &buf[params_start..j];
                let is_toggle = match terminator {
                    b'u' => csi_u_is_ctrl_o(params),
                    b'~' => modify_other_keys_is_ctrl_o(params),
                    _ => false,
                };
                if is_toggle {
                    return Scan::Toggle { start: i };
                }
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    Scan::None { hold: 0 }
}

/// Modifier bitmask from a kitty/xterm `<mods>` parameter (encoded value minus
/// one), with the lock bits masked off so Caps/Num Lock don't break the match.
fn decoded_mods(field: &str) -> Option<u16> {
    const CAPS_LOCK: u16 = 64;
    const NUM_LOCK: u16 = 128;
    let raw: u16 = field.parse().ok()?;
    Some(raw.saturating_sub(1) & !(CAPS_LOCK | NUM_LOCK))
}

/// `ESC [ <key>[:alt] ; <mods>[:event] u` — is it a Ctrl-O press/repeat?
fn csi_u_is_ctrl_o(params: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(params) else {
        return false;
    };
    let mut fields = s.split(';');
    // `split` always yields a first item; the key field may carry
    // shifted/base-layout alternate codes after colons.
    let key_field = fields.next().expect("split yields a first item");
    let key = key_field.split(':').next().expect("split yields a first item");
    if key.parse() != Ok(CTRL_O_KEYCODE) {
        return false;
    }
    // Modifier field defaults to "1" (no modifiers) and may carry an event
    // type after a colon: 1 = press, 2 = repeat, 3 = release.
    let mut sub = fields.next().unwrap_or("1").split(':');
    let mods_field = sub.next().expect("split yields a first item");
    let Some(mods) = decoded_mods(mods_field) else {
        return false;
    };
    let event = sub.next().unwrap_or("1");
    mods == CTRL_MOD && (event == "1" || event == "2")
}

/// `ESC [ 27 ; <mods> ; <key> ~` (xterm modifyOtherKeys) — is it Ctrl-O?
fn modify_other_keys_is_ctrl_o(params: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(params) else {
        return false;
    };
    let mut fields = s.split(';');
    if fields.next() != Some("27") {
        return false;
    }
    let Some(mods) = fields.next().and_then(decoded_mods) else {
        return false;
    };
    let key = fields.next().and_then(|k| k.parse().ok());
    mods == CTRL_MOD && key == Some(CTRL_O_KEYCODE)
}

pub struct Subshell {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// When true, the reader thread mirrors PTY output to the real stdout.
    active: Arc<AtomicBool>,
    pid: Option<u32>,
    /// The emulator this shell feeds; handed to the backdrop via `set_current`.
    feed: crate::console::ConsoleFeed,
}

impl Subshell {
    /// Spawn the shell in `cwd` attached to a fresh PTY of the given size.
    ///
    /// The reader thread mirrors output to the real stdout only while toggled in
    /// (`Ctrl-O`), but *always* feeds the shared console emulator (`parser`) so
    /// the backdrop stays live, raises `used` on the first byte, and — while not
    /// toggled in — nudges the render loop (`tx`) to repaint.
    pub fn spawn(
        cwd: &Path,
        rows: u16,
        cols: u16,
        feed: crate::console::ConsoleFeed,
        tx: AppSender,
    ) -> Result<Subshell> {
        let parser = feed.parser.clone();
        let used = feed.used.clone();
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::other(format!("openpty failed: {e}")))?;

        let mut cmd = CommandBuilder::new(default_shell());
        cmd.cwd(cwd);
        // Mark the shell's environment so a nested Rat Commander started from it
        // detects the nesting and disables its own (unsupported) subshell.
        cmd.env(SUBSHELL_ENV, "1");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::other(format!("failed to start shell: {e}")))?;
        let pid = child.process_id();
        // Close the slave in the parent so the PTY reports EOF when the shell exits.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::other(format!("pty reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::other(format!("pty writer: {e}")))?;

        let active = Arc::new(AtomicBool::new(false));
        // Reader thread: drain the PTY for the shell's lifetime. Every chunk feeds
        // the shared console emulator (the backdrop); it is additionally mirrored
        // to the real stdout while toggled in (Ctrl-O), or — while not toggled in
        // — signals the render loop to repaint the backdrop.
        {
            let active = active.clone();
            let mut reader = reader;
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                let mut out = std::io::stdout();
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut p) = parser.lock() {
                                p.process(&buf[..n]);
                            }
                            used.store(true, Ordering::Relaxed);
                            if active.load(Ordering::Relaxed) {
                                let _ = out.write_all(&buf[..n]);
                                let _ = out.flush();
                            } else {
                                // Coalesced nudge; a full channel just means a
                                // repaint is already pending.
                                let _ = tx.try_send(AppEvent::ConsoleOutput);
                            }
                        }
                    }
                }
            });
        }

        Ok(Subshell {
            master: pair.master,
            writer,
            child,
            active,
            pid,
            feed,
        })
    }

    /// The emulator this shell feeds, for `Console::set_current`.
    pub fn console(&self) -> crate::console::ConsoleFeed {
        self.feed.clone()
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// Write a command line to the shell (as if typed), followed by Enter, so it
    /// runs in this persistent session. Used by the command line so its commands
    /// and the `Ctrl-O` shell are one and the same session.
    pub fn send_line(&mut self, line: &str) {
        let _ = self.writer.write_all(line.as_bytes());
        let _ = self.writer.write_all(b"\n");
        let _ = self.writer.flush();
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Forward the real terminal to the shell until Ctrl-O is pressed (or the
    /// shell exits). The terminal must already be in raw mode and on the
    /// primary screen.
    pub fn run_until_toggle(&mut self) {
        self.active.store(true, Ordering::Relaxed);
        let stdin = std::io::stdin();
        let mut handle = stdin.lock();
        let mut buf = [0u8; 1024];
        // Bytes held back from the previous read: the tail of a possibly
        // unfinished escape sequence that could turn out to be Ctrl-O.
        let mut pending: Vec<u8> = Vec::new();
        loop {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            let n = match handle.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            pending.extend_from_slice(&buf[..n]);
            match scan_for_ctrl_o(&pending) {
                Scan::Toggle { start } => {
                    // Forward everything before the toggle sequence, swallow
                    // the sequence itself, then return to the panels.
                    let _ = self.writer.write_all(&pending[..start]);
                    let _ = self.writer.flush();
                    break;
                }
                Scan::None { hold } => {
                    let forward = pending.len() - hold;
                    if self.writer.write_all(&pending[..forward]).is_err() {
                        break;
                    }
                    let _ = self.writer.flush();
                    pending.drain(..forward);
                }
            }
        }
        self.active.store(false, Ordering::Relaxed);
    }

    /// The shell's current working directory, if it can be determined (Linux).
    pub fn child_cwd(&self) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "linux")]
        {
            self.pid
                .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/cwd")).ok())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self.pid;
            None
        }
    }
}

impl Drop for Subshell {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// ---------------------------------------------------------------------------
// Remote shell (SSH): Ctrl-O / command line on an SFTP/SCP panel run on the
// remote host, over the *same* SSH connection the file transfers use.
// ---------------------------------------------------------------------------

/// An interactive shell on a remote SSH host, presented exactly like the local
/// [`Subshell`]: it feeds a console emulator (the backdrop), mirrors to stdout
/// while toggled in (`Ctrl-O`), and takes typed input / command lines. The russh
/// channel is pumped by a background async task; the blocking-stdin
/// [`run_until_toggle`](RemoteShell::run_until_toggle) forwards keystrokes to it
/// through an input queue.
pub struct RemoteShell {
    /// Bytes to write to the remote shell (drained by the pump task).
    input_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// Latest requested PTY size, applied by the pump on the next loop turn.
    resize: Arc<Mutex<Option<(u16, u16)>>>,
    resize_notify: Arc<tokio::sync::Notify>,
    /// When true, the pump mirrors remote output to the real stdout.
    active: Arc<AtomicBool>,
    /// Set by the pump when the channel closes (shell exited / disconnected).
    closed: Arc<AtomicBool>,
    feed: crate::console::ConsoleFeed,
    /// The remote directory last `cd`'d to, so the shell follows the panel without
    /// re-`cd`ing on every command.
    last_cd: Option<String>,
    task: tokio::task::JoinHandle<()>,
}

impl RemoteShell {
    /// Wrap an opened remote shell channel, spawning the pump that feeds `feed`.
    pub fn spawn(
        ch: crate::vfs::remote::RemoteShellChannel,
        feed: crate::console::ConsoleFeed,
        tx: AppSender,
    ) -> RemoteShell {
        let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let resize = Arc::new(Mutex::new(None));
        let resize_notify = Arc::new(tokio::sync::Notify::new());
        let active = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(remote_pump(
            ch.channel,
            feed.clone(),
            tx,
            active.clone(),
            closed.clone(),
            input_rx,
            resize.clone(),
            resize_notify.clone(),
        ));
        RemoteShell { input_tx, resize, resize_notify, active, closed, feed, last_cd: None, task }
    }

    /// The emulator this shell feeds, for `Console::set_current`.
    pub fn console(&self) -> crate::console::ConsoleFeed {
        self.feed.clone()
    }

    /// `cd` the remote shell into `dir` (a POSIX path), unless it is already
    /// there — so the shell follows the active panel like the local one does.
    pub fn cd_to(&mut self, dir: &str) {
        if self.last_cd.as_deref() != Some(dir) {
            self.send_line(&format!("cd -- {}", crate::vfs::remote::shell_quote(dir)));
            self.last_cd = Some(dir.to_string());
        }
    }

    /// Whether the channel is still open.
    pub fn is_alive(&self) -> bool {
        !self.closed.load(Ordering::Relaxed)
    }

    /// Write a command line to the remote shell (as if typed), followed by Enter.
    pub fn send_line(&mut self, line: &str) {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        let _ = self.input_tx.send(bytes);
    }

    /// Request a PTY resize; applied by the pump on its next turn.
    pub fn resize(&self, rows: u16, cols: u16) {
        if let Ok(mut g) = self.resize.lock() {
            *g = Some((rows, cols));
        }
        self.resize_notify.notify_one();
    }

    /// Forward the real terminal to the remote shell until Ctrl-O is pressed (or
    /// the shell closes). Mirrors [`Subshell::run_until_toggle`]; the "writer" is
    /// the input queue the pump drains onto the channel.
    pub fn run_until_toggle(&mut self) {
        self.active.store(true, Ordering::Relaxed);
        let stdin = std::io::stdin();
        let mut handle = stdin.lock();
        let mut buf = [0u8; 1024];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            if self.closed.load(Ordering::Relaxed) {
                break;
            }
            let n = match handle.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            pending.extend_from_slice(&buf[..n]);
            match scan_for_ctrl_o(&pending) {
                Scan::Toggle { start } => {
                    if start > 0 {
                        let _ = self.input_tx.send(pending[..start].to_vec());
                    }
                    break;
                }
                Scan::None { hold } => {
                    let forward = pending.len() - hold;
                    if forward > 0 {
                        let _ = self.input_tx.send(pending[..forward].to_vec());
                    }
                    pending.drain(..forward);
                }
            }
        }
        self.active.store(false, Ordering::Relaxed);
    }
}

impl Drop for RemoteShell {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Pump a remote shell channel: feed its output into the console emulator (and
/// stdout while toggled in), drain queued input onto the channel, and apply
/// resizes. Runs until the channel closes.
#[allow(clippy::too_many_arguments)]
async fn remote_pump(
    mut channel: russh::Channel<russh::client::Msg>,
    feed: crate::console::ConsoleFeed,
    tx: AppSender,
    active: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    mut input_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    resize: Arc<Mutex<Option<(u16, u16)>>>,
    resize_notify: Arc<tokio::sync::Notify>,
) {
    use tokio::io::AsyncWriteExt;
    let mut writer = channel.make_writer();
    let mut out = std::io::stdout();
    loop {
        // Apply a pending resize while the channel isn't otherwise borrowed.
        let pending = resize.lock().ok().and_then(|mut g| g.take());
        if let Some((rows, cols)) = pending {
            let _ = channel.window_change(cols as u32, rows as u32, 0, 0).await;
        }
        tokio::select! {
            msg = channel.wait() => match msg {
                Some(russh::ChannelMsg::Data { data }) => {
                    feed_output(&data, &feed, &active, &mut out, &tx);
                }
                Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                    feed_output(&data, &feed, &active, &mut out, &tx);
                }
                Some(russh::ChannelMsg::Eof) | None => break,
                Some(_) => {}
            },
            Some(bytes) = input_rx.recv() => {
                let _ = writer.write_all(&bytes).await;
                let _ = writer.flush().await;
            }
            _ = resize_notify.notified() => { /* loops to apply the resize above */ }
        }
    }
    closed.store(true, Ordering::Relaxed);
    // Wake the render loop so the closed shell is noticed.
    let _ = tx.try_send(AppEvent::ConsoleOutput);
}

/// Feed a chunk of remote output into the emulator, mirroring it to stdout while
/// toggled in or nudging a repaint otherwise (mirrors the local reader thread).
fn feed_output(
    data: &[u8],
    feed: &crate::console::ConsoleFeed,
    active: &Arc<AtomicBool>,
    out: &mut std::io::Stdout,
    tx: &AppSender,
) {
    if let Ok(mut p) = feed.parser.lock() {
        p.process(data);
    }
    feed.used.store(true, Ordering::Relaxed);
    if active.load(Ordering::Relaxed) {
        let _ = out.write_all(data);
        let _ = out.flush();
    } else {
        let _ = tx.try_send(AppEvent::ConsoleOutput);
    }
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggle_at(buf: &[u8]) -> Option<usize> {
        match scan_for_ctrl_o(buf) {
            Scan::Toggle { start } => Some(start),
            Scan::None { .. } => None,
        }
    }

    #[test]
    fn raw_ctrl_o() {
        assert_eq!(toggle_at(b"\x0f"), Some(0));
        assert_eq!(toggle_at(b"ls\x0fmore"), Some(2));
    }

    #[test]
    fn kitty_csi_u_ctrl_o() {
        // Plain press, press with explicit event type, and repeat.
        assert_eq!(toggle_at(b"\x1b[111;5u"), Some(0));
        assert_eq!(toggle_at(b"\x1b[111;5:1u"), Some(0));
        assert_eq!(toggle_at(b"\x1b[111;5:2u"), Some(0));
        // Caps Lock / Num Lock bits don't break the match.
        assert_eq!(toggle_at(b"\x1b[111;69u"), Some(0));
        assert_eq!(toggle_at(b"\x1b[111;197u"), Some(0));
    }

    #[test]
    fn kitty_csi_u_rejects_non_toggles() {
        // Release must not toggle (fish enables report-event-types).
        assert!(toggle_at(b"\x1b[111;5:3u").is_none());
        // Wrong key, missing ctrl, extra modifiers.
        assert!(toggle_at(b"\x1b[112;5u").is_none());
        assert!(toggle_at(b"\x1b[111u").is_none());
        assert!(toggle_at(b"\x1b[111;1u").is_none());
        assert!(toggle_at(b"\x1b[111;7u").is_none()); // ctrl+shift+alt
        // Kitty protocol *push/query* sequences, not keys at all.
        assert!(toggle_at(b"\x1b[=5u").is_none());
        assert!(toggle_at(b"\x1b[?0u").is_none());
        assert!(toggle_at(b"\x1b[>1u").is_none());
    }

    #[test]
    fn modify_other_keys_ctrl_o() {
        assert_eq!(toggle_at(b"\x1b[27;5;111~"), Some(0));
        assert!(toggle_at(b"\x1b[27;5;112~").is_none());
        assert!(toggle_at(b"\x1b[27;2;111~").is_none());
        // Ordinary special keys (e.g. Delete) pass through.
        assert!(toggle_at(b"\x1b[3~").is_none());
    }

    #[test]
    fn alternate_key_reports() {
        // Key field may carry shifted/base-layout alternates after a colon.
        assert_eq!(toggle_at(b"\x1b[111:79;5u"), Some(0));
    }

    #[test]
    fn embedded_in_stream() {
        let buf = b"abc\x1b[A\x1b[111;5uxyz";
        assert_eq!(toggle_at(buf), Some(6));
    }

    #[test]
    fn unfinished_sequence_is_held() {
        match scan_for_ctrl_o(b"ls\x1b[111;5") {
            Scan::None { hold } => assert_eq!(hold, 7),
            Scan::Toggle { .. } => panic!("must not toggle on a prefix"),
        }
        // A bare trailing ESC is forwarded immediately (vi-mode Esc must not lag).
        match scan_for_ctrl_o(b"ls\x1b") {
            Scan::None { hold } => assert_eq!(hold, 0),
            Scan::Toggle { .. } => panic!(),
        }
        // Over-long garbage "sequences" are not held back forever.
        match scan_for_ctrl_o(b"\x1b[0123456789012345678901234567") {
            Scan::None { hold } => assert_eq!(hold, 0),
            Scan::Toggle { .. } => panic!(),
        }
    }

    // --- Remote shell round-trip over a real (in-process) SSH connection ---

    /// A throwaway ed25519 host key for the test SSH server.
    const TEST_HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\n\
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n\
QyNTUxOQAAACBhtSAp308g5/FxsHPUCHBLm2jW2k9S/rE+TqPjPHBVlAAAAJB9CQOFfQkD\n\
hQAAAAtzc2gtZWQyNTUxOQAAACBhtSAp308g5/FxsHPUCHBLm2jW2k9S/rE+TqPjPHBVlA\n\
AAAEBuA4oTbyADSU6M0oRqvoIzRfsXXZ2ESA5/JFHtNMzhKGG1ICnfTyDn8XGwc9QIcEub\n\
aNbaT1L+sT5Oo+M8cFWUAAAAB3JjLXRlc3QBAgMEBQY=\n\
-----END OPENSSH PRIVATE KEY-----\n";

    /// A minimal SSH server: accepts any password, accepts a session channel, and
    /// echoes back whatever the client sends (standing in for a remote shell).
    #[derive(Clone)]
    struct EchoServer;

    impl russh::server::Server for EchoServer {
        type Handler = EchoServer;
        fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> EchoServer {
            EchoServer
        }
    }

    impl russh::server::Handler for EchoServer {
        type Error = russh::Error;

        async fn auth_password(
            &mut self,
            _user: &str,
            _password: &str,
        ) -> std::result::Result<russh::server::Auth, Self::Error> {
            Ok(russh::server::Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            _channel: russh::Channel<russh::server::Msg>,
            reply: russh::server::ChannelOpenHandle,
            _session: &mut russh::server::Session,
        ) -> std::result::Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        async fn data(
            &mut self,
            channel: russh::ChannelId,
            data: &[u8],
            session: &mut russh::server::Session,
        ) -> std::result::Result<(), Self::Error> {
            let _ = session.data(channel, data.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    async fn remote_shell_round_trips_over_ssh() {
        use russh::server::Server as _; // brings `run_on_socket` into scope

        let key = russh::keys::PrivateKey::from_openssh(TEST_HOST_KEY).expect("host key");
        let config =
            std::sync::Arc::new(russh::server::Config { keys: vec![key], ..Default::default() });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut server = EchoServer;
            let _ = server.run_on_socket(config, &listener).await;
        });

        // Connect the client and open an interactive shell channel — the exact
        // path Ctrl-O / the command line takes on an SFTP/SCP panel.
        let creds = crate::vfs::remote::RemoteCreds {
            protocol: crate::vfs::remote::Protocol::Sftp,
            host: "127.0.0.1".to_string(),
            port,
            user: "u".to_string(),
            password: "p".to_string(),
            path: String::new(),
            passive: true,
        };
        let handle = crate::vfs::remote::ssh_connect(&creds).await.expect("ssh connect");
        let ch = crate::vfs::remote::open_shell_channel(&handle, 24, 80).await.expect("shell");

        let (tx, _rx) = crate::util::async_bridge::channel();
        let feed = crate::console::ConsoleFeed::new(24, 80);
        let mut shell = RemoteShell::spawn(ch, feed.clone(), tx);
        assert!(shell.is_alive());

        // Send a command; the echo server sends it straight back, which the pump
        // feeds into the console emulator (the backdrop).
        shell.send_line("echo hello world");
        let mut found = false;
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            let text = {
                let p = feed.parser.lock().unwrap();
                let s = p.screen();
                let (rows, cols) = s.size();
                let mut out = String::new();
                for r in 0..rows {
                    for c in 0..cols {
                        if let Some(cell) = s.cell(r, c) {
                            out.push_str(cell.contents());
                        }
                    }
                }
                out
            };
            if text.contains("echo hello world") {
                found = true;
                break;
            }
        }
        assert!(found, "the remote shell echoed the command back to the console backdrop");
    }
}
