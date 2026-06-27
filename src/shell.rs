//! Persistent Ctrl-O subshell, Midnight-Commander style.
//!
//! A single shell process is kept alive in a pseudo-terminal for the life of
//! the app. Ctrl-O *toggles* into it (forwarding the real terminal to the PTY)
//! and Ctrl-O again toggles back to the panels — the shell keeps running, so
//! its working directory, environment, history and jobs are preserved between
//! visits.

use crate::util::{Error, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Byte sent by Ctrl-O.
const CTRL_O: u8 = 0x0F;

pub struct Subshell {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// When true, the reader thread mirrors PTY output to the real stdout.
    active: Arc<AtomicBool>,
    pid: Option<u32>,
}

impl Subshell {
    /// Spawn the shell in `cwd` attached to a fresh PTY of the given size.
    pub fn spawn(cwd: &Path, rows: u16, cols: u16) -> Result<Subshell> {
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
        // Reader thread: drain the PTY for the shell's lifetime, mirroring to
        // stdout only while we're toggled into the subshell.
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
                            if active.load(Ordering::Relaxed) {
                                let _ = out.write_all(&buf[..n]);
                                let _ = out.flush();
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
        })
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
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
        loop {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            let n = match handle.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            if let Some(pos) = buf[..n].iter().position(|&b| b == CTRL_O) {
                // Forward everything before the toggle, then return.
                let _ = self.writer.write_all(&buf[..pos]);
                let _ = self.writer.flush();
                break;
            }
            if self.writer.write_all(&buf[..n]).is_err() {
                break;
            }
            let _ = self.writer.flush();
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

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}
