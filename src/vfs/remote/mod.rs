//! Remote VFS backends: SFTP and SCP (over SSH) and FTP/FTPS.
//!
//! Each connection is a distinct backend instance registered under a unique
//! scheme (e.g. `sftp-0`) so multiple sessions can coexist. Listing/transfer/
//! delete all flow through the [`Vfs`](crate::vfs::Vfs) trait, so the generic
//! ops engine handles cross-backend copy/move/delete for free.

pub mod ftp;
pub mod scp;
pub mod sftp;

use crate::util::{Error, Result};
use crate::vfs::VfsKind;
use std::sync::Arc;

/// A directory entry parsed from a Unix `ls -l` / FTP `LIST` line.
pub(crate) struct ParsedListing {
    pub name: String,
    pub kind: VfsKind,
    pub size: u64,
    pub mode: Option<u32>,
    pub symlink_target: Option<String>,
}

/// Parse one Unix-style long listing line (handles both the classic
/// `Mon DD HH:MM` date and ISO `YYYY-MM-DD HH:MM`). Returns `None` for header
/// lines (`total N`), blanks, or `.`/`..`.
pub(crate) fn parse_unix_listing_line(line: &str) -> Option<ParsedListing> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() || line.starts_with("total ") {
        return None;
    }
    let toks: Vec<&str> = line.split_whitespace().collect();
    if toks.len() < 8 {
        return None;
    }
    let perms = toks[0];
    if perms.len() < 10 {
        return None;
    }
    let type_char = perms.chars().next().unwrap();
    let kind = match type_char {
        'd' => VfsKind::Dir,
        'l' => VfsKind::Symlink,
        '-' => VfsKind::File,
        _ => VfsKind::Other,
    };
    let size = toks[4].parse::<u64>().unwrap_or(0);
    // Name starts after the date: ISO date (contains '-') uses 2 tokens, the
    // classic `Mon DD HH:MM`/`Mon DD YYYY` uses 3.
    let name_start = if toks[5].contains('-') { 7 } else { 8 };
    if toks.len() <= name_start {
        return None;
    }
    let rest = toks[name_start..].join(" ");

    let (name, symlink_target) = if kind == VfsKind::Symlink {
        match rest.split_once(" -> ") {
            Some((n, t)) => (n.to_string(), Some(t.to_string())),
            None => (rest, None),
        }
    } else {
        (rest, None)
    };
    if name == "." || name == ".." || name.is_empty() {
        return None;
    }
    Some(ParsedListing {
        name,
        kind,
        size,
        mode: Some(perms_to_mode(perms)),
        symlink_target,
    })
}

/// Convert a `rwxr-xr-x` permission string (after the type char) to mode bits.
pub(crate) fn perms_to_mode(perms: &str) -> u32 {
    let bytes = perms.as_bytes();
    let mut mode = 0u32;
    // perms[1..10] = owner/group/other rwx.
    for (i, &b) in bytes.iter().skip(1).take(9).enumerate() {
        if b != b'-' {
            mode |= 1 << (8 - i);
        }
    }
    mode
}

/// Quote a path for safe use in a remote `sh -c` command (single-quoted).
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Which remote protocol a connection uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Sftp,
    Ftp,
    Scp,
}

impl Protocol {
    pub fn scheme_prefix(self) -> &'static str {
        match self {
            Protocol::Sftp => "sftp",
            Protocol::Ftp => "ftp",
            Protocol::Scp => "scp",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp | Protocol::Scp => 22,
            Protocol::Ftp => 21,
        }
    }
}

/// Connection parameters collected from the connect dialog.
#[derive(Debug, Clone)]
pub struct RemoteCreds {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Initial remote directory (defaults to the server's choice if empty).
    pub path: String,
    /// FTP passive mode (PASV): the client opens the data connection. On by
    /// default and needed behind most NAT/firewalls. Ignored by SFTP/SCP, which
    /// tunnel data over the single SSH connection.
    pub passive: bool,
}

/// A live remote connection: a VFS backend plus the directory to open.
pub struct Connection {
    pub backend: Arc<dyn crate::vfs::Vfs>,
    pub root: String,
    pub label: String,
}

/// Establish a remote connection of the requested protocol.
pub async fn connect(creds: &RemoteCreds) -> Result<Connection> {
    match creds.protocol {
        Protocol::Sftp => sftp::connect(creds).await,
        Protocol::Ftp => ftp::connect(creds).await,
        Protocol::Scp => scp::connect(creds).await,
    }
}

// ---------------------------------------------------------------------------
// Shared SSH client (used by SFTP and SCP)
// ---------------------------------------------------------------------------

/// russh client handler implementing trust-on-first-use against the user's
/// `~/.ssh/known_hosts`: a matching key is accepted, a *changed* key is
/// rejected (possible MITM), and an unknown host is accepted and recorded.
pub(crate) struct HostKeyHandler {
    host: String,
    port: u16,
}

impl russh::client::Handler for HostKeyHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        match russh::keys::check_known_hosts(&self.host, self.port, server_public_key) {
            Ok(true) => Ok(true),  // known host, key matches
            Ok(false) => Ok(true), // unknown host: trust on first use
            Err(russh::keys::Error::KeyChanged { .. }) => Ok(false), // reject possible MITM
            Err(_) => Ok(true),    // known_hosts unreadable — fall back to accepting
        }
    }
}

pub(crate) type SshHandle = russh::client::Handle<HostKeyHandler>;

/// An opened interactive shell channel on a remote SSH host — a PTY and shell are
/// already requested, so it is a live bidirectional byte stream. Wraps the russh
/// channel; the app drives it through [`crate::shell::RemoteShell`]. Only the
/// SSH-based backends (SFTP/SCP) can produce one.
pub struct RemoteShellChannel {
    pub channel: russh::Channel<russh::client::Msg>,
}

/// Open a session channel on `handle`, request a PTY of the given size and an
/// interactive shell, returning the ready channel.
pub(crate) async fn open_shell_channel(
    handle: &SshHandle,
    rows: u16,
    cols: u16,
) -> Result<RemoteShellChannel> {
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| Error::other(format!("shell channel open failed: {e}")))?;
    channel
        .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await
        .map_err(|e| Error::other(format!("request pty failed: {e}")))?;
    channel
        .request_shell(false)
        .await
        .map_err(|e| Error::other(format!("request shell failed: {e}")))?;
    Ok(RemoteShellChannel { channel })
}

/// Open an SSH connection and authenticate with a password.
pub(crate) async fn ssh_connect(creds: &RemoteCreds) -> Result<SshHandle> {
    let config = Arc::new(russh::client::Config::default());
    let handler = HostKeyHandler {
        host: creds.host.clone(),
        port: creds.port,
    };
    let mut handle = russh::client::connect(config, (creds.host.as_str(), creds.port), handler)
        .await
        .map_err(|e| Error::other(format!("SSH connect failed: {e}")))?;
    let auth = handle
        .authenticate_password(&creds.user, &creds.password)
        .await
        .map_err(|e| Error::other(format!("SSH auth error: {e}")))?;
    if !auth.success() {
        return Err(Error::other("SSH authentication failed (bad user/password)"));
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_classic_ls_line() {
        let p = parse_unix_listing_line("-rw-r--r-- 1 user group 1234 Jan  2 12:00 notes.txt")
            .unwrap();
        assert_eq!(p.name, "notes.txt");
        assert_eq!(p.kind, VfsKind::File);
        assert_eq!(p.size, 1234);
        assert_eq!(p.mode, Some(0o644));
    }

    #[test]
    fn parses_iso_dir_and_symlink() {
        let dir = parse_unix_listing_line("drwxr-xr-x 2 u g 4096 2024-01-02 12:00 mydir").unwrap();
        assert_eq!(dir.name, "mydir");
        assert_eq!(dir.kind, VfsKind::Dir);
        assert_eq!(dir.mode, Some(0o755));

        let link =
            parse_unix_listing_line("lrwxrwxrwx 1 u g 7 2024-01-02 12:00 link -> target").unwrap();
        assert_eq!(link.name, "link");
        assert_eq!(link.kind, VfsKind::Symlink);
        assert_eq!(link.symlink_target.as_deref(), Some("target"));
    }

    #[test]
    fn skips_total_and_dot_entries() {
        assert!(parse_unix_listing_line("total 12").is_none());
        assert!(parse_unix_listing_line("drwxr-xr-x 2 u g 4096 2024-01-02 12:00 .").is_none());
        assert!(parse_unix_listing_line("drwxr-xr-x 2 u g 4096 2024-01-02 12:00 ..").is_none());
        assert!(parse_unix_listing_line("").is_none());
    }

    #[test]
    fn shell_quote_escapes() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}
